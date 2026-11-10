//! A bitmap page lost or zeroed is damage and never free space. Each damaged file refuses the call
//! and marks, and leaves named pages as they were. A healthy file, cut by no fault, never refuses.

use crate::testing::{Failed, Knobs, Object, Plant, Probe};
use adtb_harness::process::finished;
use std::collections::{BTreeMap, BTreeSet};

use crate::key::Value;
use crate::runtime::Threads;
use crate::storage::policy::{Holder, Phase};
use crate::storage::{
    self, Crc32c, Damage, HEADER_LEN, Lsn, Options, PageError, PageKind, Pager, Pid, Quarantine,
};
use crate::test_world::api;
use crate::test_world::load as open;
use crate::{CallOptions, Database, Error, FatalReason, Log, OpenOptions};
use adtb_harness::damage::{DataFile, Files};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_testkit::profile::sweep;

const FILE: &str = "lost_bitmap.adt";
const PAGE: usize = 8192;
const SPAN: usize = (PAGE - HEADER_LEN) * 8;
const SMALL_PAGE: usize = 4096;
const SMALL_SPAN: u64 = ((SMALL_PAGE - HEADER_LEN) * 8) as u64;
const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text, tag: I64 }
add relation users : User key id generate sequence
  index (tag)
add type Other = { id: Key others, name: Text }
add relation others : Other key id generate sequence
";
const SELECT: &str = "from u in users select u.id";
const INSERT_OTHER: &str = r#"insert others { id: others.key(1), name: "o" }"#;
const INSERT_USER: &str = r#"insert users { id: users.key(90), name: "u", tag: 1 }"#;
const HEADER_REGION: usize = 4096;
const DATA: DataFile = DataFile::new(FILE, PAGE);
const SMALL: DataFile = DataFile::new(FILE, SMALL_PAGE);
const ROOTS_HEAP: u32 = 2;

fn options(frames: u64) -> OpenOptions {
    OpenOptions {
        pool_bytes: Some(frames * PAGE as u64),
        ..open::options(2)
    }
}

fn text(db: &Database<SimRuntime>, query: &str) -> Result<Vec<String>, Error> {
    db.query_text(query, &[], &CallOptions::default())
        .map(|mut r| {
            if r.is_empty() {
                Vec::new()
            } else {
                api::printed(r.remove(0))
            }
        })
}

fn insert_users(db: &Database<SimRuntime>, ids: std::ops::Range<i64>, name: &str) {
    for i in ids {
        api::results(
            db,
            &format!(r#"insert users {{ id: users.key({i}), name: "{name}", tag: 1 }}"#),
        );
    }
}

fn fill_first_group(db: &Database<SimRuntime>) {
    db.testing()
        .plant(Plant::Write {
            page: Pid::FIRST_DATA,
            at: HEADER_LEN,
            bytes: vec![0xff; PAGE - HEADER_LEN],
        })
        .unwrap();
}

fn first_users_page(db: &Database<SimRuntime>) -> Pid {
    let t = db.testing();
    t.pages(t.heap("app::users", "User").unwrap()).unwrap()[0]
}

/// Returns the files of a database whose first group is full and whose users live in the second.
fn two_groups(row: &str) -> Files {
    let mut sim = Sim::new(1, Config::default());
    finished(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options(256));
        fill_first_group(&db);
        insert_users(&db, 0..3, row);
        assert!(first_users_page(&db).0 > SPAN as u64, "a page in group 1");
        db.close().unwrap();
    });
    sim.files()
}

/// Returns the files of a database with one small group, and what `pick` chooses of it.
fn one_group<T>(row: &str, pick: impl FnOnce(&Database<SimRuntime>) -> T) -> (Files, T) {
    let mut sim = Sim::new(1, Config::default());
    let mut picked = None;
    finished(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options(256));
        insert_users(&db, 0..4, row);
        picked = Some(pick(&db));
        db.close().unwrap();
    });
    (sim.files(), picked.unwrap())
}

fn assert_quarantined<T: std::fmt::Debug>(got: &Result<T, Error>, what: &str) {
    assert!(
        matches!(
            got,
            Err(Error::Fatal {
                reason: FatalReason::Quarantined { .. }
            })
        ),
        "{what}: {got:?}"
    );
}

/// What a refused call wrote before it met the damage, each page keeping its kind and owner.
#[derive(Clone, Copy, PartialEq)]
enum Writes {
    /// Only status pages at open, and scrub cursor rows in the roots heap while unmarked.
    Open,
    /// Also pages of the users heap.
    Vacuum,
    /// Also the heap, every index, the counters and the first bitmap, for a row and its entries.
    Row,
}

struct Kept {
    marks: Pid,
    list: Pid,
    users: u32,
    counters: Pid,
    writes: Writes,
}

impl Kept {
    #[expect(
        clippy::disallowed_methods,
        reason = "reads owners from file images, which have no guard"
    )]
    fn may_change(&self, page: Pid, bytes: &[u8]) -> bool {
        let kind = storage::kind(bytes);
        let owner = storage::owner(bytes);
        let index = kind == PageKind::IndexLeaf as u16 || kind == PageKind::IndexInner as u16;
        kind == PageKind::CommitLog as u16
            || owner == ROOTS_HEAP
            || self.writes != Writes::Open && owner == self.users
            || self.writes == Writes::Row
                && (index || page == self.counters || page == Pid::FIRST_DATA)
    }

    /// Assert every byte of every page of `before` stands in `after`, leaving out the header copies,
    /// the marks page, the damage list and the pages `may_change` names.
    #[expect(
        clippy::disallowed_methods,
        reason = "reads owners from file images, which have no guard"
    )]
    fn check(&self, before: &Files, after: &Files, what: &str) {
        let (a, b) = (&before[FILE], &after[FILE]);
        assert!(b.len() >= a.len(), "{what}: the file shrank");
        for (i, (x, y)) in a.chunks(PAGE).zip(b.chunks(PAGE)).enumerate() {
            let page = Pid(i as u64);
            if page < Pid::FIRST_DATA || page == self.marks || page == self.list {
                continue;
            }
            if self.may_change(page, x) {
                let shape = |p: &[u8]| (storage::kind(p), storage::owner(p));
                assert_eq!(shape(x), shape(y), "{what}: page {i} was reused");
            } else {
                assert!(x == y, "{what}: page {i} changed");
            }
        }
    }
}

/// Run `body` on an open of `files`, and assert it changed no page beyond `writes`. Returns the
/// files it left.
fn damaged_writing(
    files: &Files,
    writes: Writes,
    what: &str,
    body: impl FnOnce(&Database<SimRuntime>),
) -> Files {
    let mut sim = Sim::with_files(1, Config::default(), files.clone());
    let mut kept = None;
    finished(&mut sim, |rt| {
        let quiet = OpenOptions {
            testing: Knobs::without_background_vacuum(),
            ..options(256)
        };
        let db = Database::open_in(rt, FILE, &Log::new([APP]), &quiet).unwrap();
        let t = db.testing();
        kept = Some(Kept {
            marks: t.root(Object::Marks).unwrap(),
            list: t.root(Object::DamageList).unwrap(),
            users: t
                .heap("app::users", "User")
                .map_or(0, |h| t.owner(h).unwrap()),
            counters: t.root(Object::Sequences).unwrap(),
            writes,
        });
        body(&db);
        db.close().unwrap();
    });
    let after = sim.files();
    kept.unwrap().check(files, &after, what);
    after
}

fn damaged_run(files: &Files, what: &str, body: impl FnOnce(&Database<SimRuntime>)) -> Files {
    damaged_writing(files, Writes::Open, what, body)
}

/// Assert a write finds the lost bitmap, marks the database, and every call then fails.
fn assert_all_refused_as_database(db: &Database<SimRuntime>, what: &str) {
    assert_quarantined(&text(db, INSERT_OTHER), what);
    assert_eq!(
        BTreeSet::from([Quarantine::Database]),
        db.testing().probe().marks(),
        "{what}"
    );
    assert_quarantined(&text(db, SELECT), what);
}

#[test]
fn a_cut_losing_a_group_with_large_values_refuses_marks_and_stays_marked() {
    adtb_heap_cap::raise_cap(3 << 30);
    let row = "l".repeat(3 * PAGE);
    let mut files = two_groups(&row);
    DATA.truncate(&mut files, SPAN - 100);
    let after = damaged_run(&files, "lost group", |db| {
        assert_quarantined(&text(db, INSERT_USER), "large insert");
        assert_quarantined(&text(db, INSERT_USER), "retry");
        assert_quarantined(&text(db, INSERT_OTHER), "other relation");
        let marks = db.testing().probe().marks();
        assert!(!marks.is_empty(), "{marks:?}");
        assert_quarantined(&text(db, SELECT), "select");
    });
    damaged_run(&after, "lost group reopened", |db| {
        let found = api::printed(db.verify(None).unwrap());
        assert!(
            found.iter().any(|f| f.starts_with("Structure")),
            "{found:?}"
        );
        assert_quarantined(&text(db, SELECT), "reopened select");
        assert_quarantined(&text(db, INSERT_OTHER), "reopened insert");
    });
}

/// Returns the files of a database with five users, whose last four commit on a status page lying
/// in the second group, and that page.
fn commits_on_a_status_page_in_group_one() -> (Files, Pid) {
    let mut sim = Sim::new(1, Config::default());
    let mut status = None;
    finished(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options(256));
        insert_users(&db, 0..1, "a");
        fill_first_group(&db);
        let t = db.testing();
        let per_page = ((PAGE - HEADER_LEN) * 4) as u64;
        while t.take_xid().unwrap().0 < per_page {}
        insert_users(&db, 1..5, "a");
        status = t.commit_log().status().get(&1).copied();
        db.close().unwrap();
    });
    let status = status.unwrap();
    assert!(status.0 >= SPAN as u64, "{status:?} lies in group 1");
    (sim.files(), status)
}

fn assert_committed_rows_never_read_as_aborted(files: &Files, what: &str) {
    damaged_run(files, what, |db| {
        assert_quarantined(&text(db, SELECT), what);
        assert_eq!(
            BTreeSet::from([Quarantine::Database]),
            db.testing().probe().marks(),
            "{what}"
        );
    });
}

#[test]
fn a_blank_status_page_refuses_and_never_aborts_committed_rows() {
    adtb_heap_cap::raise_cap(3 << 30);
    let (files, status) = commits_on_a_status_page_in_group_one();
    let mut zeroed = files.clone();
    DATA.zero(&mut zeroed, status);
    assert_committed_rows_never_read_as_aborted(&zeroed, "zeroed status");
    let mut cut = files;
    DATA.truncate(&mut cut, SPAN);
    assert_committed_rows_never_read_as_aborted(&cut, "cut at the group");
}

#[test]
fn a_bitmap_zeroed_in_place_refuses_and_overwrites_nothing() {
    let (files, ()) = one_group("a", |_| ());
    let mut files = files;
    DATA.zero(&mut files, Pid::FIRST_DATA);
    let after = damaged_run(&files, "zeroed map", |db| {
        assert_all_refused_as_database(db, "zeroed map");
    });
    damaged_run(&after, "zeroed map reopened", |db| {
        assert_quarantined(&text(db, INSERT_OTHER), "reopened insert");
        assert_quarantined(&text(db, SELECT), "reopened select");
    });
}

#[test]
fn every_allocator_path_refuses_a_zeroed_bitmap() {
    let (files, page) = one_group("a", first_users_page);
    let damage = Damage::Stale {
        expected: storage::lsn(DATA.page(&files, Pid::FIRST_DATA)),
        found: Lsn(0),
    };
    let mut files = files;
    DATA.zero(&mut files, Pid::FIRST_DATA);
    damaged_run(&files, "allocator paths", |db| {
        let t = db.testing();
        let lost = PageError {
            pid: Pid::FIRST_DATA,
            damage,
        };
        let refused = |got: Result<_, Failed>| match got {
            Err(Failed::Page(e)) => e,
            other => panic!("not refused: {other:?}"),
        };
        assert_eq!(lost, refused(t.plant(Plant::Free(page))), "free");
        assert_eq!(lost, t.is_free(page).unwrap_err(), "is_free");
        assert_eq!(lost, t.allocated().unwrap_err(), "allocated");
        assert_eq!(
            lost,
            refused(t.plant(Plant::Orphan {
                kind: PageKind::Catalog,
                owner: 1
            })),
            "allocate"
        );
    });
}

#[test]
fn a_zeroed_directory_root_refuses_a_first_insert_and_marks_the_heap() {
    let (files, root) = one_group("a", |db| {
        let t = db.testing();
        let heap = t.heap("app::users", "User").unwrap();
        let data = t.pages(heap).unwrap();
        let owned = t.owned(heap).unwrap();
        (
            owned.into_iter().find(|p| !data.contains(p)).unwrap(),
            t.own_mark(heap).unwrap(),
        )
    });
    let mut files = files;
    DATA.zero(&mut files, root.0);
    damaged_run(&files, "zero directory", |db| {
        assert_quarantined(&text(db, INSERT_USER), "insert");
        assert_eq!(BTreeSet::from([root.1]), db.testing().probe().marks());
    });
}

#[test]
fn a_zeroed_large_store_page_refuses_vacuum_and_marks() {
    let row = "l".repeat(3 * PAGE);
    let (files, page) = one_group(&row, |db| {
        let t = db.testing();
        t.pages(t.large("app::users", "User").unwrap()).unwrap()[0]
    });
    let mut files = files;
    DATA.zero(&mut files, page);
    damaged_writing(&files, Writes::Vacuum, "zero large page", |db| {
        assert_quarantined(&text(db, "vacuum users"), "vacuum");
        assert_eq!(
            BTreeSet::from([Quarantine::Database]),
            db.testing().probe().marks()
        );
    });
}

fn index_page(root: bool) -> (Files, (Pid, Quarantine)) {
    one_group("a", |db| {
        let t = db.testing();
        let index = t.index("app::users", "index (tag)").unwrap();
        let pages = t.reachable(index).unwrap().0;
        let pick = if root {
            t.root(index).unwrap()
        } else {
            *pages.last().unwrap()
        };
        (pick, t.own_mark(index).unwrap())
    })
}

fn assert_index_page_refused(root: bool, query: &str) {
    let (files, (page, mark)) = index_page(root);
    let mut files = files;
    DATA.zero(&mut files, page);
    damaged_writing(&files, Writes::Row, query, |db| {
        assert_quarantined(&text(db, query), query);
        assert!(db.testing().probe().marks().contains(&mark), "{query}");
    });
}

#[test]
fn a_zeroed_index_root_refuses_a_probe_and_an_insert() {
    assert_index_page_refused(true, "from u in users where u.tag == 1 select u.id");
    assert_index_page_refused(true, INSERT_USER);
}

#[test]
fn zeroed_index_leaves_refuse_an_insert_that_latches_one() {
    let mut sim = Sim::new(1, Config::default());
    let mut leaves = Vec::new();
    let mut mark = None;
    finished(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options(256));
        let rows: String = (0..900)
            .map(|i| format!("insert users {{ id: users.key({i}), name: \"r\", tag: {i} }}\n"))
            .collect();
        text(&db, &rows).unwrap();
        let t = db.testing();
        let index = t.index("app::users", "index (tag)").unwrap();
        for page in t.reachable(index).unwrap().0 {
            if storage::kind(&t.probe().page(page).unwrap()) == PageKind::IndexLeaf as u16 {
                leaves.push(page);
            }
        }
        mark = Some(t.own_mark(index).unwrap());
        db.close().unwrap();
    });
    assert!(leaves.len() > 1, "the index has an inner page");
    let mut files = sim.files();
    for &p in &leaves {
        DATA.zero(&mut files, p);
    }
    damaged_writing(&files, Writes::Row, "zero leaves", |db| {
        assert_quarantined(&text(db, INSERT_USER), "insert");
        assert!(db.testing().probe().marks().contains(&mark.unwrap()));
    });
}

#[test]
fn a_migration_over_a_zeroed_bitmap_is_refused_and_the_files_own_log_opens_for_verify_only() {
    const MORE: &str = "module more\nadd module more\nadd type T = { id: Key ts }\nadd relation ts : T key id generate sequence\n";
    let (files, ()) = one_group("a", |_| ());
    let mut files = files;
    DATA.zero(&mut files, Pid::FIRST_DATA);
    let mut sim = Sim::with_files(1, Config::default(), files);
    finished(&mut sim, |rt| {
        let got = Database::open_in(rt.clone(), FILE, &Log::new([APP, MORE]), &options(256));
        let Err(crate::OpenError::MigrationRefused { version, reason }) = got else {
            panic!("expected migration 2 refused, found {:?}", got.map(drop));
        };
        let database = Error::Fatal {
            reason: FatalReason::Quarantined {
                what: "the database".into(),
            },
        };
        assert_eq!((2, database), (version, reason));
        let db = Database::open_in(rt, FILE, &Log::new([APP]), &options(256)).unwrap();
        assert!(db.testing().probe().marks().contains(&Quarantine::Database));
        assert_quarantined(&text(&db, SELECT), "select");
        assert!(!api::printed(db.verify(None).unwrap()).is_empty());
        db.close().unwrap();
    });
}

#[test]
fn a_visibility_map_root_zeroed_refuses_vacuum_and_marks_the_heap() {
    let (files, (page, mark)) = one_group("a", |db| {
        let t = db.testing();
        let heap = t.heap("app::users", "User").unwrap();
        let owned = t.owned(heap).unwrap();
        let vm = owned
            .into_iter()
            .find(|&p| storage::kind(&t.probe().page(p).unwrap()) == PageKind::VisibilityMap as u16)
            .unwrap();
        (vm, t.own_mark(heap).unwrap())
    });
    let mut files = files;
    DATA.zero(&mut files, page);
    damaged_run(&files, "zero vm", |db| {
        assert_quarantined(&text(db, "vacuum users"), "vacuum");
        assert!(db.testing().probe().marks().contains(&mark));
    });
}

fn stored_groups(files: &Files, data: DataFile) -> [u64; 2] {
    [0, 1].map(|copy| {
        let at = HEADER_REGION - 8;
        u64::from_le_bytes(data.page(files, Pid(copy))[at..at + 8].try_into().unwrap())
    })
}

/// Returns `files` as an engine that never stored the group count would have left them.
fn before_the_group_count(files: &Files, data: DataFile) -> Files {
    let mut files = files.clone();
    for copy in 0..2 {
        let region = &mut data.page_mut(&mut files, Pid(copy))[..HEADER_REGION];
        region[HEADER_REGION - 8..].fill(0);
        region[8..12].fill(0);
        let sum = Crc32c::new()
            .update(&copy.to_le_bytes())
            .update(region)
            .finish();
        region[8..12].copy_from_slice(&sum.to_le_bytes());
    }
    files
}

#[test]
fn a_file_before_the_count_opens_healthy_and_gains_it() {
    let (files, ()) = one_group("a", |_| ());
    let old = before_the_group_count(&files, DATA);
    assert_eq!([0, 0], stored_groups(&old, DATA));
    let mut sim = Sim::with_files(1, Config::default(), old);
    finished(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options(256));
        assert_eq!(BTreeSet::new(), db.testing().probe().marks());
        assert_eq!(4, text(&db, SELECT).unwrap().len());
        text(&db, INSERT_OTHER).unwrap();
        db.close().unwrap();
    });
    assert!(stored_groups(&sim.files(), DATA).contains(&1));
}

#[test]
fn a_file_before_the_count_refuses_a_zeroed_bitmap_inside_it() {
    let (files, ()) = one_group("a", |_| ());
    let mut files = files;
    DATA.zero(&mut files, Pid::FIRST_DATA);
    let old = before_the_group_count(&files, DATA);
    damaged_run(&old, "old zero", |db| {
        assert_all_refused_as_database(db, "old zero");
    });
}

/// Zero the page `pick` chooses, and assert the open neither panics nor answers: it marks the
/// database, keeps every page, and leaves `verify`.
fn assert_open_refuses(what: &str, pick: impl FnOnce(&Database<SimRuntime>) -> Pid) {
    let (files, page) = one_group("a", pick);
    let mut files = files;
    DATA.zero(&mut files, page);
    damaged_run(&files, what, |db| {
        assert!(
            db.testing().probe().marks().contains(&Quarantine::Database),
            "{what}"
        );
        assert_quarantined(&text(db, SELECT), what);
        assert_quarantined(&text(db, INSERT_OTHER), what);
        assert!(!api::printed(db.verify(None).unwrap()).is_empty(), "{what}");
    });
}

#[test]
fn a_zeroed_commit_log_root_refuses_the_open_for_verify_only() {
    assert_open_refuses("clog root", |db| db.testing().commit_log().root());
}

#[test]
fn a_zeroed_commit_log_directory_refuses_the_open_for_verify_only() {
    assert_open_refuses("clog directory", |db| {
        db.testing().commit_log().directory()[0]
    });
}

#[test]
fn a_zeroed_counter_page_refuses_the_open() {
    assert_open_refuses("counters", |db| {
        db.testing().root(Object::Sequences).unwrap()
    });
}

#[test]
fn a_zeroed_first_status_page_refuses_the_open() {
    assert_open_refuses("status", |db| db.testing().commit_log().status()[&0]);
}

#[test]
fn a_zeroed_status_page_below_the_resolved_ids_refuses_the_open() {
    assert_open_refuses("resolved status", |db| {
        let t = db.testing();
        let per_page = ((PAGE - HEADER_LEN) * 4) as u64;
        while t.take_xid().unwrap().0 <= per_page {}
        insert_users(db, 10..11, "a");
        t.commit_log().status()[&0]
    });
}

fn small_options() -> Options {
    Options {
        page_size: SMALL_PAGE as u32,
        log_block: 512,
        segment_size: 1 << 20,
        pool_frames: 16,
        max_pages: 2 * SMALL_SPAN + 64,
        ..Options::new(FILE)
    }
}

fn pager_of(rt: SimRuntime) -> Pager<SimRuntime> {
    Pager::open(rt, &small_options()).unwrap().0
}

fn take(pager: &Pager<SimRuntime>) -> Pid {
    pager.allocate(1, PageKind::Catalog, 1).unwrap()
}

/// Returns the files of a closed pager whose first group is full, every bit naming a page inside
/// the file.
fn full_group() -> Files {
    let mut sim = Sim::new(1, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt, &small_options()).unwrap();
        pager
            .update(&[Pid::FIRST_DATA], |v| {
                v[0][HEADER_LEN..SMALL_PAGE - 1].fill(0xff);
                v[0][SMALL_PAGE - 1] = 0x7f;
            })
            .unwrap();
        assert_eq!(Pid(SMALL_SPAN - 1), take(&pager));
        pager.close();
    });
    sim.files()
}

/// Returns the files of a closed pager with a page in the second group, and that page.
fn grown() -> (Files, Pid) {
    let mut sim = Sim::with_files(1, Config::default(), full_group());
    let mut named = None;
    finished(&mut sim, |rt| {
        let pager = pager_of(rt);
        named = Some(take(&pager));
        pager.close();
    });
    let named = named.unwrap();
    assert_eq!(Pid(SMALL_SPAN + 1), named);
    (sim.files(), named)
}

fn small_cut(files: &Files, pages: u64) -> Files {
    let mut files = files.clone();
    SMALL.truncate(&mut files, pages as usize);
    files
}

/// Returns the damage a cut or zeroing of the second group's bitmap page in `files` gives it:
/// stale, at the LSN that page has in `files`.
fn lost_damage(files: &Files) -> Damage {
    Damage::Stale {
        expected: storage::lsn(SMALL.page(files, Pid(SMALL_SPAN))),
        found: Lsn(0),
    }
}

/// Assert every allocator path refuses the lost second group with `damage` and the file keeps its
/// pages but the damage list.
fn assert_second_group_refused(files: Files, named: Pid, damage: Damage, what: &str) {
    let before = files[FILE].clone();
    let mut sim = Sim::with_files(1, Config::default(), files);
    let mut list = Pid(0);
    finished(&mut sim, |rt| {
        let pager = pager_of(rt);
        list = pager.damage_list_page();
        let lost = PageError {
            pid: Pid(SMALL_SPAN),
            damage,
        };
        assert_eq!(lost, pager.is_free(named).unwrap_err(), "{what}: is_free");
        assert_eq!(lost, pager.free(named).unwrap_err(), "{what}: free");
        assert_eq!(lost, pager.allocated().unwrap_err(), "{what}: allocated");
        assert_eq!(
            lost,
            pager.allocate(1, PageKind::Catalog, 1).unwrap_err(),
            "{what}: allocate"
        );
        assert_eq!(
            Some(BTreeSet::from([Holder::Bitmap])),
            pager.holders_of(Pid(SMALL_SPAN), Phase::Opening, |_| {}),
            "{what}"
        );
        pager.close();
    });
    let after = &sim.files()[FILE];
    assert_eq!(before.len(), after.len(), "{what}: length");
    for (i, (x, y)) in before
        .chunks(SMALL_PAGE)
        .zip(after.chunks(SMALL_PAGE))
        .enumerate()
    {
        let page = Pid(i as u64);
        assert!(
            page < Pid::FIRST_DATA || page == list || x == y,
            "{what}: page {i} changed"
        );
    }
}

#[test]
fn a_cut_at_or_before_the_group_refuses_every_allocator_path() {
    let (files, named) = grown();
    let cut = lost_damage(&files);
    assert_second_group_refused(small_cut(&files, SMALL_SPAN), named, cut, "at the group");
    assert_second_group_refused(small_cut(&files, SMALL_SPAN - 100), named, cut, "before it");
}

#[test]
fn a_zeroed_second_bitmap_refuses_every_allocator_path() {
    let (mut files, named) = grown();
    let lost = lost_damage(&files);
    SMALL.zero(&mut files, Pid(SMALL_SPAN));
    assert_second_group_refused(files, named, lost, "zeroed");
}

#[test]
fn growing_into_a_group_stores_the_count_and_a_file_before_the_count_gains_it() {
    let (files, named) = grown();
    assert!(stored_groups(&files, SMALL).contains(&2));
    let old = before_the_group_count(&files, SMALL);
    assert_eq!([0, 0], stored_groups(&old, SMALL));
    let mut sim = Sim::with_files(1, Config::default(), old.clone());
    finished(&mut sim, |rt| {
        let pager = pager_of(rt);
        assert_eq!(BTreeMap::new(), Probe::of(&pager).refusals());
        assert_eq!(Ok(false), pager.is_free(named));
    });
    assert!(stored_groups(&sim.files(), SMALL).contains(&2));
    let lost = lost_damage(&old);
    let mut blank = old;
    SMALL.zero(&mut blank, Pid(SMALL_SPAN));
    assert_second_group_refused(blank, named, lost, "old file, zeroed");
}

/// Returns the files of a pager whose grant of a page in the second group is only in the log.
fn crashed_after_growing() -> (Files, Pid) {
    let mut sim = Sim::with_files(1, Config::default(), full_group());
    let mut named = None;
    finished(&mut sim, |rt| {
        let pager = pager_of(rt);
        named = Some(take(&pager));
        pager.flush_log();
    });
    (sim.files(), named.unwrap())
}

#[test]
fn recovery_stores_the_count_so_a_later_cut_never_reuses_a_named_page() {
    let (files, named) = crashed_after_growing();
    let mut sim = Sim::with_files(1, Config::default(), files);
    finished(&mut sim, |rt| drop(pager_of(rt)));
    let recovered = sim.files();
    assert!(stored_groups(&recovered, SMALL).contains(&2));
    let cut = lost_damage(&recovered);
    assert_second_group_refused(
        small_cut(&recovered, SMALL_SPAN),
        named,
        cut,
        "after recovery",
    );
}

#[test]
fn a_crash_in_recovery_then_a_cut_never_reuses_a_named_page() {
    adtb_heap_cap::raise_cap(2 << 30);
    let (files, named) = crashed_after_growing();
    let mut sim = Sim::with_files(1, Config::default(), files);
    sim.record();
    finished(&mut sim, |rt| drop(pager_of(rt)));
    let recording = sim.recording();
    for at in 0..=recording.len() {
        for image in recording.images(at, 2, at as u64) {
            let what = format!("crash at {at}");
            let mut sim = Sim::with_files(1, Config::default(), small_cut(&image, SMALL_SPAN));
            finished(&mut sim, |rt| {
                let pager = pager_of(rt);
                assert_ne!(Ok(true), pager.is_free(named), "{what}");
                if let Ok(pid) = pager.allocate(1, PageKind::Catalog, 1) {
                    assert_ne!(named, pid, "{what}");
                }
            });
        }
    }
}

/// Allocate into the second group, free, flush and checkpoint in the orders a crash can split.
fn cross_a_group(pager: &Pager<SimRuntime>) {
    let first = take(pager);
    pager.flush_log();
    let second = take(pager);
    pager.checkpoint();
    pager.flush_pages(usize::MAX);
    pager.free(first).unwrap();
    pager.checkpoint();
    pager.free(second).unwrap();
    take(pager);
    pager.flush_log();
    pager.flush_pages(1);
    pager.close();
}

#[test]
fn no_crash_while_crossing_a_group_refuses_a_healthy_file() {
    adtb_heap_cap::raise_cap(2 << 30);
    for seed in sweep(0..1) {
        let mut sim = Sim::with_files(seed, Config::default(), full_group());
        sim.record();
        finished(&mut sim, |rt| cross_a_group(&pager_of(rt)));
        let recording = sim.recording();
        for at in 0..=recording.len() {
            for image in recording.images(at, 4, seed + at as u64) {
                let what = format!("seed {seed} crash at {at}");
                let mut sim = Sim::with_files(seed, Config::default(), image);
                finished(&mut sim, |rt| {
                    let pager = pager_of(rt);
                    assert_eq!(BTreeMap::new(), Probe::of(&pager).refusals(), "{what}");
                    assert!(pager.allocated().is_ok(), "{what}");
                    assert!(pager.allocate(1, PageKind::Catalog, 1).is_ok(), "{what}");
                    assert_eq!(BTreeSet::new(), Probe::of(&pager).marks(), "{what}");
                    pager.close();
                });
            }
        }
    }
}

#[test]
fn a_page_vacuum_frees_and_another_heap_reuses_under_a_reader_is_not_refused() {
    let mut sim = Sim::new(1, Config::default());
    finished(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options(256));
        insert_users(&db, 0..4, "u");
        let t = db.testing();
        let heap = t.heap("app::users", "User").unwrap();
        let index = t.index("app::users", "key id").unwrap();
        let listing = t.pages(heap).unwrap();
        let key = [Value::I64(1)];
        let held = t.entries(index, &key).unwrap();
        assert_eq!(1, held.len());
        api::results(&db, "delete u in users");
        api::results(&db, "vacuum users");
        let others = || t.pages(t.heap("app::others", "Other").unwrap()).unwrap();
        for i in 0..64 {
            if others().contains(&listing[0]) {
                break;
            }
            api::results(
                &db,
                &format!(r#"insert others {{ id: others.key({i}), name: "o" }}"#),
            );
        }
        assert!(
            others().contains(&listing[0]),
            "others reuses {:?}",
            listing[0]
        );
        assert_eq!(0, t.versions(heap, Some(&listing)).unwrap().len());
        t.refuse_named(heap, index, &held[0]).expect("not refused");
        assert_eq!(BTreeSet::new(), t.probe().marks());
        assert_eq!(Vec::<String>::new(), text(&db, SELECT).unwrap());
    });
}

/// Returns what `query` answers on `db`, panicking on anything but rows.
fn rows_or_panic(db: &Database<SimRuntime>, query: &str, seed: u64) -> Vec<String> {
    text(db, query).unwrap_or_else(|e| panic!("seed {seed}: {query}: {e:?}"))
}

#[test]
fn lookups_racing_a_vacuum_that_frees_slots_of_a_page_it_keeps_are_not_refused() {
    let seeds = sweep(0..8);
    let mut ran = 0;
    for seed in seeds.clone() {
        let config = Config {
            switch_rate: 0.3,
            ..Config::default()
        };
        let mut sim = Sim::new(seed, config);
        finished(&mut sim, |rt| {
            let db = open::opened(&rt, FILE, &Log::new([APP]), &options(256));
            insert_users(&db, 0..4, "kept");
            let page = first_users_page(&db);
            let reader = {
                let db = db.clone();
                rt.spawn("reader", move || {
                    for k in 0..12 {
                        let by_key = format!(
                            "from u in users where u.id == users.key({}) select u.id",
                            100 + k / 2
                        );
                        assert!(rows_or_panic(&db, &by_key, seed).len() <= 1);
                        let by_tag = "from u in users where u.tag == 2 select u.id";
                        assert!(rows_or_panic(&db, by_tag, seed).len() <= 1);
                        api::results(
                            &db,
                            &format!("insert others {{ id: others.key({k}), name: \"o\" }}"),
                        );
                    }
                })
            };
            for k in 0..6 {
                let row = 100 + k;
                api::results(
                    &db,
                    &format!("insert users {{ id: users.key({row}), name: \"g\", tag: 2 }}"),
                );
                api::results(
                    &db,
                    &format!("delete u in users where key u == users.key({row})"),
                );
                api::results(&db, "vacuum users");
            }
            rt.join(reader);
            assert_eq!(
                page,
                first_users_page(&db),
                "seed {seed}: the page survives"
            );
            assert_eq!(BTreeSet::new(), db.testing().probe().marks(), "seed {seed}");
            ran += 1;
        });
    }
    assert_eq!(seeds.count(), ran);
}
