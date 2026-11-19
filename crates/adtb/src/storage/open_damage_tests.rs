//! Opening, verifying and dropping a transaction on a file with a page no relation holds damaged:
//! the file opens for `verify` only and every other call fails `Quarantined`.

use crate::testing::{Object, Plant, Probe};
use adtb_testkit::process::finished;
use adtb_testkit::scratch::Scratch;
use std::collections::{BTreeMap, BTreeSet};

use crate::runtime::{IoError, OpKind};
use crate::storage::order::ordered_for_test;
use crate::storage::{Damage, HEADER_LEN, Options, PageError, Pager, Pid, Quarantine};
use crate::test_world::api;
use crate::test_world::load as open;
use crate::{CallOptions, Database, Error, FatalReason, Log, OpenError, OpenOptions, TxnOptions};
use adtb_sim::{Action, Config, Match, Plan, Sim, SimRuntime};
use adtb_testkit::damage::{DataFile, Files};

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

/// Assert only `verify` runs on `db`, and its findings include the `Checksum` finding of page
/// `checksum`, or with none are only the note that a run quarantined whole keeps no findings.
fn assert_only_verify_runs(db: &Database<SimRuntime>, checksum: Option<Pid>) {
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
    assert_findings(&found, checksum);
    let found = api::printed(db.testing().verify_holding().unwrap());
    assert_findings(&found, checksum);
}

fn assert_opens_for_verify(seed: u64, mut files: Files, page: Pid) {
    DATA.flip(&mut files, page, 300 - HEADER_LEN);
    let mut sim = Sim::with_files(seed, Config::default(), files);
    for round in 0..2 {
        finished(&mut sim, |rt| {
            let db = open::open_in(&rt, FILE, &app(), &open::options(2))
                .unwrap_or_else(|e| panic!("round {round}: {e:?}"));
            assert_only_verify_runs(&db, Some(page));
            db.close().unwrap();
        });
    }
}

/// Space: a flip of a page of the catalog's rows, of an empty grants heap, of the catalog's root,
/// of the status page and of the commit log's root and directory, on the simulator, and of the
/// first three in a real file; two opens of each.
#[test]
fn a_flip_of_a_page_no_relation_holds_opens_for_verify_only_and_verify_names_it() {
    type RealPick = fn(&Database) -> Pid;
    let (files, _) = seeded(1, status_page);
    let mut sim = Sim::with_files(1, Config::default(), files.clone());
    let mut pages = Vec::new();
    finished(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        pages = [
            roles_page,
            grants_page,
            catalog_root,
            status_page,
            clog_root,
            clog_directory,
        ]
        .map(|pick| pick(&db))
        .to_vec();
    });
    assert_eq!(6, pages.iter().collect::<BTreeSet<_>>().len(), "{pages:?}");
    for page in pages {
        assert_opens_for_verify(1, files.clone(), page);
    }
    let real: [(&str, RealPick); 3] = [
        ("roles", roles_page),
        ("grants", grants_page),
        ("catalog_root", catalog_root),
    ];
    for (name, pick) in real {
        let (_dir, path, page) = real_damaged(name, pick);
        assert_real_opens_for_verify(&path, &app(), Some(page));
        let found = api::printed(crate::verify_file(&path, &app(), &real_options()).unwrap());
        assert_findings(&found, Some(page));
    }
}

#[test]
fn verify_of_an_unread_catalog_says_what_it_did_not_check() {
    let (mut files, page) = seeded(6, catalog_root);
    DATA.flip(&mut files, page, 300 - HEADER_LEN);
    let mut sim = Sim::with_files(6, Config::default(), files);
    finished(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        let found = api::printed(db.verify(None).unwrap());
        assert_eq!(vec![UNREAD, NOT_KEPT], structure_places(&found));
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

/// Space: a flipped status page, on the simulator and in a real file.
#[test]
fn a_marked_open_refuses_a_diverged_log_and_opens_with_none() {
    let (mut files, page) = seeded(20, status_page);
    DATA.flip(&mut files, page, 300 - HEADER_LEN);
    let mut sim = Sim::with_files(20, Config::default(), files);
    finished(&mut sim, |rt| {
        let got = open::open_in(&rt, FILE, &Log::new([CLEAN]), &open::options(2));
        assert_diverged_at_1(got.map(drop));
        let db = open::open_in(&rt, FILE, &none(), &open::options(2)).unwrap();
        assert!(holds_users(&db), "the file's own committed migration reads");
        assert_only_verify_runs(&db, Some(page));
        db.close().unwrap();
    });
    let (_dir, path, page) = real_damaged("status", status_page);
    let got = Database::open(&path, &Log::new([CLEAN]), &real_options());
    assert_diverged_at_1(got.map(drop));
    assert_real_opens_for_verify(&path, &none(), Some(page));
}

fn assert_diverged_at_1(got: Result<(), OpenError>) {
    let Err(OpenError::Diverged { version }) = got else {
        panic!("not Diverged: {got:?}");
    };
    assert_eq!(1, version);
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

/// Flip `status`, open twice under the empty log, and return whether each open holds the file's
/// own migration and reads the catalog. Each open is for `verify` alone and writes only the mark
/// and the damage list past the file header.
fn opens_damaged(seed: u64, status: Pid, mut files: Files) -> Vec<(bool, bool)> {
    DATA.flip(&mut files, status, 300 - HEADER_LEN);
    let before = files[FILE].clone();
    let mut sim = Sim::with_files(seed, Config::default(), files);
    let mut read = Vec::new();
    for round in 0..2 {
        finished(&mut sim, |rt| {
            let db = open::open_in(&rt, FILE, &none(), &open::options(2))
                .unwrap_or_else(|e| panic!("round {round}: {e:?}"));
            read.push((holds_users(&db), catalog_read(&db)));
            assert_only_verify_runs(&db, Some(status));
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

/// Space: a status page flipped under a first migration whose commit record is not yet durable,
/// durable, durable and retained in a closed file's log, or gone from the log with every older
/// segment, or whose writer recovery aborted; two opens of each under the empty log.
#[test]
fn a_damaged_status_page_reads_the_catalog_only_when_the_log_proves_its_commit() {
    let c = Crashes::of(30);
    let commit = c.commit_flush();
    let unproven = c.recovered(c.image(commit - 1));
    let empty = c.recovered(c.image(0));
    assert!(
        differing(&empty[FILE], &unproven[FILE]) > 1,
        "the image holds rows of the running migration"
    );
    let mut aborted = Sim::with_files(c.seed, Config::default(), c.image(commit - 1));
    finished(&mut aborted, |rt| {
        let db = open::open_in(&rt, FILE, &none(), &open::options(2)).unwrap();
        assert!(!holds_users(&db), "recovery aborts the migration");
        db.close().unwrap();
    });
    let (retained, retained_status) = seeded(31, status_page);
    let (gone, gone_status) = with_old_segments_gone(34);
    let mut intact = Sim::with_files(34, Config::default(), gone.clone());
    finished(&mut intact, |rt| {
        let db = open::open_in(&rt, FILE, &none(), &open::options(2)).unwrap();
        assert!(holds_users(&db), "the status page settles it");
        db.close().unwrap();
    });
    let rows = [
        ("unproven", c.seed, unproven, c.status, false),
        (
            "proven",
            c.seed,
            c.recovered(c.image(commit)),
            c.status,
            true,
        ),
        ("aborted", c.seed, aborted.files(), c.status, false),
        ("retained", 31, retained, retained_status, true),
        ("gone", 34, gone, gone_status, false),
    ];
    let (mut got, mut want) = (Vec::new(), Vec::new());
    for (name, seed, files, status, read) in rows {
        got.push((name, opens_damaged(seed, status, files)));
        want.push((name, vec![(read, read); 2]));
    }
    assert_eq!(want, got);
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
fn a_file_whose_header_is_damaged_still_fails_open() {
    let (mut files, _) = seeded(32, status_page);
    for copy in (0..Pid::FIRST_DATA.0).map(Pid) {
        DATA.page_mut(&mut files, copy).fill(0x5a);
    }
    let mut sim = Sim::with_files(32, Config::default(), files);
    finished(&mut sim, |rt| {
        let Err(OpenError::Corrupt { detail }) =
            open::open_in(&rt, FILE, &app(), &open::options(2))
        else {
            panic!("the open is refused Corrupt");
        };
        assert_eq!("neither copy of the file header verifies", detail);
    });
}

/// Drop a transaction that wrote once its status page is refused, after a call the refusal
/// marked the database for when `marked_first`, and return the status page.
fn dropped_on_damage<R: crate::runtime::Runtime>(db: &Database<R>, marked_first: bool) -> Pid {
    let mut open = db.begin(&TxnOptions::default()).unwrap();
    open.query_text(
        r#"insert users { id: users.key(5), name: "n" }"#,
        &[],
        &crate::TxnCallOptions::default(),
    )
    .unwrap();
    let page = status_page(db);
    db.testing()
        .plant(Plant::Refuse {
            page,
            damage: Damage::Checksum,
        })
        .unwrap();
    if marked_first {
        let got = db.query_text(INSERT, &[], &CallOptions::default());
        assert_eq!("the database", quarantined(got));
    }
    drop(open);
    assert_eq!(
        "the database",
        quarantined(db.query_text(SELECT, &[], &CallOptions::default()))
    );
    let marks = db.testing().probe().marks();
    assert_eq!(BTreeSet::from([Quarantine::Database]), marks);
    page
}

/// Space: a transaction dropped on a refused status page, the database unmarked or already marked,
/// on the simulator, and unmarked in a real file; then, unmarked, an open of the file with the page
/// flipped.
#[test]
fn dropping_a_transaction_on_a_damaged_commit_log_marks_the_database() {
    for marked_first in [false, true] {
        let mut sim = Sim::new(40, Config::default());
        let mut page = None;
        finished(&mut sim, |rt| {
            let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
            page = Some(dropped_on_damage(&db, marked_first));
            db.close().unwrap();
        });
        if marked_first {
            continue;
        }
        let pid = page.unwrap();
        let mut files = sim.files();
        DATA.flip(&mut files, pid, 300 - HEADER_LEN);
        let mut sim = Sim::with_files(40, Config::default(), files);
        finished(&mut sim, |rt| {
            let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
            let damage = Damage::Checksum;
            assert_eq!(
                Err(PageError { pid, damage }),
                db.testing().probe().page(pid).map(drop)
            );
            assert_only_verify_runs(&db, Some(pid));
            db.close().unwrap();
        });
    }
    let (_dir, path) = scratch("drop");
    let db = Database::open(&path, &app(), &real_options()).unwrap();
    let page = dropped_on_damage(&db, false);
    db.close().unwrap();
    let mut files = Files::from([(FILE.to_owned(), std::fs::read(&path).unwrap())]);
    DATA.flip(&mut files, page, 300 - HEADER_LEN);
    std::fs::write(&path, &files[FILE]).unwrap();
    assert_real_opens_for_verify(&path, &app(), Some(page));
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

fn real_damaged(name: &str, pick: fn(&Database) -> Pid) -> (Scratch, std::path::PathBuf, Pid) {
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
    (dir, path, page)
}

/// Assert `found` holds the `Checksum` finding of page `checksum`, or with none only the note that
/// a run quarantined whole keeps no findings.
fn assert_findings(found: &[String], checksum: Option<Pid>) {
    match checksum {
        Some(page) => {
            let place = format!("where: \"page {}: Checksum\" }}", page.0);
            let named = |r: &&String| r.starts_with("Checksum {") && r.ends_with(&place);
            assert!(found.iter().any(|r| named(&r)), "{page:?}: {found:?}");
        }
        None => assert_eq!((1, vec![NOT_KEPT]), (found.len(), structure_places(found))),
    }
}

pub(super) const NOT_KEPT: &str =
    "the database: quarantined whole, so the findings of this run are not kept";
pub(super) const UNREAD: &str = "the database: only the pages and the log were checked";

/// Returns the constructor and place of each row of a `verify`.
pub(super) fn places(rows: &[String]) -> Vec<(String, String)> {
    let row = |r: &String| {
        let (ctor, rest) = r.split_once(" {").unwrap();
        let (_, at) = rest.split_once("where: \"").unwrap();
        (ctor.to_owned(), at.strip_suffix("\" }").unwrap().to_owned())
    };
    rows.iter().map(row).collect()
}

/// Returns the place of each `Structure` row of `rows`, sorted.
fn structure_places(rows: &[String]) -> Vec<&str> {
    let mut places: Vec<&str> = rows
        .iter()
        .filter_map(|r| r.strip_prefix("Structure {"))
        .map(|r| {
            let (_, at) = r.split_once("where: \"").unwrap();
            at.strip_suffix("\" }").unwrap()
        })
        .collect();
    places.sort_unstable();
    places
}

fn assert_real_opens_for_verify(path: &std::path::Path, log: &Log, checksum: Option<Pid>) {
    for round in 0..2 {
        let db = Database::open(path, log, &real_options())
            .unwrap_or_else(|e| panic!("round {round}: {e:?}"));
        let none = CallOptions::default();
        assert_eq!(
            "the database",
            quarantined(db.query_text(SELECT, &[], &none))
        );
        let found = api::printed(db.verify(None).unwrap());
        assert_findings(&found, checksum);
        db.close().unwrap();
    }
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

/// Space: a database marked whole with nothing damaged, verified whole, holding and of a relation,
/// and verified as a real file; and a database with only a heap marked.
#[test]
fn verify_of_a_marked_database_keeps_no_findings_and_writes_no_page() {
    let (files, marks) = marked_by_hand(50);
    let before = files[FILE].clone();
    let mut sim = Sim::with_files(50, Config::default(), files);
    let marked = vec![NOT_KEPT];
    finished(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        assert!(catalog_read(&db));
        let whole = api::printed(db.verify(None).unwrap());
        let holding = api::printed(db.testing().verify_holding().unwrap());
        let relation = api::printed(db.verify(Some("users")).unwrap());
        let got = [&whole, &holding, &relation].map(|r| structure_places(r));
        assert_eq!([marked.clone(), marked.clone(), marked.clone()], got);
        db.close().unwrap();
    });
    let mut other = changed(&before, &sim.files()[FILE]);
    other.remove(&marks.0);
    assert_eq!(BTreeSet::new(), other);

    let (_dir, path) = scratch("marked");
    let db = Database::open(&path, &app(), &real_options()).unwrap();
    db.testing()
        .plant(Plant::Mark(Quarantine::Database))
        .unwrap();
    db.close().unwrap();
    let found = api::printed(crate::verify_file(&path, &app(), &real_options()).unwrap());
    assert_eq!(marked, structure_places(&found));

    let mut sim = Sim::new(53, Config::default());
    finished(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        let t = db.testing();
        let heap = t.own_mark(t.heap("app::users", "User").unwrap()).unwrap();
        t.plant(Plant::Mark(heap)).unwrap();
        let rows = api::printed(db.verify(None).unwrap());
        assert_eq!(Vec::<String>::new(), rows, "a heap mark adds no row");
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
    let flips = usize::try_from(adtb_testkit::seeds::sweep(0..count as u64).end).unwrap();
    for page in flip_order(&kinds).into_iter().take(flips) {
        let mut flipped = files.clone();
        DATA.flip(&mut flipped, page, 300 - HEADER_LEN);
        let mut sim = Sim::with_files(seed, Config::default(), flipped);
        for round in 0..2 {
            finished(&mut sim, |rt| {
                let db = open::open_in(&rt, FILE, &app(), &open::options(2))
                    .unwrap_or_else(|e| panic!("{page:?} {:?} round {round}: {e:?}", kinds[&page]));
                let rows = api::printed(db.verify(None).unwrap());
                assert_findings(&rows, Some(page));
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
            let Err(OpenError::Corrupt { detail }) =
                open::open_in(&rt, FILE, &app(), &open::options(2))
            else {
                panic!("round {round}: the open is refused Corrupt");
            };
            let negative = "Corrupt(\"a roots row holds negative id -1\")";
            assert_eq!(negative, detail, "round {round}");
        });
    }
    assert_eq!(
        BTreeSet::new(),
        with_pager(&mut rounds, |p| Probe::of(p).marks())
    );
}

/// Space: every read of one page the open needs failing, for the catalog's rows, its root, the
/// status page and the commit log's root and directory.
#[test]
fn an_unreadable_page_fails_open_unmarked() {
    let (files, _) = seeded(62, status_page);
    let mut sim = Sim::with_files(62, Config::default(), files.clone());
    let marks = with_pager(&mut sim, Pager::quarantine_page);
    let mut pages = Vec::new();
    finished(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        let picks = [
            roles_page,
            catalog_root,
            status_page,
            clog_root,
            clog_directory,
        ];
        pages = picks.map(|pick| pick(&db)).to_vec();
        db.close().unwrap();
    });
    let (mut got, mut want) = (Vec::new(), Vec::new());
    for &pid in &pages {
        let mut sim = Sim::with_files(62, Config::default(), files.clone());
        finished(&mut sim, |rt| {
            let reads = Match::kind(OpKind::Read).offset(DATA.range(pid).start as u64);
            rt.plan(Plan::new().rule(reads, Action::Fail(IoError::Failed)));
            let detail = match open::open_in(&rt, FILE, &app(), &open::options(2)) {
                Err(OpenError::Corrupt { detail }) => detail,
                other => format!("not Corrupt: {:?}", other.map(drop)),
            };
            got.push((pid, detail));
        });
        let unreadable = PageError {
            pid,
            damage: Damage::Unreadable,
        };
        want.push((pid, format!("Page({unreadable:?})")));
        assert_eq!(
            DATA.page(&files, marks),
            DATA.page(&sim.files(), marks),
            "{pid:?}"
        );
    }
    assert_eq!(want, got);
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
        assert_eq!(
            Some(crate::storage::OpenError::Io(IoError::NotADirectory)),
            opened("blocker/open.adt")
        );
        assert_eq!(
            Some(crate::storage::OpenError::NoHeader),
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

/// Returns the files of `FILE` holding `rows` users, written under `seed`.
fn with_users(seed: u64, rows: i64) -> Files {
    let mut sim = Sim::new(seed, Config::default());
    adtb_testkit::process::run(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &app(), &open::options(2));
        for id in 0..rows {
            let text = format!(r#"insert users {{ id: users.key({id}), name: "user {id}" }}"#);
            api::results(&db, &text);
        }
        db.close().unwrap();
    });
    sim.files()
}

/// Refuse a log another file wrote, placed beside a file, as long, shorter or longer than its own,
/// as foreign.
#[test]
#[ignore = "the pager answers a whole log of another file as `Damaged` at its first block, not `Foreign`"]
fn a_foreign_log_beside_a_file_is_refused_as_foreign() {
    let log_dir = format!("{FILE}-log/");
    let ours = with_users(7, 8);
    for rows in [8, 0, 40] {
        let mut files = ours.clone();
        files.retain(|path, _| !path.starts_with(&log_dir));
        let theirs = with_users(8, rows);
        files.extend(theirs.into_iter().filter(|(p, _)| p.starts_with(&log_dir)));
        let mut sim = Sim::with_files(9, Config::default(), files);
        let (cause, got) = adtb_testkit::process::run(&mut sim, |rt| {
            let cause = Pager::open(rt.clone(), &Options::new(FILE)).err();
            let none = OpenOptions {
                create: None,
                ..open::options(2)
            };
            (
                cause,
                Database::open_in(rt, FILE, &app(), &none).map(Database::close),
            )
        });
        let Some(
            cause @ crate::storage::OpenError::Log(crate::storage::LogFinding {
                problem: crate::storage::LogProblem::Foreign,
                ..
            }),
        ) = cause
        else {
            panic!("a log of {rows} rows: the pager answers {cause:?}");
        };
        let Err(OpenError::Corrupt { detail }) = got else {
            panic!("a log of {rows} rows opens");
        };
        assert_eq!(format!("{cause:?}"), detail, "a log of {rows} rows");
    }
}

/// Refuse a log of this file whose tail holds a block another file wrote, naming it foreign.
#[test]
fn a_foreign_block_past_the_end_of_a_log_is_refused_as_foreign() {
    const BLOCK: usize = 512;
    let segment = format!("{FILE}-log/");
    let mut files = with_users(7, 8);
    let theirs = with_users(8, 40);
    let (name, ours) = files
        .iter_mut()
        .find(|(path, _)| path.starts_with(&segment))
        .expect("a segment");
    let end = ours
        .chunks(BLOCK)
        .position(|block| block.iter().all(|&b| b == 0))
        .expect("the log ends before the segment does")
        * BLOCK;
    let tail = &theirs[name.as_str()][end..end + BLOCK];
    assert!(tail.iter().any(|&b| b != 0), "their log passes ours");
    ours[end..end + BLOCK].copy_from_slice(tail);
    let mut sim = Sim::with_files(9, Config::default(), files.clone());
    let cause =
        adtb_testkit::process::run(&mut sim, |rt| Pager::open(rt, &Options::new(FILE)).err());
    let Some(
        cause @ crate::storage::OpenError::Log(crate::storage::LogFinding {
            problem: crate::storage::LogProblem::Foreign,
            ..
        }),
    ) = cause
    else {
        panic!("the pager refuses the log as foreign: {cause:?}");
    };
    let mut sim = Sim::with_files(9, Config::default(), files);
    let got = adtb_testkit::process::run(&mut sim, |rt| {
        let none = OpenOptions {
            create: None,
            ..open::options(2)
        };
        Database::open_in(rt, FILE, &app(), &none).map(Database::close)
    });
    let Err(OpenError::Corrupt { detail }) = got else {
        panic!("the open is no refusal of a corrupt file");
    };
    assert_eq!(format!("{cause:?}"), detail);
}

/// Answer a log a read-only check cannot list as a finding of an unreadable log.
#[test]
fn a_check_reports_a_log_it_cannot_list() {
    let mut sim = Sim::with_files(28, Config::default(), with_users(1, 2));
    adtb_testkit::process::run(&mut sim, |rt| {
        let db = open::open_in(&rt, FILE, &app(), &open::options(2)).unwrap();
        let lists = Match::kind(OpKind::List).prefixed(FILE);
        rt.plan(Plan::new().rule(lists, Action::Fail(IoError::Failed)));
        let t = db.testing();
        assert!(t.probe().logged_commits().is_empty());
        let unreadable = crate::storage::Finding::Log(crate::storage::LogFinding {
            lsn: crate::storage::Lsn(0),
            problem: crate::storage::LogProblem::Unreadable,
        });
        assert_eq!(vec![unreadable], t.verify_pages().findings);
        rt.plan(Plan::new());
        db.close().unwrap();
    });
}
