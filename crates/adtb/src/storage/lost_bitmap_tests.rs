//! A bitmap page lost or zeroed is damage and never free space. Each damaged file refuses the call
//! and marks, and leaves named pages as they were. A healthy file, cut by no fault, never refuses.

use crate::testing::{Knobs, Object, Plant, Probe};
use adtb_testkit::process::finished;
use std::collections::{BTreeMap, BTreeSet};

use super::open_damage_tests::{NOT_KEPT, UNREAD, places};
use crate::key::Value;
use crate::runtime::Threads;
use crate::storage::policy::{Holder, Phase};
use crate::storage::{
    self, Crc32c, Damage, HEADER_LEN, Lsn, Options, PageError, PageKind, Pager, Pid, Quarantine,
    Shape,
};
use crate::test_world::api;
use crate::test_world::load as open;
use crate::{CallOptions, Database, Error, FatalReason, Log, OpenOptions};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_testkit::damage::{DataFile, Files};
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
        let database = Error::Fatal {
            reason: FatalReason::Quarantined {
                what: "the database".into(),
            },
        };
        assert_eq!(Err(database), text(db, SELECT), "{what}");
        assert_eq!(
            BTreeSet::from([Quarantine::Database]),
            db.testing().probe().marks(),
            "{what}"
        );
    });
}

#[test]
fn a_blank_or_cut_status_page_of_committed_rows_refuses_the_select_and_marks_the_database() {
    adtb_heap_cap::raise_cap(3 << 30);
    let (files, status) = commits_on_a_status_page_in_group_one();
    let mut zeroed = files.clone();
    DATA.zero(&mut zeroed, status);
    assert_committed_rows_never_read_as_aborted(&zeroed, "zeroed status");
    let mut cut = files;
    DATA.truncate(&mut cut, SPAN);
    assert_committed_rows_never_read_as_aborted(&cut, "cut at the group");
}

/// A role of zeroed pages, the pages, what the calls that meet them may write, the calls, and the
/// mark they leave.
type Zeroed = (
    &'static str,
    Vec<Pid>,
    Writes,
    &'static [&'static str],
    Quarantine,
);

const OPEN_CALLS: &[&str] = &[SELECT, INSERT_OTHER];

/// Returns a file with an index of 900 entries and its rows: the index root under a probe and
/// under an insert, every index leaf under an insert that latches one, the visibility map root
/// under vacuum, and the commit log root, commit log directory, counter page and first status page
/// under an open.
fn zeroed_index() -> (Files, Vec<Zeroed>) {
    use Writes::{Open, Row};
    const PROBE: &[&str] = &["from u in users where u.tag == 1 select u.id"];
    let mut sim = Sim::new(1, Config::default());
    let mut rows = Vec::new();
    finished(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options(256));
        let inserts: String = (0..900)
            .map(|i| format!("insert users {{ id: users.key({i}), name: \"r\", tag: {i} }}\n"))
            .collect();
        text(&db, &inserts).unwrap();
        let t = db.testing();
        let index = t.index("app::users", "index (tag)").unwrap();
        let heap = t.heap("app::users", "User").unwrap();
        let of = |kind: PageKind| {
            move |p: &Pid| storage::kind(&t.probe().page(*p).unwrap()) == kind as u16
        };
        let leaves = t
            .reachable(index)
            .unwrap()
            .0
            .into_iter()
            .filter(of(PageKind::IndexLeaf));
        let leaves: Vec<Pid> = leaves.collect();
        assert!(leaves.len() > 1, "the index has an inner page");
        let vm = t
            .owned(heap)
            .unwrap()
            .into_iter()
            .filter(of(PageKind::VisibilityMap));
        let (root, mark) = (vec![t.root(index).unwrap()], t.own_mark(index).unwrap());
        let insert: &[&str] = &[INSERT_USER];
        rows.push(("index root", root.clone(), Row, PROBE, mark));
        rows.push(("index root", root, Row, insert, mark));
        rows.push(("index leaves", leaves, Row, insert, mark));
        let heap_mark = t.own_mark(heap).unwrap();
        rows.push(("vm root", vm.collect(), Open, &["vacuum users"], heap_mark));
        let clog = t.commit_log();
        for (role, page) in [
            ("clog root", clog.root()),
            ("clog directory", clog.directory()[0]),
            ("counters", t.root(Object::Sequences).unwrap()),
            ("status", clog.status()[&0]),
        ] {
            rows.push((role, vec![page], Open, OPEN_CALLS, Quarantine::Database));
        }
        db.close().unwrap();
    });
    (sim.files(), rows)
}

/// Returns a one group file and the row of its first status page once the ids it holds are
/// resolved, under an open.
fn zeroed_resolved_status() -> (Files, Vec<Zeroed>) {
    one_group("a", |db| {
        let t = db.testing();
        let per_page = ((PAGE - HEADER_LEN) * 4) as u64;
        while t.take_xid().unwrap().0 <= per_page {}
        insert_users(db, 10..11, "a");
        let (page, database) = (vec![t.commit_log().status()[&0]], Quarantine::Database);
        vec![("resolved status", page, Writes::Open, OPEN_CALLS, database)]
    })
}

/// Space: a zeroed index root under a probe and an insert, zeroed index leaves under an insert
/// that latches one, a zeroed visibility map root under vacuum, and a zeroed commit log root,
/// commit log directory, counter page, first status page and status page below the resolved ids
/// under an open and a select and an insert.
#[test]
fn a_zeroed_page_refuses_each_call_that_meets_it_and_marks_only_its_holder() {
    let (mut got, mut want) = (Vec::new(), Vec::new());
    for (files, rows) in [zeroed_index(), zeroed_resolved_status()] {
        for (role, pages, writes, calls, mark) in rows {
            let mut zeroed = files.clone();
            pages.iter().for_each(|&p| DATA.zero(&mut zeroed, p));
            let whole = mark == Quarantine::Database;
            damaged_writing(&zeroed, writes, role, |db| {
                let refused: Vec<_> = calls.iter().map(|c| text(db, c).map(drop)).collect();
                let verify = whole.then(|| places(&api::sorted(db.verify(None).unwrap())));
                got.push((role, refused, db.testing().probe().marks(), verify));
            });
            let what = match mark {
                Quarantine::Database => "the database",
                Quarantine::Index(_) => "app::users index (tag)",
                Quarantine::Heap(_) => "app::users.User",
            };
            let refused = Err(Error::Fatal {
                reason: FatalReason::Quarantined { what: what.into() },
            });
            let verify = whole.then(|| {
                let (expected, found) = (storage::lsn(DATA.page(&files, pages[0])), Lsn(0));
                let stale = format!(
                    "page {}: Stale (holding LSN {} where the log expects {})",
                    pages[0].0, found.0, expected.0
                );
                let rows = [
                    ("StalePage", stale.as_str()),
                    ("Structure", UNREAD),
                    ("Structure", NOT_KEPT),
                ];
                rows.map(|(c, a)| (c.to_owned(), a.to_owned())).to_vec()
            });
            let marks = BTreeSet::from([mark]);
            want.push((role, vec![refused; calls.len()], marks, verify));
        }
    }
    assert_eq!(9, got.len());
    assert_eq!(want, got);
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

/// Assert an allocation and a free refuse the lost second group with `damage` and the file keeps
/// its pages but the damage list.
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
        assert_eq!(
            lost,
            pager.allocate(1, PageKind::Catalog, 1).unwrap_err(),
            "{what}: allocate"
        );
        assert_eq!(lost, pager.free(named).unwrap_err(), "{what}: free");
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

type Call = fn(&Pager<SimRuntime>, Pid) -> Result<(), PageError>;

#[test]
fn the_bitmap_gate_refuses_a_bitmap_page_of_another_kind_under_either_latch() {
    let (files, named) = grown();
    let map = Pid(SMALL_SPAN);
    let calls: [(&str, Call); 2] = [
        ("allocate", |p, _| {
            p.allocate(1, PageKind::Catalog, 1).map(drop)
        }),
        ("free", |p, named| p.free(named).map(drop)),
    ];
    for (what, call) in calls {
        let mut sim = Sim::with_files(1, Config::default(), files.clone());
        finished(&mut sim, |rt| {
            let pager = pager_of(rt);
            pager
                .update(&[map], |v| {
                    v[0][storage::FIRST_EDITABLE..HEADER_LEN]
                        .copy_from_slice(&storage::header_edit(1, PageKind::Catalog, 1, 0));
                })
                .unwrap();
            let refused = PageError {
                pid: map,
                damage: Damage::Shape(Shape::Kind),
            };
            assert_eq!(Err(refused), call(&pager, named), "{what}");
        });
    }
}

/// How the second group's bitmap page is lost.
#[derive(Clone, Copy, Debug)]
enum Lost {
    /// Cut away after an open.
    Cut,
    /// Zeroed with no open between.
    ZeroedUnopened,
}

/// Space: a page granted in the second group by a closed session, or only in the log and recovered,
/// with the header's group count erased as an engine before the count left it; an open of each,
/// then the second group's bitmap page cut away, and for the closed session zeroed with no open
/// between.
#[test]
fn an_open_stores_the_group_count_so_a_lost_group_is_refused_and_never_reused() {
    let mut cells = Vec::new();
    for (source, made) in [
        ("grown", grown as fn() -> _),
        ("recovered", crashed_after_growing),
    ] {
        let (base, named) = made();
        let files = before_the_group_count(&base, SMALL);
        assert_eq!([0, 0], stored_groups(&files, SMALL), "{source}");
        let mut sim = Sim::with_files(1, Config::default(), files.clone());
        finished(&mut sim, |rt| {
            let pager = pager_of(rt);
            assert_eq!(BTreeMap::new(), Probe::of(&pager).refusals());
            assert_eq!(Ok(false), Probe::of(&pager).is_free(named));
            pager.close();
        });
        let opened = sim.files();
        assert!(stored_groups(&opened, SMALL).contains(&2), "{source}");
        for damage in [Lost::Cut, Lost::ZeroedUnopened] {
            let from = match damage {
                Lost::ZeroedUnopened if source == "grown" => &files,
                Lost::ZeroedUnopened => continue,
                Lost::Cut => &opened,
            };
            let mut damaged = from.clone();
            match damage {
                Lost::Cut => SMALL.truncate(&mut damaged, SMALL_SPAN as usize),
                Lost::ZeroedUnopened => SMALL.zero(&mut damaged, Pid(SMALL_SPAN)),
            }
            let what = format!("{source}, {damage:?}");
            assert_second_group_refused(damaged, named, lost_damage(from), &what);
            cells.push(what);
        }
    }
    assert_eq!(3, cells.len(), "{cells:?}");
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
                assert_ne!(Ok(true), Probe::of(&pager).is_free(named), "{what}");
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
                    let before = pager
                        .allocated()
                        .unwrap_or_else(|e| panic!("{what}: {e:?}"));
                    let pid = pager
                        .allocate(1, PageKind::Catalog, 1)
                        .unwrap_or_else(|e| panic!("{what}: {e:?}"));
                    assert!(!before.contains(&pid), "{what}: {pid:?} was allocated");
                    let mut after = before;
                    after.push(pid);
                    after.sort_unstable();
                    assert_eq!(Ok(after), pager.allocated(), "{what}");
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
