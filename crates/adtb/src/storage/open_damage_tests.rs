//! Opening, verifying and dropping a transaction on a file with a page no relation holds damaged:
//! the file opens for `verify` only and every other call fails `Quarantined`.

use crate::testing::{Object, Plant, Probe};
use adtb_harness::process::finished;
use adtb_testkit::scratch::Scratch;
use std::collections::{BTreeMap, BTreeSet};

use crate::runtime::{IoError, OpKind};
use crate::storage::order::ordered_for_test;
use crate::storage::{Damage, HEADER_LEN, Options, PageError, Pager, Pid, Quarantine};
use crate::test_world::api;
use crate::test_world::load as open;
use crate::{CallOptions, Database, Error, FatalReason, Log, OpenError, OpenOptions, TxnOptions};
use adtb_harness::damage::{DataFile, Files};
use adtb_sim::{Action, Config, Match, Plan, Sim, SimRuntime};
use adtb_sys::runtime::Faults;

const FILE: &str = "open_damage.adt";
const PAGE: usize = 8192;
const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
";
const CLEAN: &str = r"module app
add module app
add type Other = { id: Key others }
add relation others : Other key id generate sequence
";
const INSERT: &str = r#"insert users { id: users.key(9), name: "x" }"#;
const SELECT: &str = "from u in users select u.id";

const DATA: DataFile = DataFile::new(FILE, PAGE);

fn app() -> Log {
    Log::new([APP])
}

fn quarantined<T: std::fmt::Debug>(got: Result<T, Error>) -> String {
    match got {
        Err(Error::Fatal {
            reason: FatalReason::Quarantined { what },
        }) => what,
        other => panic!("not Quarantined: {other:?}"),
    }
}

fn status_page<R: crate::runtime::Runtime>(db: &Database<R>) -> Pid {
    *db.testing()
        .commit_log()
        .status()
        .values()
        .next_back()
        .unwrap()
}

fn roles_page<R: crate::runtime::Runtime>(db: &Database<R>) -> Pid {
    let t = db.testing();
    t.pages(t.heap("catalog::roles", "Role").unwrap()).unwrap()[0]
}

fn grants_page<R: crate::runtime::Runtime>(db: &Database<R>) -> Pid {
    let t = db.testing();
    t.owned(t.heap("catalog::grants", "RoleGrant").unwrap())
        .unwrap()[0]
}

fn catalog_root<R: crate::runtime::Runtime>(db: &Database<R>) -> Pid {
    db.testing().root(Object::Catalog).unwrap()
}

fn clog_root<R: crate::runtime::Runtime>(db: &Database<R>) -> Pid {
    db.testing().commit_log().root()
}

fn clog_directory<R: crate::runtime::Runtime>(db: &Database<R>) -> Pid {
    db.testing().commit_log().directory()[0]
}

/// Returns the files of a seeded, closed database and the page `pick` picks.
fn seeded(seed: u64, pick: fn(&Database<SimRuntime>) -> Pid) -> (Files, Pid) {
    let mut sim = Sim::new(seed, Config::default());
    let mut page = None;
    finished(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        api::results(&db, r#"insert users { id: users.key(1), name: "a" }"#);
        page = Some(pick(&db));
        db.close().unwrap();
    });
    (sim.files(), page.unwrap())
}

/// Assert only `verify` runs on `db`, and its findings include one starting `finding`, or with
/// none are only the note that a run quarantined whole keeps no findings.
fn assert_only_verify_runs(db: &Database<SimRuntime>, finding: Option<&str>) {
    let marks = db.testing().probe().marks();
    assert_eq!(BTreeSet::from([Quarantine::Database]), marks);
    let none = CallOptions::default();
    for text in [SELECT, INSERT, "from nothing in nowhere select 1", "vacuum"] {
        let what = quarantined(db.query_text(text, &[], &none));
        assert_eq!("the database", what, "{text}");
    }
    assert_eq!(
        "the database",
        quarantined(db.begin(&TxnOptions::default()).map(drop))
    );
    assert_eq!("the database", quarantined(db.registered("nope").map(drop)));
    let found = api::printed(db.verify(None).unwrap());
    assert_findings(&found, finding);
    let found = api::printed(db.testing().verify_holding().unwrap());
    assert_findings(&found, finding);
}

fn assert_opens_for_verify(seed: u64, pick: fn(&Database<SimRuntime>) -> Pid) {
    let (mut files, page) = seeded(seed, pick);
    DATA.flip(&mut files, page, 300 - HEADER_LEN);
    let mut sim = Sim::with_files(seed, Config::default(), files);
    for round in 0..2 {
        finished(&mut sim, |rt| {
            let db = open::open_in(&rt, FILE, &app(), &open::options(2))
                .unwrap_or_else(|e| panic!("round {round}: {e:?}"));
            assert_only_verify_runs(&db, Some("Checksum"));
            db.close().unwrap();
        });
    }
}

#[test]
fn a_damaged_catalog_row_page_opens_for_verify() {
    assert_opens_for_verify(1, roles_page);
}

#[test]
fn a_damaged_page_of_an_empty_grants_heap_opens_for_verify() {
    assert_opens_for_verify(7, grants_page);
}

#[test]
fn a_damaged_catalog_root_opens_for_verify() {
    assert_opens_for_verify(2, catalog_root);
}

#[test]
fn a_damaged_status_page_opens_for_verify() {
    assert_opens_for_verify(3, status_page);
}

#[test]
fn a_damaged_commit_log_root_opens_for_verify() {
    assert_opens_for_verify(4, clog_root);
}

#[test]
fn a_damaged_commit_log_directory_opens_for_verify() {
    assert_opens_for_verify(5, clog_directory);
}

#[test]
fn verify_of_an_unread_catalog_says_what_it_did_not_check() {
    let (mut files, page) = seeded(6, catalog_root);
    DATA.flip(&mut files, page, 300 - HEADER_LEN);
    let mut sim = Sim::with_files(6, Config::default(), files);
    finished(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        let found = api::printed(db.verify(None).unwrap());
        assert!(
            found
                .iter()
                .any(|r| r.starts_with("Structure") && r.contains("only the pages and the log")),
            "{found:?}"
        );
        assert_eq!(
            "the database",
            quarantined(db.verify(Some("catalog::roles")).map(drop))
        );
        assert_eq!(
            "the database",
            quarantined(db.verify(Some("users")).map(drop))
        );
    });
}

/// Returns each page of `file` that differs from `before`, past the file header.
fn changed(before: &[u8], file: &[u8]) -> BTreeSet<u64> {
    assert_eq!(before.len(), file.len());
    before
        .chunks(PAGE)
        .zip(file.chunks(PAGE))
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(n, _)| n as u64)
        .filter(|&n| n >= Pid::FIRST_DATA.0)
        .collect()
}

fn with_pager<T>(sim: &mut Sim, body: impl FnOnce(&Pager<SimRuntime>) -> T) -> T {
    let mut out = None;
    finished(sim, |rt| {
        let options = Options {
            page_size: PAGE as u32,
            threads: 2,
            ..Options::new(FILE)
        };
        let (p, _) = Pager::open(rt, &options).unwrap();
        out = Some(body(&p));
    });
    out.unwrap()
}

/// An intact commit log resolves the ids left in progress before the open reads the damage.
#[test]
fn an_open_of_a_damaged_file_writes_only_the_mark() {
    let picks: [fn(&Database<SimRuntime>) -> Pid; 5] = [
        roles_page,
        catalog_root,
        status_page,
        clog_root,
        grants_page,
    ];
    for (n, pick) in picks.into_iter().enumerate() {
        let seed = 10 + n as u64;
        let (mut files, page) = seeded(seed, pick);
        DATA.flip(&mut files, page, 300 - HEADER_LEN);
        let before = files[FILE].clone();
        let mut sim = Sim::with_files(seed, Config::default(), files);
        finished(&mut sim, |rt| {
            open::open_in(&rt, FILE, &app(), &open::options(2))
                .unwrap()
                .close()
                .unwrap();
        });
        let marked = sim.files()[FILE].clone();
        let marks = with_pager(&mut sim, Pager::quarantine_page);
        let mut expected = BTreeSet::from([marks.0]);
        if n == 0 || n == 4 {
            expected.insert(seeded(seed, status_page).1.0);
        }
        assert_eq!(expected, changed(&before, &marked), "pick {n}");
        let list = with_pager(&mut sim, Pager::damage_list_page);
        let mut sim = Sim::with_files(seed, Config::default(), sim.files());
        finished(&mut sim, |rt| {
            open::open_in(&rt, FILE, &app(), &open::options(2))
                .unwrap()
                .close()
                .unwrap();
        });
        let mut second = changed(&marked, &sim.files()[FILE]);
        second.remove(&list.0);
        assert_eq!(BTreeSet::new(), second, "pick {n}, the second open");
    }
}

#[test]
fn a_marked_open_refuses_a_diverged_log_and_opens_with_none() {
    let (mut files, page) = seeded(20, status_page);
    DATA.flip(&mut files, page, 300 - HEADER_LEN);
    let mut sim = Sim::with_files(20, Config::default(), files);
    finished(&mut sim, |rt| {
        let got = open::open_in(&rt, FILE, &Log::new([CLEAN]), &open::options(2)).map(drop);
        assert!(
            matches!(got, Err(OpenError::Diverged { version: 1 })),
            "{got:?}"
        );
        let db = open::open_in(&rt, FILE, &none(), &open::options(2)).unwrap();
        assert!(holds_users(&db), "the file's own committed migration reads");
        assert_only_verify_runs(&db, Some("Checksum"));
        db.close().unwrap();
    });
}

#[test]
fn a_marked_open_resolves_no_id() {
    let (files, _) = seeded(21, status_page);
    let mut sim = Sim::with_files(21, Config::default(), files);
    let marks = with_pager(&mut sim, |p| {
        p.quarantine(Quarantine::Database, &ordered_for_test())
            .unwrap();
        p.quarantine_page()
    });
    let before = sim.files()[FILE].clone();
    finished(&mut sim, |rt| {
        open::open_in(&rt, FILE, &app(), &open::options(2))
            .unwrap()
            .close()
            .unwrap();
    });
    let mut other = changed(&before, &sim.files()[FILE]);
    other.remove(&marks.0);
    assert_eq!(BTreeSet::new(), other);
}

fn holds_users(db: &Database<SimRuntime>) -> bool {
    db.schema().relations.iter().any(|r| r.name == "users")
}

/// Returns a migration long enough to evict pages of its transaction, flushing the log before
/// its commit record.
fn long_migration() -> String {
    let mut text = APP.to_owned();
    for n in 0..20 {
        text.push_str(&format!(
            "add type T{n} = {{ id: Key t{n} }}\nadd relation t{n} : T{n} key id generate sequence\n"
        ));
    }
    text
}

/// A first migration's crash images, in the order the device saw them, and its status page.
struct Crashes {
    seed: u64,
    recording: adtb_sim::Recording,
    flushes: Vec<usize>,
    status: Pid,
}

impl Crashes {
    fn of(seed: u64) -> Crashes {
        let mut sim = Sim::new(seed, Config::default());
        sim.record();
        let mut start = 0;
        let mut status = None;
        finished(&mut sim, |rt| {
            let db = open::open_in(&rt, FILE, &none(), &open::options(2)).unwrap();
            status = Some(status_page(&db));
            db.close().unwrap();
            start = rt.recorded();
            let small = OpenOptions {
                pool_bytes: Some(32 * PAGE as u64),
                ..open::options(2)
            };
            let long = Log::new([long_migration()]);
            Database::open_in(rt, FILE, &long, &small)
                .unwrap()
                .close()
                .unwrap();
        });
        let recording = sim.recording();
        let flushes = recording
            .flushes()
            .into_iter()
            .filter(|&f| f >= start)
            .collect();
        Crashes {
            seed,
            recording,
            flushes,
            status: status.unwrap(),
        }
    }

    fn image(&self, flush: usize) -> Files {
        self.recording
            .images(self.flushes[flush], 1, self.seed)
            .swap_remove(0)
    }

    fn migrated(&self, flush: usize) -> bool {
        let mut sim = Sim::with_files(self.seed, Config::default(), self.image(flush));
        let mut migrated = false;
        finished(&mut sim, |rt| {
            migrated = holds_users(&open::open_in(&rt, FILE, &none(), &open::options(2)).unwrap());
        });
        migrated
    }

    /// Returns the first flush whose image reads as migrated: its commit record is durable, and
    /// the flush before holds the migration's rows without it.
    fn commit_flush(&self) -> usize {
        let (mut unmigrated, mut migrated) = (0, self.flushes.len() - 1);
        assert!(!self.migrated(unmigrated));
        assert!(self.migrated(migrated));
        while migrated - unmigrated > 1 {
            let mid = usize::midpoint(unmigrated, migrated);
            if self.migrated(mid) {
                migrated = mid;
            } else {
                unmigrated = mid;
            }
        }
        migrated
    }

    fn recovered(&self, image: Files) -> Files {
        let mut sim = Sim::with_files(self.seed, Config::default(), image);
        with_pager(&mut sim, |_| ());
        sim.files()
    }
}

fn none() -> Log {
    Log::new(Vec::<String>::new())
}

/// Returns the number of pages `a` and `b` both hold that differ.
fn differing(a: &[u8], b: &[u8]) -> usize {
    a.chunks(PAGE)
        .zip(b.chunks(PAGE))
        .filter(|(a, b)| a != b)
        .count()
}

/// Flip the status page, open twice under the empty log, and return whether the file's own
/// migration read. Each open is for `verify` alone and writes only the
/// mark past the file header.
fn opens_damaged(c: &Crashes, mut files: Files) -> bool {
    DATA.flip(&mut files, c.status, 300 - HEADER_LEN);
    let before = files[FILE].clone();
    let mut sim = Sim::with_files(c.seed, Config::default(), files);
    let mut read = false;
    for round in 0..2 {
        finished(&mut sim, |rt| {
            let db = open::open_in(&rt, FILE, &none(), &open::options(2))
                .unwrap_or_else(|e| panic!("round {round}: {e:?}"));
            read = holds_users(&db);
            assert_only_verify_runs(&db, Some("Checksum"));
            db.close().unwrap();
        });
    }
    let marks = with_pager(&mut sim, Pager::quarantine_page);
    let list = with_pager(&mut sim, Pager::damage_list_page);
    let mut other = changed(&before, &sim.files()[FILE]);
    other.remove(&marks.0);
    other.remove(&list.0);
    assert_eq!(BTreeSet::new(), other);
    read
}

/// The last flush before the commit record holds the migration's rows and no proof of them: the
/// damaged file does not read as migrated. The first flush after holds the proof, and does.
#[test]
fn a_crashed_migration_on_a_damaged_status_page_does_not_read_as_applied() {
    for seed in adtb_harness::seeds::sweep(30..31) {
        let c = Crashes::of(seed);
        let commit = c.commit_flush();
        let before = c.recovered(c.image(commit - 1));
        let empty = c.recovered(c.image(0));
        assert!(
            differing(&empty[FILE], &before[FILE]) > 1,
            "seed {seed}: the image holds rows of the running migration"
        );
        assert!(
            !opens_damaged(&c, before),
            "seed {seed}: read an uncommitted row"
        );
        let after = c.recovered(c.image(commit));
        assert!(
            opens_damaged(&c, after),
            "seed {seed}: the commit record proves the row"
        );
    }
}

/// Returns whether the catalog was read: `verify r` runs only then.
fn catalog_read(db: &Database<SimRuntime>) -> bool {
    match db.verify(Some("catalog::roles")) {
        Ok(_) => true,
        Err(e) => {
            quarantined::<()>(Err(e));
            false
        }
    }
}

#[test]
fn an_aborted_writer_on_a_damaged_status_page_leaves_the_catalog_unread() {
    for seed in adtb_harness::seeds::sweep(33..34) {
        let c = Crashes::of(seed);
        let commit = c.commit_flush();
        let mut sim = Sim::with_files(seed, Config::default(), c.image(commit - 1));
        finished(&mut sim, |rt| {
            let db = open::open_in(&rt, FILE, &none(), &open::options(2)).unwrap();
            assert!(
                !holds_users(&db),
                "seed {seed}: recovery aborts the migration"
            );
            db.close().unwrap();
        });
        let mut files = sim.files();
        DATA.flip(&mut files, c.status, 300 - HEADER_LEN);
        let mut sim = Sim::with_files(seed, Config::default(), files);
        for round in 0..2 {
            finished(&mut sim, |rt| {
                let db = open::open_in(&rt, FILE, &none(), &open::options(2))
                    .unwrap_or_else(|e| panic!("seed {seed}, round {round}: {e:?}"));
                assert!(!catalog_read(&db), "seed {seed}, round {round}");
                assert_only_verify_runs(&db, Some("Checksum"));
                db.close().unwrap();
            });
        }
    }
}

#[test]
fn a_committed_migration_on_a_damaged_status_page_reads_from_the_retained_log() {
    let (mut files, page) = seeded(31, status_page);
    DATA.flip(&mut files, page, 300 - HEADER_LEN);
    let mut sim = Sim::with_files(31, Config::default(), files);
    finished(&mut sim, |rt| {
        assert!(holds_users(
            &open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap()
        ));
    });
}

#[test]
fn a_file_whose_header_is_damaged_still_fails_open() {
    let (mut files, _) = seeded(32, status_page);
    for copy in (0..Pid::FIRST_DATA.0).map(Pid) {
        DATA.page_mut(&mut files, copy).fill(0x5a);
    }
    let mut sim = Sim::with_files(32, Config::default(), files);
    finished(&mut sim, |rt| {
        let got = open::open_in(&rt, FILE, &app(), &open::options(2));
        assert!(
            matches!(got, Err(OpenError::Corrupt { .. })),
            "{:?}",
            got.err()
        );
    });
}

fn dropped_on_damage(db: &Database<SimRuntime>) -> Pid {
    let mut open = db.begin(&TxnOptions::default()).unwrap();
    open.query_text(
        r#"insert users { id: users.key(5), name: "n" }"#,
        &[],
        &crate::TxnCallOptions::default(),
    )
    .unwrap();
    db.testing()
        .plant(Plant::Refuse {
            page: status_page(db),
            damage: Damage::Checksum,
        })
        .unwrap();
    drop(open);
    assert_eq!(
        "the database",
        quarantined(db.query_text(SELECT, &[], &CallOptions::default()))
    );
    let marks = db.testing().probe().marks();
    assert_eq!(BTreeSet::from([Quarantine::Database]), marks);
    status_page(db)
}

#[test]
fn dropping_a_transaction_on_a_damaged_commit_log_marks_the_database() {
    let mut sim = Sim::new(40, Config::default());
    let mut page = None;
    finished(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        page = Some(dropped_on_damage(&db));
        db.close().unwrap();
    });
    let mut files = sim.files();
    DATA.flip(&mut files, page.unwrap(), 300 - HEADER_LEN);
    let mut sim = Sim::with_files(40, Config::default(), files);
    finished(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        let pid = page.unwrap();
        let damage = Damage::Checksum;
        assert_eq!(
            Err(PageError { pid, damage }),
            db.testing().probe().page(pid).map(drop)
        );
        assert_only_verify_runs(&db, Some("Checksum"));
    });
}

#[test]
fn dropping_a_transaction_on_a_database_already_quarantined_does_not_panic() {
    let mut sim = Sim::new(41, Config::default());
    finished(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        let first = db.begin(&TxnOptions::default()).unwrap();
        db.testing()
            .plant(Plant::Refuse {
                page: status_page(&db),
                damage: Damage::Checksum,
            })
            .unwrap();
        let got = db.query_text(INSERT, &[], &CallOptions::default());
        assert_eq!("the database", quarantined(got));
        drop(first);
        let later = db.query_text(SELECT, &[], &CallOptions::default());
        assert_eq!("the database", quarantined(later));
    });
}

/// Returns a new scratch directory labelled `name` and the path of `app.adt` in it.
fn scratch(name: &str) -> (Scratch, std::path::PathBuf) {
    let dir = Scratch::new("open_damage", name);
    let path = dir.path().join("app.adt");
    (dir, path)
}

fn real_options() -> OpenOptions {
    OpenOptions {
        create: Some(crate::CreateOptions::default()),
        ..OpenOptions::default()
    }
}

fn real_damaged(name: &str, pick: fn(&Database) -> Pid) -> (Scratch, std::path::PathBuf) {
    let (dir, path) = scratch(name);
    let db = Database::open(&path, &app(), &real_options()).unwrap();
    db.query_text(
        r#"insert users { id: users.key(1), name: "a" }"#,
        &[],
        &CallOptions::default(),
    )
    .unwrap();
    let page = pick(&db);
    db.close().unwrap();
    let mut files = Files::from([(FILE.to_owned(), std::fs::read(&path).unwrap())]);
    DATA.flip(&mut files, page, 300 - HEADER_LEN);
    std::fs::write(&path, &files[FILE]).unwrap();
    (dir, path)
}

/// Assert `found` holds a finding starting `finding`, or with none only the note that a run
/// quarantined whole keeps no findings.
fn assert_findings(found: &[String], finding: Option<&str>) {
    let note =
        "where: \"the database: quarantined whole, so the findings of this run are not kept\" }";
    match finding {
        Some(finding) => assert!(found.iter().any(|r| r.starts_with(finding)), "{found:?}"),
        None => assert!(
            matches!(found, [only] if only.starts_with("Structure {") && only.ends_with(note)),
            "{found:?}"
        ),
    }
}

fn assert_real_opens_for_verify(path: &std::path::Path, log: &Log, finding: Option<&str>) {
    for round in 0..2 {
        let db = Database::open(path, log, &real_options())
            .unwrap_or_else(|e| panic!("round {round}: {e:?}"));
        let none = CallOptions::default();
        assert_eq!(
            "the database",
            quarantined(db.query_text(SELECT, &[], &none))
        );
        let found = api::printed(db.verify(None).unwrap());
        assert_findings(&found, finding);
        db.close().unwrap();
    }
}

#[test]
fn a_real_file_with_a_damaged_catalog_page_opens_for_verify() {
    let (_dir, path) = real_damaged("catalog_root", catalog_root);
    assert_real_opens_for_verify(&path, &app(), Some("Checksum"));
    let (_dir, path) = real_damaged("roles", roles_page);
    assert_real_opens_for_verify(&path, &app(), Some("Checksum"));
    let found = crate::verify_file(&path, &app(), &real_options()).unwrap();
    let found = api::printed(found);
    assert!(found.iter().any(|r| r.starts_with("Checksum")), "{found:?}");
}

#[test]
fn a_real_file_with_a_damaged_status_page_refuses_a_diverged_log_and_opens_with_none() {
    let (_dir, path) = real_damaged("status", status_page);
    let got = Database::open(&path, &Log::new([CLEAN]), &real_options()).map(drop);
    assert!(
        matches!(got, Err(OpenError::Diverged { version: 1 })),
        "{got:?}"
    );
    assert_real_opens_for_verify(&path, &none(), Some("Checksum"));
}

#[test]
fn dropping_a_transaction_on_a_damaged_commit_log_of_a_real_file_does_not_panic() {
    let (_dir, path) = scratch("drop");
    let db = Database::open(&path, &app(), &real_options()).unwrap();
    let mut open = db.begin(&TxnOptions::default()).unwrap();
    open.query_text(INSERT, &[], &crate::TxnCallOptions::default())
        .unwrap();
    let page = status_page(&db);
    db.testing()
        .plant(Plant::Refuse {
            page,
            damage: Damage::Checksum,
        })
        .unwrap();
    drop(open);
    let none = CallOptions::default();
    assert_eq!(
        "the database",
        quarantined(db.query_text(SELECT, &[], &none))
    );
    db.close().unwrap();
    let mut files = Files::from([(FILE.to_owned(), std::fs::read(&path).unwrap())]);
    DATA.flip(&mut files, page, 300 - HEADER_LEN);
    std::fs::write(&path, &files[FILE]).unwrap();
    assert_real_opens_for_verify(&path, &app(), Some("Checksum"));
}

/// Returns the files of a seeded, closed database with `nobody`, a role without `adhoc`, marked
/// whole with no page damaged, and the page of the marks.
fn marked_by_hand(seed: u64) -> (Files, Pid) {
    let mut sim = Sim::new(seed, Config::default());
    finished(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        api::results(&db, r#"insert users { id: users.key(1), name: "a" }"#);
        db.role_authority().create_role("nobody").unwrap();
        db.close().unwrap();
    });
    let marks = with_pager(&mut sim, |p| {
        p.quarantine(Quarantine::Database, &ordered_for_test())
            .unwrap();
        p.quarantine_page()
    });
    (sim.files(), marks)
}

const MARK_ROW: &str = "the database: quarantined whole";

fn assert_names_the_mark(rows: &[String]) {
    assert!(
        rows.iter()
            .any(|r| r.starts_with("Structure") && r.contains(MARK_ROW)),
        "{rows:?}"
    );
}

#[test]
fn verify_of_a_marked_database_keeps_no_findings_and_writes_no_page() {
    let (files, marks) = marked_by_hand(50);
    let before = files[FILE].clone();
    let mut sim = Sim::with_files(50, Config::default(), files);
    finished(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        assert!(catalog_read(&db));
        assert_names_the_mark(&api::printed(db.verify(None).unwrap()));
        assert_names_the_mark(&api::printed(db.testing().verify_holding().unwrap()));
        assert_names_the_mark(&api::printed(db.verify(Some("users")).unwrap()));
        db.close().unwrap();
    });
    let mut other = changed(&before, &sim.files()[FILE]);
    other.remove(&marks.0);
    assert_eq!(BTreeSet::new(), other);
}

#[test]
fn verify_names_the_mark_once_with_no_row_about_keeping() {
    let (mut files, page) = seeded(51, status_page);
    DATA.flip(&mut files, page, 300 - HEADER_LEN);
    let mut sim = Sim::with_files(51, Config::default(), files);
    finished(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        let rows = api::printed(db.verify(None).unwrap());
        assert_names_the_mark(&rows);
        assert!(
            !rows.iter().any(|r| r.contains("were not kept:")),
            "{rows:?}"
        );
    });
}

#[test]
fn verify_of_a_relation_on_a_damaged_commit_log_says_only_its_pages_were_checked() {
    let (mut files, page) = seeded(52, status_page);
    DATA.flip(&mut files, page, 300 - HEADER_LEN);
    let mut sim = Sim::with_files(52, Config::default(), files);
    finished(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        let rows = api::printed(db.verify(Some("users")).unwrap());
        assert!(
            rows.iter()
                .any(|r| r.starts_with("Structure") && r.contains("users: only its pages")),
            "{rows:?}"
        );
        assert!(
            !rows.iter().any(|r| r.contains("the pages and the log")),
            "{rows:?}"
        );
        let rows = api::printed(db.verify(None).unwrap());
        assert!(
            rows.iter().any(|r| r.contains("the pages and the log")),
            "{rows:?}"
        );
    });
}

#[test]
fn a_heap_mark_adds_no_row() {
    let mut sim = Sim::new(53, Config::default());
    finished(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        let t = db.testing();
        let heap = t.own_mark(t.heap("app::users", "User").unwrap()).unwrap();
        t.plant(Plant::Mark(heap)).unwrap();
        let rows = api::printed(db.verify(None).unwrap());
        assert!(!rows.iter().any(|r| r.contains(MARK_ROW)), "{rows:?}");
        db.close().unwrap();
    });
}

#[test]
fn a_call_refused_on_a_marked_database_writes_no_audit_row() {
    let (files, marks) = marked_by_hand(54);
    let before = files[FILE].clone();
    let mut sim = Sim::with_files(54, Config::default(), files);
    finished(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        let nobody = db.role_authority().session("nobody").unwrap();
        for text in ["verify", SELECT, "from nothing in nowhere select 1"] {
            let what = quarantined(api::task(&rt, &nobody, text));
            assert_eq!("the database", what, "{text}");
        }
        db.close().unwrap();
    });
    let mut other = changed(&before, &sim.files()[FILE]);
    other.remove(&marks.0);
    assert_eq!(BTreeSet::new(), other);
}

#[test]
fn a_real_file_with_a_damaged_page_of_an_empty_grants_heap_opens_for_verify() {
    let (_dir, path) = real_damaged("grants", grants_page);
    assert_real_opens_for_verify(&path, &app(), Some("Checksum"));
}

#[test]
fn verify_file_of_a_real_file_marked_with_nothing_damaged_finds_the_mark() {
    let (_dir, path) = scratch("marked");
    let db = Database::open(&path, &app(), &real_options()).unwrap();
    db.testing()
        .plant(Plant::Mark(Quarantine::Database))
        .unwrap();
    db.close().unwrap();
    let found = api::printed(crate::verify_file(&path, &app(), &real_options()).unwrap());
    assert_names_the_mark(&found);
}

/// Returns the files of a closed database under log segments small enough that later inserts
/// leave the first migration's commit record behind, with every segment but the last gone, and its
/// status page.
fn with_old_segments_gone(seed: u64) -> (Files, Pid) {
    let mut sim = Sim::new(seed, Config::default());
    let mut status = None;
    finished(&mut sim, |rt| {
        let options = Options {
            page_size: PAGE as u32,
            segment_size: 1 << 16,
            threads: 2,
            ..Options::new(FILE)
        };
        Pager::create(rt.clone(), &options).unwrap().close();
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        let name = "x".repeat(2000);
        for n in 2..80 {
            let insert = format!(r#"insert users {{ id: users.key({n}), name: "{name}" }}"#);
            api::results(&db, &insert);
        }
        status = Some(status_page(&db));
        db.close().unwrap();
    });
    let mut files = sim.files();
    let log = format!("{FILE}-log");
    let segments: Vec<String> = files.keys().filter(|k| k.contains(&log)).cloned().collect();
    assert!(segments.len() > 2, "{segments:?}");
    for gone in &segments[..segments.len() - 1] {
        files.remove(gone);
    }
    (files, status.unwrap())
}

#[test]
fn a_commit_record_gone_from_the_log_on_a_damaged_status_page_leaves_the_catalog_unread() {
    let seed = 34;
    let (mut files, status) = with_old_segments_gone(seed);
    let mut intact = Sim::with_files(seed, Config::default(), files.clone());
    finished(&mut intact, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        assert!(holds_users(&db), "the status page settles it");
        db.close().unwrap();
    });
    DATA.flip(&mut files, status, 300 - HEADER_LEN);
    let mut sim = Sim::with_files(seed, Config::default(), files);
    for round in 0..2 {
        finished(&mut sim, |rt| {
            let db = open::open_in(&rt, FILE, &app(), &open::options(2))
                .unwrap_or_else(|e| panic!("round {round}: {e:?}"));
            assert!(!catalog_read(&db), "round {round}");
            assert_only_verify_runs(&db, Some("Checksum"));
            db.close().unwrap();
        });
    }
}

/// Returns the pages of `file`, each with its kind and owner.
#[expect(
    clippy::disallowed_methods,
    reason = "reads owners from file images, which have no guard"
)]
fn page_kinds(file: &[u8]) -> BTreeMap<Pid, (u16, u32)> {
    file.chunks(PAGE)
        .enumerate()
        .skip(Pid::FIRST_DATA.0 as usize)
        .map(|(n, page)| {
            let found = (crate::storage::kind(page), crate::storage::owner(page));
            (Pid(n as u64), found)
        })
        .collect()
}

/// Returns the pages of `kinds` that a sweep flips first: one of each kind, one of each kind and
/// owner, then the rest.
fn flip_order(kinds: &BTreeMap<Pid, (u16, u32)>) -> Vec<Pid> {
    let mut first_of_kind = BTreeSet::new();
    let mut first_of_owner = BTreeSet::new();
    let mut order: Vec<(u8, Pid)> = kinds
        .iter()
        .map(|(&pid, &(kind, owner))| {
            let rank = if first_of_kind.insert(kind) {
                0
            } else if first_of_owner.insert((kind, owner)) {
                1
            } else {
                2
            };
            (rank, pid)
        })
        .collect();
    order.sort();
    order.into_iter().map(|(_, pid)| pid).collect()
}

/// Flip one page of each page kind of a fresh file, more under the deep profile. File and log
/// come back whole for each flip. Every flip opens, twice, and `verify` answers it.
#[test]
fn a_flip_of_any_page_kind_opens_and_verify_finds_it() {
    let seed = 60;
    let (files, _) = seeded(seed, status_page);
    let kinds = page_kinds(&files[FILE]);
    let count = kinds.values().map(|k| k.0).collect::<BTreeSet<_>>().len();
    let flips = usize::try_from(adtb_harness::seeds::sweep(0..count as u64).end).unwrap();
    for page in flip_order(&kinds).into_iter().take(flips) {
        let mut flipped = files.clone();
        DATA.flip(&mut flipped, page, 300 - HEADER_LEN);
        let mut sim = Sim::with_files(seed, Config::default(), flipped);
        for round in 0..2 {
            finished(&mut sim, |rt| {
                let db = open::open_in(&rt, FILE, &app(), &open::options(2))
                    .unwrap_or_else(|e| panic!("{page:?} {:?} round {round}: {e:?}", kinds[&page]));
                let rows = api::printed(db.verify(None).unwrap());
                assert!(
                    !rows.is_empty(),
                    "{page:?} {:?} round {round}",
                    kinds[&page]
                );
                db.close().unwrap();
            });
        }
    }
}

#[test]
fn a_row_that_does_not_decode_on_a_good_page_fails_open_unmarked() {
    let (files, _) = seeded(61, status_page);
    let mut sim = Sim::with_files(61, Config::default(), files);
    finished(&mut sim, |rt| {
        let options = Options {
            page_size: PAGE as u32,
            threads: 2,
            ..Options::new(FILE)
        };
        let (pager, _) = Pager::open(rt.clone(), &options).unwrap();
        let (catalog, txns) = crate::catalog::Catalog::open(rt, &pager).unwrap();
        let begun = txns.begin(&pager).unwrap();
        let x = begun.xid();
        let negative = crate::key::Value::Sum {
            ctor: 0,
            fields: vec![crate::key::Value::I64(-1); 4],
        };
        catalog
            .structures()
            .rows()
            .unwrap()
            .insert(&pager, x, &[negative])
            .unwrap();
        txns.commit(&pager, begun).unwrap();
        pager.close();
    });
    let mut rounds = Sim::with_files(61, Config::default(), sim.files());
    for round in 0..2 {
        finished(&mut rounds, |rt| {
            let got = open::open_in(&rt, FILE, &app(), &open::options(2));
            assert!(
                matches!(got, Err(OpenError::Corrupt { .. })),
                "round {round}: {:?}",
                got.err()
            );
        });
    }
    assert_eq!(
        BTreeSet::new(),
        with_pager(&mut rounds, |p| Probe::of(p).marks())
    );
}

/// A read fault lands on a page chosen by the seed, so some seeds fault the file header or a page
/// the open passes over. A seed that faults a page the open needs refuses the open.
#[test]
fn an_unreadable_page_fails_open_unmarked() {
    let (files, _) = seeded(62, status_page);
    let mut marks_of = Sim::with_files(62, Config::default(), files.clone());
    let marks = with_pager(&mut marks_of, Pager::quarantine_page);
    let mark_bytes = |f: &Files| DATA.page(f, marks).to_vec();
    let mut refused = 0;
    for seed in adtb_harness::seeds::sweep(0..16) {
        let mut sim = Sim::with_files(seed, Config::default(), files.clone());
        sim.set_faults(Faults {
            latent_p: 0.02,
            ..Faults::none(512)
        });
        finished(&mut sim, |rt| {
            let got = open::open_in(&rt, FILE, &app(), &open::options(2));
            if let Err(OpenError::Corrupt { detail }) = &got {
                refused += usize::from(detail.contains("Unreadable"));
            }
            if let Ok(db) = got {
                let marks = db.testing().probe().marks();
                assert!(!marks.contains(&Quarantine::Database), "seed {seed}");
                db.close().ok();
            }
        });
        assert_eq!(mark_bytes(&files), mark_bytes(&sim.files()), "seed {seed}");
    }
    assert!(refused > 0, "no seed faults a page the open needs");
}

#[test]
fn the_roots_heap_marks_the_database_at_open_and_nothing_once_running() {
    let (mut files, _) = seeded(42, catalog_root);
    let mut sim = Sim::with_files(42, Config::default(), files.clone());
    let mut page = None;
    finished(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        db.testing().plant(Plant::RefuseKeptStore).unwrap();
        assert!(!api::printed(db.verify(None).unwrap()).is_empty());
        assert_eq!(
            BTreeSet::new(),
            db.testing().probe().marks(),
            "once running"
        );
        db.close().unwrap();
    });
    let mut sim2 = Sim::with_files(42, Config::default(), files.clone());
    finished(&mut sim2, |rt| {
        let options = Options {
            page_size: PAGE as u32,
            threads: 2,
            ..Options::new(FILE)
        };
        let (p, _) = Pager::open(rt.clone(), &options).unwrap();
        let (catalog, _) = crate::catalog::Catalog::load(rt, &p).unwrap();
        page = Some(catalog.kept_pages(&p).unwrap()[0]);
    });
    DATA.flip(&mut files, page.unwrap(), 300 - HEADER_LEN);
    let mut sim = Sim::with_files(42, Config::default(), files);
    finished(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        assert_eq!(
            BTreeSet::from([Quarantine::Database]),
            db.testing().probe().marks(),
            "at open"
        );
        db.close().unwrap();
    });
}

#[test]
fn an_open_whose_walk_of_the_roots_heap_cannot_read_its_damaged_page_marks_nothing() {
    let (files, page) = seeded(44, |db| db.testing().root(Object::CatalogRows).unwrap());
    let mut sim = Sim::with_files(44, Config::default(), files);
    let marks = with_pager(&mut sim, Pager::quarantine_page);
    finished(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        let damage = Damage::Checksum;
        db.testing().plant(Plant::Refuse { page, damage }).unwrap();
        assert_eq!(BTreeSet::new(), db.testing().probe().marks());
        db.close().unwrap();
    });
    finished(&mut sim, |rt| {
        let reads = Match::kind(OpKind::Read).offset(DATA.range(page).start as u64);
        rt.plan(Plan::new().rule(reads, Action::Fail(IoError::Failed)));
        let Err(OpenError::Corrupt { detail }) =
            open::open_in(&rt, FILE, &app(), &open::options(2))
        else {
            panic!("the open reads the page");
        };
        let refused = PageError {
            pid: page,
            damage: Damage::Unreadable,
        };
        assert_eq!(format!("Page({refused:?})"), detail);
    });
    let marked = crate::storage::quarantine::decode(DATA.page(&sim.files(), marks));
    assert_eq!(Some(BTreeSet::new()), marked);
}

#[test]
fn a_stale_commit_times_page_opens_under_the_database_mark() {
    let mut sim = Sim::new(43, Config::default());
    let mut page = None;
    finished(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        api::results(&db, "set setting RetentionWindow = { length: 1h }");
        api::results(&db, r#"insert users { id: users.key(1), name: "a" }"#);
        page = Some(db.testing().root(Object::CommitTimes).unwrap());
        db.close().unwrap();
    });
    let (first, page) = (sim.files(), page.unwrap());
    finished(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        api::results(&db, r#"insert users { id: users.key(2), name: "b" }"#);
        db.close().unwrap();
    });
    let mut files = sim.files();
    assert_ne!(DATA.page(&first, page), DATA.page(&files, page));
    DATA.replace(&mut files, page, DATA.page(&first, page));
    let mut sim = Sim::with_files(43, Config::default(), files);
    finished(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        assert!(
            matches!(
                db.testing().probe().records().pages().get(&page),
                Some(Damage::Stale { .. })
            ),
            "the open records the lost write"
        );
        assert_eq!(
            BTreeSet::from([Quarantine::Database]),
            db.testing().probe().marks()
        );
        db.close().unwrap();
    });
}

#[test]
fn an_io_failure_opening_the_file_is_not_a_missing_header() {
    let files = Files::from([
        ("blocker".to_owned(), vec![1; PAGE]),
        ("empty.adt".to_owned(), Vec::new()),
    ]);
    let mut sim = Sim::with_files(0, Config::default(), files);
    finished(&mut sim, |rt| {
        let opened = |name: &str| {
            let options = Options {
                page_size: PAGE as u32,
                ..Options::new(name)
            };
            Pager::open(rt.clone(), &options).err()
        };
        assert!(
            matches!(
                opened("blocker/open.adt"),
                Some(crate::storage::OpenError::Io(IoError::NotADirectory))
            ),
            "{:?}",
            opened("blocker/open.adt")
        );
        assert!(
            matches!(
                opened("empty.adt"),
                Some(crate::storage::OpenError::NoHeader)
            ),
            "{:?}",
            opened("empty.adt")
        );
        let Err(OpenError::Io(e)) =
            Database::open_in(rt, "blocker/open.adt", &app(), &open::options(2))
        else {
            panic!("the open is no I/O failure");
        };
        assert_eq!(std::io::ErrorKind::NotADirectory, e.kind());
    });
}
