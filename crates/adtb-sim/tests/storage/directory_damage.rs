//! A directory page, or a page a directory names, that is not of its kind or its space's owner is
//! damage. Every call meeting it answers `Quarantined` in bounded time and marks the holder, a
//! read that meets no such page answers exactly, other relations of a heap's damage read exactly,
//! and the file reopens.

use adtb::testing::{PageKind, Pid, Plant, Quarantine, page, page::FIRST_EDITABLE};
use adtb_harness::process::finished;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use adtb::{CallOptions, Database, Error, FatalReason, Log};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_simkit::api;
use adtb_simkit::open;
use adtb_testkit::profile::{assert_reached, every_case};

const FILE: &str = "directory_damage.adt";
const PAGE: usize = 8192;
const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text, tag: I64 }
add relation users : User key id generate sequence
  index (tag)
add type Other = { id: Key others, name: Text }
add relation others : Other key id generate sequence
";
const OTHERS: &str = "from o in others select o.name";
const KIND: usize = 14;
const BOUND: Duration = Duration::from_secs(5);

type Files = BTreeMap<String, Vec<u8>>;

fn text(db: &Database<SimRuntime>, query: &str) -> Result<Vec<String>, Error> {
    db.query_text(query, &[], &CallOptions::default())
        .map(|mut r| r.pop().map(api::printed).unwrap_or_default())
}

fn large() -> String {
    "l".repeat(3 * PAGE)
}

fn insert(id: u32, name: &str) -> String {
    format!(r#"insert users {{ id: users.key({id}), name: "{name}", tag: {id} }}"#)
}

/// Returns the files of a database whose users hold a row with a small and a row with a large
/// value, and whose others hold one row.
fn fixture() -> Files {
    let mut sim = Sim::new(1, Config::default());
    finished(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &open::options(2));
        api::results(&db, &insert(1, "a"));
        api::results(&db, &insert(2, &large()));
        api::results(&db, r#"insert others { id: others.key(1), name: "o" }"#);
        db.close().unwrap();
    });
    sim.files()
}

/// Run `body` on an open of `files` and returns the files it leaves, failing when it has not
/// returned within `BOUND` of real time.
fn bounded(
    files: &Files,
    what: String,
    body: impl FnOnce(&Database<SimRuntime>) + Send + 'static,
) -> Files {
    let files = files.clone();
    let (done, answer) = mpsc::channel();
    #[expect(
        clippy::disallowed_methods,
        reason = "gone once the simulator stops a run that spins, so no case needs its own thread"
    )]
    std::thread::spawn(move || {
        let mut sim = Sim::with_files(1, Config::default(), files);
        finished(&mut sim, |rt| {
            let db = open::opened(&rt, FILE, &Log::new([APP]), &open::options(2));
            body(&db);
            db.close().unwrap();
        });
        done.send(sim.files()).unwrap();
    });
    match answer.recv_timeout(BOUND) {
        Ok(files) => files,
        Err(RecvTimeoutError::Timeout) => panic!("{what}: no answer within {BOUND:?}"),
        Err(RecvTimeoutError::Disconnected) => panic!("{what}: the run panicked"),
    }
}

fn quarantined(got: Result<Vec<String>, Error>, what: &str) -> String {
    match got {
        Err(Error::Fatal {
            reason: FatalReason::Quarantined { what },
        }) => what,
        other => panic!("{what}: not Quarantined: {other:?}"),
    }
}

#[derive(Clone, Copy, Debug)]
enum Target {
    UsersHeap,
    LargeStore,
    /// The first page the users heap's directory names.
    UsersPage,
}

impl Target {
    const ALL: [Target; 3] = [Target::UsersHeap, Target::LargeStore, Target::UsersPage];

    /// Returns the target page and the page of the others heap standing for it, of another owner.
    fn pages(self, db: &Database<SimRuntime>) -> (Pid, Pid) {
        let t = db.testing();
        let users = t.heap("app::users", "User").unwrap();
        let others = t.heap("app::others", "Other").unwrap();
        let first = |heap| t.pages(heap).unwrap()[0];
        match self {
            Target::UsersHeap => (t.root(users).unwrap(), t.root(others).unwrap()),
            Target::LargeStore => {
                let large = t.large("app::users", "User").unwrap();
                (t.root(large).unwrap(), t.root(others).unwrap())
            }
            Target::UsersPage => (first(users), first(others)),
        }
    }

    /// Returns the name a refusal gives the holder of the target.
    fn holder(self) -> &'static str {
        match self {
            Target::UsersHeap | Target::UsersPage => "app::users.User",
            Target::LargeStore => "the database",
        }
    }

    /// Assert the others relation answers as the holder's mark leaves it.
    fn assert_others(self, db: &Database<SimRuntime>, what: &str) {
        match self {
            Target::UsersHeap | Target::UsersPage => {
                assert_eq!(vec![r#""o""#], text(db, OTHERS).unwrap(), "{what}");
            }
            Target::LargeStore => assert_eq!(self.holder(), quarantined(text(db, OTHERS), what)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Replacement {
    Kind(PageKind),
    /// The page of the others heap standing for the target.
    Others,
}

impl Replacement {
    /// Returns every kind a page of `target`'s directory cannot have, or the first of them unless
    /// every case runs, then the others heap's page.
    fn all(target: Target) -> Vec<Replacement> {
        let own: &[PageKind] = match target {
            Target::UsersPage => &[PageKind::HeapFixed, PageKind::HeapSlotted],
            _ => &[PageKind::FreeSpaceMap],
        };
        let kinds = (1..=11).filter_map(PageKind::from_u16);
        let kinds = kinds.filter(|k| !own.contains(k));
        let kinds = kinds.take(if every_case() { usize::MAX } else { 1 });
        let mut all: Vec<_> = kinds.map(Replacement::Kind).collect();
        all.push(Replacement::Others);
        all
    }

    /// Overwrite the page of `target` in `db` in place with this replacement.
    #[expect(clippy::disallowed_methods, reason = "until heap owns page checks")]
    fn apply(self, db: &Database<SimRuntime>, target: Target) {
        let (pid, others) = target.pages(db);
        let t = db.testing();
        let source = t.probe().page(others).unwrap();
        let shape = || {
            let page = t.probe().page(pid).unwrap();
            (page::kind(&page), page::owner(&page))
        };
        let before = shape();
        let (at, bytes) = match self {
            Replacement::Kind(k) => (KIND, (k as u16).to_le_bytes().to_vec()),
            Replacement::Others => (FIRST_EDITABLE, source[FIRST_EDITABLE..].to_vec()),
        };
        t.plant(Plant::Write {
            page: pid,
            at,
            bytes,
        })
        .unwrap();
        assert_ne!(before, shape(), "{self:?} changes the page's kind or owner");
    }
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Insert,
    LargeInsert,
    Read,
    Scan,
}

impl Op {
    const ALL: [Op; 4] = [Op::Insert, Op::LargeInsert, Op::Read, Op::Scan];

    fn text(self) -> String {
        match self {
            Op::Insert => insert(9, "n"),
            Op::LargeInsert => insert(9, &large()),
            Op::Read => "from u in users where u.id == users.key(1) select u.name".into(),
            Op::Scan => "from u in users select u.id".into(),
        }
    }

    /// Returns the rows this answers when it meets no page of `target`, or `None` when it does.
    fn rows(self, target: Target) -> Option<Vec<&'static str>> {
        match (self, target) {
            (Op::Read, Target::UsersHeap | Target::LargeStore) => Some(vec![r#""a""#]),
            (Op::Insert, Target::LargeStore) => Some(vec![]),
            (Op::Scan, Target::LargeStore) => Some(vec!["users.key(1)", "users.key(2)"]),
            _ => None,
        }
    }
}

#[test]
fn a_large_insert_after_its_directory_page_is_freed_refuses_in_bounded_time() {
    let what = "freed large directory".to_owned();
    bounded(&fixture(), what.clone(), move |db| {
        let root = Target::LargeStore.pages(db).0;
        db.testing().plant(Plant::Free(root)).unwrap();
        let got = text(db, &Op::LargeInsert.text());
        assert_eq!("the database", quarantined(got, &what));
    });
}

#[test]
fn every_call_over_a_foreign_page_in_a_directory_answers_in_bounded_time() {
    let base = fixture();
    let mut refused = 0;
    let mut read = 0;
    let mut reopened = 0;
    for target in Target::ALL {
        for replacement in Replacement::all(target) {
            let mut reopen = true;
            for op in Op::ALL {
                let what = format!("{target:?} replaced by {replacement:?}, {op:?}");
                let expected = op.rows(target);
                let (w, refusal) = (what.clone(), expected.is_none());
                let after = bounded(&base, what.clone(), move |db| {
                    replacement.apply(db, target);
                    let got = text(db, &op.text());
                    match &expected {
                        Some(rows) => {
                            assert_eq!(rows, &got.unwrap(), "{w}");
                            assert_eq!(vec![r#""o""#], text(db, OTHERS).unwrap(), "{w}");
                        }
                        None => {
                            assert_eq!(target.holder(), quarantined(got, &w));
                            target.assert_others(db, &w);
                        }
                    }
                });
                if !refusal {
                    read += 1;
                    continue;
                }
                refused += 1;
                if !std::mem::take(&mut reopen) {
                    continue;
                }
                bounded(&after, format!("{what}, reopened"), move |db| {
                    let got = text(db, &Op::Scan.text());
                    assert_eq!(target.holder(), quarantined(got, &what));
                    target.assert_others(db, &what);
                });
                reopened += 1;
            }
        }
    }
    let cases = |t| Replacement::all(t).len();
    let heap = cases(Target::UsersHeap);
    let (large, page) = (cases(Target::LargeStore), cases(Target::UsersPage));
    assert_reached("refused", refused, 3 * heap + large + 4 * page);
    assert_reached("read", read, heap + 3 * large);
    assert_reached("reopened", reopened, heap + large + page);
}

/// Make the one entry of directory page `dir` naming `from` name `to`.
fn point(db: &Database<SimRuntime>, dir: Pid, from: Pid, to: Pid) {
    let page = db.testing().probe().page(dir).unwrap();
    let id = |pid: Pid| pid.0.to_le_bytes()[..6].to_vec();
    let found: Vec<usize> = (FIRST_EDITABLE..page.len() - 6)
        .filter(|&at| page[at..at + 6] == id(from))
        .collect();
    assert_eq!(1, found.len(), "{dir:?} names {from:?} at {found:?}");
    db.testing()
        .plant(Plant::Write {
            page: dir,
            at: found[0],
            bytes: id(to),
        })
        .unwrap();
}

#[test]
fn an_entry_naming_a_foreign_page_quarantines_only_the_heap_of_its_directory() {
    let foreigns = [
        "another heap's page",
        "an allocation bitmap page",
        "its own directory page",
    ];
    for foreign in foreigns {
        let what = format!("an entry naming {foreign}");
        bounded(&fixture(), what.clone(), move |db| {
            let t = db.testing();
            let users = t.heap("app::users", "User").unwrap();
            let root = t.root(users).unwrap();
            let named = match foreign {
                "another heap's page" => Target::UsersPage.pages(db).1,
                "its own directory page" => root,
                _ => Pid::FIRST_DATA,
            };
            let kind = page::kind(&t.probe().page(named).unwrap());
            let bitmap = kind == PageKind::AllocationBitmap as u16;
            assert_eq!(foreign.contains("bitmap"), bitmap, "{what}: {kind}");
            point(db, root, t.pages(users).unwrap()[0], named);
            let got = text(db, &Op::Insert.text());
            assert_eq!("app::users.User", quarantined(got, &what));
            let only = BTreeSet::from([t.mark_of(users).unwrap()]);
            assert_eq!(only, t.probe().marks(), "{what}");
            assert_eq!(vec![r#""o""#], text(db, OTHERS).unwrap(), "{what}");
        });
    }
}

#[test]
fn a_read_over_an_entry_naming_a_large_value_or_map_page_blames_its_directory_page() {
    let mut cases = 0;
    for foreign in ["a large-value page", "another heap's map page"] {
        for (form, read) in [("scan", Op::Scan.text()), ("vacuum", "vacuum users".into())] {
            let what = format!("an entry naming {foreign}, {form}");
            let (w, read) = (what.clone(), read.clone());
            bounded(&fixture(), what, move |db| {
                let w = &w;
                let t = db.testing();
                let users = t.heap("app::users", "User").unwrap();
                let others = t.heap("app::others", "Other").unwrap();
                let root = t.root(users).unwrap();
                let map = PageKind::VisibilityMap as u16;
                let named = match foreign {
                    "a large-value page" => {
                        t.pages(t.large("app::users", "User").unwrap()).unwrap()[0]
                    }
                    _ => *(t.owned(others).unwrap().iter())
                        .find(|&&p| page::kind(&t.probe().page(p).unwrap()) == map)
                        .expect("the others heap has a map page"),
                };
                point(db, root, t.pages(users).unwrap()[0], named);
                assert_eq!("app::users.User", quarantined(text(db, &read), w));
                let met: BTreeSet<Pid> = t.probe().refusals().into_keys().collect();
                assert_eq!(BTreeSet::from([root]), met, "{w}");
                let only = BTreeSet::from([t.mark_of(users).unwrap()]);
                assert_eq!(only, t.probe().marks(), "{w}");
            });
            cases += 1;
        }
    }
    assert_eq!(4, cases);
}

#[test]
fn an_index_entry_outliving_its_page_refuses_the_index() {
    let what = "an index entry outliving its page".to_owned();
    bounded(&fixture(), what.clone(), move |db| {
        let t = db.testing();
        let users = t.heap("app::users", "User").unwrap();
        Replacement::Others.apply(db, Target::UsersPage);
        let pid = Target::UsersPage.pages(db).0;
        point(db, t.root(users).unwrap(), pid, Pid(0));
        assert_eq!(
            "app::users key id",
            quarantined(text(db, &Op::Read.text()), &what)
        );
        assert_eq!(0, t.probe().refusals().len(), "{what}");
        let marks = Vec::from_iter(t.probe().marks());
        assert!(
            matches!(marks[..], [Quarantine::Index(_)]),
            "{what}: {marks:?}"
        );
    });
}
