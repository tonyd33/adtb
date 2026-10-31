//! A bitmap page lost or zeroed is damage and never free space. Each damaged file refuses the call
//! and marks, and leaves named pages as they were. A healthy file, cut by no fault, never refuses.

use std::collections::{BTreeMap, BTreeSet};

use adtb::storage::{self, Crc32c, HEADER_LEN, Options, PageKind, Pager, Pid, Quarantine};
use adtb::{CallOptions, Database, Error, FatalReason, Log, OpenOptions};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_simkit::api;
use adtb_simkit::harness::finished;
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
const ROOTS_HEAP: u32 = 2;

type Files = BTreeMap<String, Vec<u8>>;

fn options(frames: u64) -> OpenOptions {
    OpenOptions {
        pool_bytes: Some(frames * PAGE as u64),
        ..api::options(2)
    }
}

fn open(rt: SimRuntime) -> Database<SimRuntime> {
    Database::open_in(rt, FILE, &Log::new([APP]), &options(256)).unwrap()
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
    db.write_path()
        .pager()
        .update(&[Pid::FIRST_DATA], |v| v[0][HEADER_LEN..].fill(0xff))
        .unwrap();
}

fn first_users_page(db: &Database<SimRuntime>) -> Pid {
    let w = db.write_path();
    w.heap_named("app::users", "User")
        .unwrap()
        .pages(w.pager())
        .unwrap()[0]
}

/// Returns the files of a database whose first group is full and whose users live in the second.
fn two_groups(row: &str) -> Files {
    let mut sim = Sim::new(1, Config::default());
    finished(&mut sim, |rt| {
        let db = open(rt);
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
        let db = open(rt);
        insert_users(&db, 0..4, row);
        picked = Some(pick(&db));
        db.close().unwrap();
    });
    (sim.files(), picked.unwrap())
}

fn cut(files: &Files, pages: usize) -> Files {
    let mut files = files.clone();
    files.get_mut(FILE).unwrap().truncate(pages * PAGE);
    files
}

fn zeroed(files: &Files, page: Pid) -> Files {
    let mut files = files.clone();
    let at = page.0 as usize * PAGE;
    files.get_mut(FILE).unwrap()[at..at + PAGE].fill(0);
    files
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
    users: u32,
    counters: Pid,
    writes: Writes,
}

impl Kept {
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
    /// the marks page and the pages `may_change` names.
    fn check(&self, before: &Files, after: &Files, what: &str) {
        let (a, b) = (&before[FILE], &after[FILE]);
        assert!(b.len() >= a.len(), "{what}: the file shrank");
        for (i, (x, y)) in a.chunks(PAGE).zip(b.chunks(PAGE)).enumerate() {
            let page = Pid(i as u64);
            if page < Pid::FIRST_DATA || page == self.marks {
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
        let db = open(rt);
        let w = db.write_path();
        kept = Some(Kept {
            marks: w.pager().quarantine_page(),
            users: w.heap_named("app::users", "User").map_or(0, |h| h.id().0),
            counters: w.sequences_root(),
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
        db.write_path().pager().quarantined(),
        "{what}"
    );
    assert_quarantined(&text(db, SELECT), what);
}

#[test]
fn a_cut_losing_a_group_with_large_values_refuses_marks_and_stays_marked() {
    let row = "l".repeat(3 * PAGE);
    let files = cut(&two_groups(&row), SPAN - 100);
    let after = damaged_run(&files, "lost group", |db| {
        assert_quarantined(&text(db, INSERT_USER), "large insert");
        assert_quarantined(&text(db, INSERT_USER), "retry");
        assert_quarantined(&text(db, INSERT_OTHER), "other relation");
        let marks = db.write_path().pager().quarantined();
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
        let db = open(rt);
        insert_users(&db, 0..1, "a");
        fill_first_group(&db);
        let w = db.write_path();
        let per_page = ((PAGE - HEADER_LEN) * 4) as u64;
        while w
            .txns()
            .begin(w.pager())
            .map(|x| {
                w.txns().abort(w.pager(), x).unwrap();
                x.0
            })
            .unwrap()
            < per_page
        {}
        insert_users(&db, 1..5, "a");
        let log = w.txns().commit_log();
        status = log.status_pages().get(&1).copied();
        drop(log);
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
            db.write_path().pager().quarantined(),
            "{what}"
        );
    });
}

#[test]
fn a_blank_status_page_refuses_and_never_aborts_committed_rows() {
    let (files, status) = commits_on_a_status_page_in_group_one();
    assert_committed_rows_never_read_as_aborted(&zeroed(&files, status), "zeroed status");
    assert_committed_rows_never_read_as_aborted(&cut(&files, SPAN), "cut at the group");
}

#[test]
fn a_bitmap_zeroed_in_place_refuses_and_overwrites_nothing() {
    let (files, ()) = one_group("a", |_| ());
    let files = zeroed(&files, Pid::FIRST_DATA);
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
    let files = zeroed(&files, Pid::FIRST_DATA);
    damaged_run(&files, "allocator paths", |db| {
        let w = db.write_path();
        let p = w.pager();
        assert!(p.free(page).is_err(), "free");
        assert!(p.is_free(page).is_err(), "is_free");
        assert!(p.allocated().is_err(), "allocated");
        assert!(p.allocate(1, PageKind::Catalog, 1).is_err(), "allocate");
        assert!(
            p.reserve().page(1, PageKind::Catalog, 1).is_err(),
            "reserve"
        );
    });
}

#[test]
fn a_zeroed_directory_root_refuses_a_first_insert_and_marks_the_heap() {
    let (files, root) = one_group("a", |db| {
        let w = db.write_path();
        let heap = w.heap_named("app::users", "User").unwrap();
        let data = heap.pages(w.pager()).unwrap();
        let owned = heap.owned_pages(w.pager()).unwrap();
        (
            owned.into_iter().find(|p| !data.contains(p)).unwrap(),
            Quarantine::Heap(heap.id().0),
        )
    });
    let files = zeroed(&files, root.0);
    damaged_run(&files, "zero directory", |db| {
        assert_quarantined(&text(db, INSERT_USER), "insert");
        assert_eq!(
            BTreeSet::from([root.1]),
            db.write_path().pager().quarantined()
        );
    });
}

#[test]
fn a_zeroed_large_store_page_refuses_vacuum_and_marks() {
    let row = "l".repeat(3 * PAGE);
    let (files, page) = one_group(&row, |db| {
        let w = db.write_path();
        w.heap_named("app::users", "User")
            .unwrap()
            .large()
            .pages(w.pager())
            .unwrap()[0]
    });
    let files = zeroed(&files, page);
    damaged_writing(&files, Writes::Vacuum, "zero large page", |db| {
        assert_quarantined(&text(db, "vacuum users"), "vacuum");
        assert_eq!(
            BTreeSet::from([Quarantine::Database]),
            db.write_path().pager().quarantined()
        );
    });
}

fn index_page(root: bool) -> (Files, (Pid, Quarantine)) {
    one_group("a", |db| {
        let w = db.write_path();
        let tree = w.index_named("app::users", "index (tag)").unwrap().tree();
        let pages = tree.reachable(w.pager()).0;
        let pick = if root {
            tree.root()
        } else {
            *pages.last().unwrap()
        };
        (pick, Quarantine::Index(tree.owner()))
    })
}

fn assert_index_page_refused(root: bool, query: &str) {
    let (files, (page, mark)) = index_page(root);
    let files = zeroed(&files, page);
    damaged_writing(&files, Writes::Row, query, |db| {
        assert_quarantined(&text(db, query), query);
        assert!(
            db.write_path().pager().quarantined().contains(&mark),
            "{query}"
        );
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
        let db = open(rt);
        let rows: String = (0..900)
            .map(|i| format!("insert users {{ id: users.key({i}), name: \"r\", tag: {i} }}\n"))
            .collect();
        text(&db, &rows).unwrap();
        let w = db.write_path();
        let tree = w.index_named("app::users", "index (tag)").unwrap().tree();
        for page in tree.reachable(w.pager()).0 {
            if storage::kind(&w.pager().read(page).unwrap()) == PageKind::IndexLeaf as u16 {
                leaves.push(page);
            }
        }
        mark = Some(Quarantine::Index(tree.owner()));
        db.close().unwrap();
    });
    assert!(leaves.len() > 1, "the index has an inner page");
    let files = leaves.iter().fold(sim.files(), |f, &p| zeroed(&f, p));
    damaged_writing(&files, Writes::Row, "zero leaves", |db| {
        assert_quarantined(&text(db, INSERT_USER), "insert");
        assert!(
            db.write_path()
                .pager()
                .quarantined()
                .contains(&mark.unwrap())
        );
    });
}

#[test]
fn a_migration_over_a_zeroed_bitmap_opens_for_verify_only() {
    const MORE: &str = "module more\nadd module more\nadd type T = { id: Key ts }\nadd relation ts : T key id generate sequence\n";
    let (files, ()) = one_group("a", |_| ());
    let mut sim = Sim::with_files(1, Config::default(), zeroed(&files, Pid::FIRST_DATA));
    finished(&mut sim, |rt| {
        let db = Database::open_in(rt, FILE, &Log::new([APP, MORE]), &options(256)).unwrap();
        assert!(
            db.write_path()
                .pager()
                .quarantined()
                .contains(&Quarantine::Database)
        );
        assert_quarantined(&text(&db, SELECT), "select");
        assert!(!api::printed(db.verify(None).unwrap()).is_empty());
        db.close().unwrap();
    });
}

#[test]
fn a_visibility_map_root_zeroed_refuses_vacuum_and_marks_the_heap() {
    let (files, (page, mark)) = one_group("a", |db| {
        let w = db.write_path();
        let heap = w.heap_named("app::users", "User").unwrap();
        let owned = heap.owned_pages(w.pager()).unwrap();
        let vm = owned
            .into_iter()
            .find(|&p| storage::kind(&w.pager().read(p).unwrap()) == PageKind::VisibilityMap as u16)
            .unwrap();
        (vm, Quarantine::Heap(heap.id().0))
    });
    let files = zeroed(&files, page);
    damaged_run(&files, "zero vm", |db| {
        assert_quarantined(&text(db, "vacuum users"), "vacuum");
        assert!(db.write_path().pager().quarantined().contains(&mark));
    });
}

fn stored_groups(files: &Files, page: usize) -> [u64; 2] {
    [0, 1].map(|copy| {
        let at = copy * page + HEADER_REGION - 8;
        u64::from_le_bytes(files[FILE][at..at + 8].try_into().unwrap())
    })
}

/// Returns `files` as an engine that never stored the group count would have left them.
fn before_the_group_count(files: &Files, page: usize) -> Files {
    let mut files = files.clone();
    let file = files.get_mut(FILE).unwrap();
    for copy in 0..2 {
        let region = &mut file[copy * page..copy * page + HEADER_REGION];
        region[HEADER_REGION - 8..].fill(0);
        region[8..12].fill(0);
        let sum = Crc32c::new()
            .update(&(copy as u64).to_le_bytes())
            .update(region)
            .finish();
        region[8..12].copy_from_slice(&sum.to_le_bytes());
    }
    files
}

#[test]
fn a_file_before_the_count_opens_healthy_and_gains_it() {
    let (files, ()) = one_group("a", |_| ());
    let old = before_the_group_count(&files, PAGE);
    assert_eq!([0, 0], stored_groups(&old, PAGE));
    let mut sim = Sim::with_files(1, Config::default(), old);
    finished(&mut sim, |rt| {
        let db = open(rt);
        assert_eq!(BTreeSet::new(), db.write_path().pager().quarantined());
        assert_eq!(4, text(&db, SELECT).unwrap().len());
        text(&db, INSERT_OTHER).unwrap();
        db.close().unwrap();
    });
    assert!(stored_groups(&sim.files(), PAGE).contains(&1));
}

#[test]
fn a_file_before_the_count_refuses_a_zeroed_bitmap_inside_it() {
    let (files, ()) = one_group("a", |_| ());
    let old = before_the_group_count(&zeroed(&files, Pid::FIRST_DATA), PAGE);
    damaged_run(&old, "old zero", |db| {
        assert_all_refused_as_database(db, "old zero");
    });
}

/// Zero the page `pick` chooses, and assert the open neither panics nor answers: it marks the
/// database, keeps every page, and leaves `verify`.
fn assert_open_refuses(what: &str, pick: impl FnOnce(&Database<SimRuntime>) -> Pid) {
    let (files, page) = one_group("a", pick);
    let files = zeroed(&files, page);
    damaged_run(&files, what, |db| {
        assert!(
            db.write_path()
                .pager()
                .quarantined()
                .contains(&Quarantine::Database),
            "{what}"
        );
        assert_quarantined(&text(db, SELECT), what);
        assert_quarantined(&text(db, INSERT_OTHER), what);
        assert!(!api::printed(db.verify(None).unwrap()).is_empty(), "{what}");
    });
}

#[test]
fn a_zeroed_commit_log_root_refuses_the_open_for_verify_only() {
    assert_open_refuses("clog root", |db| db.write_path().txns().commit_log().root());
}

#[test]
fn a_zeroed_commit_log_directory_refuses_the_open_for_verify_only() {
    assert_open_refuses("clog directory", |db| {
        db.write_path().txns().commit_log().directory_pages()[0]
    });
}

#[test]
fn a_zeroed_counter_page_refuses_the_open() {
    assert_open_refuses("counters", |db| db.write_path().sequences_root());
}

#[test]
fn a_zeroed_first_status_page_refuses_the_open() {
    assert_open_refuses("status", |db| {
        db.write_path().txns().commit_log().status_pages()[&0]
    });
}

#[test]
fn a_zeroed_status_page_below_the_resolved_ids_refuses_the_open() {
    assert_open_refuses("resolved status", |db| {
        let w = db.write_path();
        let per_page = ((PAGE - HEADER_LEN) * 4) as u64;
        loop {
            let x = w.txns().begin(w.pager()).unwrap();
            w.txns().abort(w.pager(), x).unwrap();
            if x.0 > per_page {
                break;
            }
        }
        insert_users(db, 10..11, "a");
        w.txns().commit_log().status_pages()[&0]
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
    files
        .get_mut(FILE)
        .unwrap()
        .truncate(pages as usize * SMALL_PAGE);
    files
}

/// Assert every allocator path refuses the lost second group and the file keeps its pages.
fn assert_second_group_refused(files: Files, named: Pid, what: &str) {
    let before = files[FILE].clone();
    let mut sim = Sim::with_files(1, Config::default(), files);
    finished(&mut sim, |rt| {
        let pager = pager_of(rt);
        assert!(pager.is_free(named).is_err(), "{what}: is_free");
        assert!(pager.free(named).is_err(), "{what}: free");
        assert!(pager.allocated().is_err(), "{what}: allocated");
        let got = pager.allocate(1, PageKind::Catalog, 1);
        assert!(got.is_err(), "{what}: allocate {got:?}");
        assert_eq!(
            BTreeSet::from([Quarantine::Database]),
            pager.holders_of(Pid(SMALL_SPAN), None, |_| {}),
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
        assert!(page < Pid::FIRST_DATA || x == y, "{what}: page {i} changed");
    }
}

#[test]
fn a_cut_at_or_before_the_group_refuses_every_allocator_path() {
    let (files, named) = grown();
    assert_second_group_refused(small_cut(&files, SMALL_SPAN), named, "at the group");
    assert_second_group_refused(small_cut(&files, SMALL_SPAN - 100), named, "before it");
}

#[test]
fn a_zeroed_second_bitmap_refuses_every_allocator_path() {
    let (mut files, named) = grown();
    let at = SMALL_SPAN as usize * SMALL_PAGE;
    files.get_mut(FILE).unwrap()[at..at + SMALL_PAGE].fill(0);
    assert_second_group_refused(files, named, "zeroed");
}

#[test]
fn growing_into_a_group_stores_the_count_and_a_file_before_the_count_gains_it() {
    let (files, named) = grown();
    assert!(stored_groups(&files, SMALL_PAGE).contains(&2));
    let old = before_the_group_count(&files, SMALL_PAGE);
    assert_eq!([0, 0], stored_groups(&old, SMALL_PAGE));
    let mut sim = Sim::with_files(1, Config::default(), old.clone());
    finished(&mut sim, |rt| {
        let pager = pager_of(rt);
        assert_eq!(BTreeMap::new(), pager.damaged());
        assert_eq!(Ok(false), pager.is_free(named));
    });
    assert!(stored_groups(&sim.files(), SMALL_PAGE).contains(&2));
    let mut blank = old;
    let at = SMALL_SPAN as usize * SMALL_PAGE;
    blank.get_mut(FILE).unwrap()[at..at + SMALL_PAGE].fill(0);
    assert_second_group_refused(blank, named, "old file, zeroed");
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
    assert!(stored_groups(&recovered, SMALL_PAGE).contains(&2));
    assert_second_group_refused(small_cut(&recovered, SMALL_SPAN), named, "after recovery");
}

#[test]
fn a_crash_in_recovery_then_a_cut_never_reuses_a_named_page() {
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
                    assert_eq!(BTreeMap::new(), pager.damaged(), "{what}");
                    assert!(pager.allocated().is_ok(), "{what}");
                    assert!(pager.allocate(1, PageKind::Catalog, 1).is_ok(), "{what}");
                    assert_eq!(BTreeSet::new(), pager.quarantined(), "{what}");
                    pager.close();
                });
            }
        }
    }
}
