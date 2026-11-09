use crate::testing::{CommitLogPages, Object, Plant, Probe};
use adtb_harness::process;
use adtb_harness::process::finished;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::btree::{self, BTree};
use crate::exec::Val;
use crate::key::Value;
use crate::runtime::{Clock, Io, Op, Runtime, Threads};
use crate::storage::policy::{self, Fault, Holder, Phase};
use crate::storage::{Damage, HEADER_LEN, Options, PageError, Pager, Pid, Quarantine, Stopped};
use crate::{
    CallOptions, Called, Database, Error, FatalReason, Log, OpenOptions, Registration, TxnOptions,
};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};

use crate::test_world as harness;
use crate::test_world::api;
use crate::test_world::load as open;
use adtb_harness::damage::DataFile;
use adtb_sys::os::{Backend, OsRuntime};
use adtb_testkit::scratch::Scratch;

use harness::{NAME, PAGE};

fn options() -> Options {
    Options {
        segment_size: 64 * 1024,
        max_pages: 1024,
        ..harness::options(16)
    }
}

fn create(sim: &mut Sim) {
    finished(sim, |rt| {
        Pager::create(rt, &options()).unwrap();
    });
}

fn with_pager<T>(sim: &mut Sim, body: impl FnOnce(&Pager<SimRuntime>) -> T) -> T {
    let mut out = None;
    finished(sim, |rt| {
        let (p, _) = Pager::open(rt, &options()).unwrap();
        out = Some(body(&p));
    });
    out.unwrap()
}

fn with_named_pager<T>(sim: &mut Sim, body: impl FnOnce(&Pager<SimRuntime>) -> T) -> T {
    let mut out = None;
    finished(sim, |rt| {
        let options = Options {
            page_size: API_PAGE as u32,
            threads: 2,
            ..Options::new(FILE)
        };
        let (p, _) = Pager::open(rt, &options).unwrap();
        out = Some(body(&p));
    });
    out.unwrap()
}

#[test]
fn a_new_database_quarantines_nothing() {
    let mut sim = Sim::new(1, Config::default());
    create(&mut sim);
    with_pager(&mut sim, |p| {
        assert!(Probe::of(p).marks().is_empty());
        assert!(!Probe::of(p).marked(Quarantine::Heap(1)));
    });
}

#[test]
fn marks_survive_a_restart_until_released() {
    let mut sim = Sim::new(2, Config::default());
    create(&mut sim);
    with_pager(&mut sim, |p| {
        p.quarantine(Quarantine::Heap(7)).unwrap();
        p.quarantine(Quarantine::Index(3)).unwrap();
        p.quarantine(Quarantine::Heap(7)).unwrap();
        assert!(Probe::of(p).marked(Quarantine::Heap(7)));
        assert!(
            !Probe::of(p).marked(Quarantine::Heap(3)),
            "heap and index ids are apart"
        );
    });
    with_pager(&mut sim, |p| {
        assert_eq!(
            BTreeSet::from([Quarantine::Heap(7), Quarantine::Index(3)]),
            Probe::of(p).marks()
        );
        p.release(Quarantine::Heap(7)).unwrap();
    });
    with_pager(&mut sim, |p| {
        assert_eq!(BTreeSet::from([Quarantine::Index(3)]), Probe::of(p).marks());
        assert!(p.verify().findings.is_empty());
    });
}

#[test]
fn a_mark_is_durable_once_quarantine_returns() {
    for seed in 0..8 {
        let mut sim = Sim::new(seed, Config::default());
        create(&mut sim);
        sim.crash_at(sim.now() + Duration::from_secs(1));
        let outcome = sim.run(|rt| {
            let (p, _) = Pager::open(rt.clone(), &options()).unwrap();
            p.quarantine(Quarantine::Heap(40 + seed as u32)).unwrap();
            let mut io = rt.io();
            io.submit(Op::Timer {
                at: rt.now() + Duration::from_secs(10),
            });
            loop {
                rt.park();
            }
        });
        assert_eq!(Outcome::Crashed, outcome);
        with_pager(&mut sim, |p| {
            assert!(
                Probe::of(p).marked(Quarantine::Heap(40 + seed as u32)),
                "seed {seed}"
            );
        });
    }
}

#[test]
fn a_database_mark_covers_every_object() {
    let mut sim = Sim::new(3, Config::default());
    create(&mut sim);
    with_pager(&mut sim, |p| p.quarantine(Quarantine::Database).unwrap());
    with_pager(&mut sim, |p| {
        assert!(Probe::of(p).marked(Quarantine::Heap(9)));
        assert!(Probe::of(p).marked(Quarantine::Index(9)));
    });
}

#[test]
fn a_damaged_mark_page_quarantines_the_database() {
    let mut sim = Sim::new(4, Config::default());
    create(&mut sim);
    let page = with_pager(&mut sim, |p| {
        p.quarantine(Quarantine::Index(5)).unwrap();
        p.quarantine_page()
    });
    with_pager(&mut sim, |_| ());
    let mut files = sim.files();
    DataFile::new(NAME, PAGE).flip(&mut files, page, 200 - HEADER_LEN);
    let mut sim = Sim::with_files(4, Config::default(), files);
    with_pager(&mut sim, |p| {
        assert!(Probe::of(p).marked(Quarantine::Database));
        assert_eq!(
            Err(PageError {
                pid: page,
                damage: Damage::Checksum,
            }),
            p.quarantine(Quarantine::Heap(1))
        );
    });
}

const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
add type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text }
  | Google    { gid: Text }
  | Apple     { sub: Text }
add relation identities : Identity key id generate sequence
  index (user) in (Google | Apple)
  index (gid) in Google
add type Note = { id: Key notes, about: Key users }
add relation notes : Note key id generate sequence
  index (about)
add type Label = { id: Key tags, owner: Key users }
add relation tags : Label key id generate sequence
  index (owner)
add constraint no_apple_tags =
  from t in tags
  from a in identities.Apple
  where a.user == t.owner
  select t
";

const FILE: &str = "quarantine.adt";
const API_PAGE: usize = 8192;
const DATA: DataFile = DataFile::new(FILE, API_PAGE);
const SIDES: &str = "from i in identities.(EmailPass | Google) select i.id";
const ALL: &str = "from i in identities select i.id";

fn reads() -> Registration {
    Registration::new(
        "reads",
        "import app
query by_key (k: Key identities) -> Set (Key identities) =
  from i in identities where i.id == k select i.id
query by_gid (g: Text) -> Set (Key identities) =
  from i in identities.Google where i.gid == g select i.id
query google_of (u: Key users) -> Set Text =
  from i in identities.Google where i.user == u select i.gid
fn every_id () -> Set (Key identities) = from i in identities select i.id
query through_fn () -> Set (Key identities) = every_id()
",
    )
}

fn open_app(rt: SimRuntime) -> Database<SimRuntime> {
    let options = OpenOptions {
        registrations: vec![reads()],
        ..open::options(2)
    };
    Database::open_in(rt, FILE, &Log::new([APP]), &options).expect("the database opens")
}

fn seed(db: &Database<SimRuntime>) {
    for i in 1..=6 {
        api::results(
            db,
            &format!(
                r#"insert users {{ id: users.key({i}), name: "n{i}" }}
                   insert identities.EmailPass {{ id: identities.key({}), user: users.key({i}), email: "e{i}" }}
                   insert identities.Google {{ id: identities.key({}), user: users.key({i}), gid: "g{i}" }}
                   insert identities.Apple {{ id: identities.key({}), user: users.key({i}), sub: "a{i}" }}"#,
                i * 10,
                i * 10 + 1,
                i * 10 + 2,
            ),
        );
    }
}

fn heap_page(db: &Database<SimRuntime>, relation: &str, ctor: &str) -> Pid {
    let t = db.testing();
    t.pages(t.heap(relation, ctor).unwrap()).unwrap()[0]
}

/// Returns a simulator over a seeded file with page `pick` picks flipped on disk.
fn flipped(seed_no: u64, pick: impl FnOnce(&Database<SimRuntime>) -> Pid) -> Sim {
    let mut sim = Sim::new(seed_no, Config::default());
    let page = process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let page = pick(&db);
        db.close().unwrap();
        page
    });
    let mut files = sim.files();
    DATA.flip(&mut files, page, 300 - HEADER_LEN);
    Sim::with_files(seed_no, Config::default(), files)
}

fn apple(db: &Database<SimRuntime>) -> Pid {
    heap_page(db, "app::identities", "Apple")
}

fn quarantined(e: &Error) -> &str {
    match e {
        Error::Fatal {
            reason: FatalReason::Quarantined { what },
        } => what,
        other => panic!("not Quarantined: {other:?}"),
    }
}

fn failed_on(db: &Database<SimRuntime>, text: &str) -> String {
    quarantined(&api::failure(db, text)).to_owned()
}

fn marks<R: Runtime>(db: &Database<R>) -> BTreeSet<Quarantine> {
    db.testing().probe().marks()
}

fn apple_mark(db: &Database<SimRuntime>) -> Quarantine {
    let t = db.testing();
    t.mark_of(t.heap("app::identities", "Apple").unwrap())
        .unwrap()
}

fn call(db: &Database<SimRuntime>, query: &str, args: &[Val]) -> Result<Vec<String>, Error> {
    let bound = db.registered("reads").unwrap();
    let ordinal = bound.ordinal(query).unwrap();
    match bound.call(ordinal, args, &CallOptions::default())? {
        Called::Rows(rows) => Ok(api::sorted(rows)),
        other => panic!("{query} returns rows: {other:?}"),
    }
}

#[test]
fn a_file_with_a_damaged_catalog_root_opens_and_keeps_the_mark() {
    let mut sim = flipped(19, |db| db.testing().root(Object::Catalog).unwrap());
    process::run(&mut sim, |rt| {
        let got = Database::open_in(rt, FILE, &Log::new([APP]), &open::options(2));
        got.unwrap().close().unwrap();
    });
    with_named_pager(&mut sim, |p| {
        assert_eq!(BTreeSet::from([Quarantine::Database]), Probe::of(p).marks());
    });
}

#[test]
fn a_page_vacuum_frees_during_the_walk_marks_nothing() {
    let mut sim = Sim::new(52, Config::default());
    create(&mut sim);
    with_pager(&mut sim, |p| {
        let pid = p
            .allocate(1, crate::storage::PageKind::Quarantine, 0)
            .unwrap();
        let held = marked(p.holders_of(pid, Phase::Running, |_| {
            p.free(pid).unwrap();
        }));
        assert_eq!(Some(BTreeSet::new()), held);
    });
}

#[test]
fn a_failed_open_marks_the_heap_of_the_damage_it_met() {
    let mut sim = flipped(57, apple);
    let later = "module app\nadd constraint nope =\n  from a in identities.Apple\n  where a.sub == \"zz\"\n  select a\n";
    process::run(&mut sim, |rt| {
        let opts = OpenOptions {
            registrations: vec![reads()],
            ..open::options(2)
        };
        let got = Database::open_in(rt, FILE, &Log::new([APP, later]), &opts);
        assert!(
            matches!(got, Err(crate::OpenError::Corrupt { .. })),
            "{:?}",
            got.err()
        );
    });
    with_named_pager(&mut sim, |p| {
        let marks = Probe::of(p).marks();
        assert_eq!(1, marks.len(), "{marks:?}");
        assert!(
            matches!(marks.first(), Some(Quarantine::Heap(_))),
            "{marks:?}"
        );
    });
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert_eq!(BTreeSet::from([apple_mark(&db)]), marks(&db));
        assert_eq!("app::identities.Apple", failed_on(&db, ALL));
        api::query(&db, SIDES);
    });
}

#[test]
fn a_failed_open_on_a_damaged_page_of_the_large_value_store_marks_the_database() {
    let later = "module app\nadd constraint nope =\n  from u in users\n  where u.name == \"zz\"\n  select u\n";
    let mut sim = Sim::new(58, Config::default());
    let page = process::run(&mut sim, |rt| {
        let db = open_app(rt);
        let long = "x".repeat(40_000);
        api::results(
            &db,
            &format!(r#"insert users {{ id: users.key(1), name: "{long}" }}"#),
        );
        let t = db.testing();
        let large = t.large("app::users", "User").unwrap();
        let page = *t.owned(large).unwrap().last().unwrap();
        db.close().unwrap();
        page
    });
    let mut files = sim.files();
    DATA.flip(&mut files, page, 300 - HEADER_LEN);
    let mut sim = Sim::with_files(58, Config::default(), files);
    process::run(&mut sim, |rt| {
        let got = Database::open_in(rt, FILE, &Log::new([APP, later]), &open::options(2));
        let db = got.expect("a file quarantined whole opens for verify");
        assert_eq!(
            "the database",
            failed_on(&db, "from u in users select u.id")
        );
    });
    let database = BTreeSet::from([Quarantine::Database]);
    with_named_pager(&mut sim, |p| assert_eq!(database, Probe::of(p).marks()));
}

fn commit_log_page(seed_no: u64, pick: impl FnOnce(CommitLogPages) -> Pid) -> Sim {
    let mut sim = Sim::new(seed_no, Config::default());
    let page = process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let page = pick(db.testing().commit_log());
        db.close().unwrap();
        page
    });
    let mut files = sim.files();
    DATA.flip(&mut files, page, 300 - HEADER_LEN);
    Sim::with_files(seed_no, Config::default(), files)
}

#[test]
fn a_file_with_a_damaged_commit_log_status_page_opens_for_verify() {
    let mut sim = commit_log_page(58, |clog| *clog.status().values().next_back().unwrap());
    for _ in 0..2 {
        process::run(&mut sim, |rt| {
            let db = open_app(rt);
            assert_eq!(BTreeSet::from([Quarantine::Database]), marks(&db));
            assert_eq!(
                "the database",
                failed_on(&db, "from u in users select u.id")
            );
            assert_eq!(
                "the database",
                failed_on(&db, r#"insert users { id: users.key(99), name: "x" }"#)
            );
            assert_eq!(
                "the database",
                quarantined(&db.begin(&TxnOptions::default()).err().unwrap())
            );
            let e = call(&db, "through_fn", &[]).expect_err("fails");
            assert_eq!("the database", quarantined(&e));
            let found = api::printed(db.verify(None).unwrap());
            assert!(found.iter().any(|r| r.starts_with("Checksum")), "{found:?}");
            db.close().unwrap();
        });
    }
}

#[test]
fn a_file_with_a_damaged_commit_log_root_opens_and_keeps_the_mark() {
    let mut sim = commit_log_page(59, |clog| clog.root());
    process::run(&mut sim, |rt| {
        let got = Database::open_in(rt, FILE, &Log::new([APP]), &open::options(2));
        got.unwrap().close().unwrap();
    });
    with_named_pager(&mut sim, |p| {
        assert_eq!(BTreeSet::from([Quarantine::Database]), Probe::of(p).marks());
    });
}

const BOUNDED: &str = "from i in identities where i.id == identities.key(11) select i.user";

fn key_index_marked(db: &Database<SimRuntime>) -> Object {
    let t = db.testing();
    let key = t.index("app::identities", "key id").unwrap();
    t.plant(Plant::Mark(t.mark_of(key).unwrap())).unwrap();
    key
}

#[test]
fn a_probe_from_the_heaps_stops_on_the_check_of_its_call() {
    let mut sim = Sim::new(79, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let before = api::query(&db, BOUNDED);
        let key = key_index_marked(&db);
        assert_eq!(before, api::query(&db, BOUNDED));
        let got = db.testing().probe_heaps(key, &[], 1 << 30, true).unwrap();
        assert!(matches!(got, Err(crate::exec::Error::Cancelled)), "{got:?}");
    });
}

#[test]
fn a_probe_from_the_heaps_charges_the_versions_it_scans_to_the_grant() {
    let mut sim = Sim::new(80, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let key = key_index_marked(&db);
        let got = db
            .testing()
            .probe_heaps(key, &[Value::I64(i64::MAX)], 8, false)
            .unwrap();
        assert!(
            matches!(got, Err(crate::exec::Error::StatementLimit(_))),
            "{got:?}"
        );
    });
}

fn entry(i: u32) -> Vec<u8> {
    let mut v = i.to_be_bytes().to_vec();
    v.resize(500, 7);
    v
}

/// Returns a scratch directory labelled `name`, a pager on real threads over it and three trees
/// of it. The pager must not outlive the directory.
fn real_trees(name: &str) -> (Scratch, OsRuntime, Arc<Pager<OsRuntime>>, [BTree; 3]) {
    let dir = Scratch::new("quarantine", name);
    let rt = OsRuntime::new(dir.path(), Backend::Pool { threads: 4 });
    let mut o = Options::new("w.adt");
    o.threads = 70;
    o.pool_frames = 8192;
    o.page_size = 4096;
    let pager = Arc::new(Pager::create(rt.clone(), &o).unwrap());
    let trees = [41, 42, 43].map(|owner| BTree::create(&*pager, owner).unwrap());
    (dir, rt, pager, trees)
}

/// Yield until `limit` passes.
fn wait(rt: &OsRuntime, limit: Duration) {
    let deadline = rt.now() + limit;
    while rt.now() < deadline {
        rt.yield_now();
    }
}

/// Run `mutate` and `map` on 64 threads until `limit` passes. Returns each mapping `map` answered,
/// with how many times.
fn mapped_under_churn<T: std::fmt::Debug>(
    rt: &OsRuntime,
    limit: Duration,
    mutate: impl FnOnce() + Send + 'static,
    map: impl Fn() -> T + Send + Sync + 'static,
) -> BTreeMap<String, u64> {
    let map = Arc::new(map);
    let stop = Arc::new(AtomicBool::new(false));
    let got = Arc::new(Mutex::new(BTreeMap::new()));
    let mappers: Vec<_> = (0..64)
        .map(|i| {
            let (map, stop, got) = (map.clone(), stop.clone(), got.clone());
            rt.spawn(&format!("mapper {i}"), move || {
                while !stop.load(Ordering::Relaxed) {
                    let held = map();
                    *got.lock()
                        .unwrap()
                        .entry(format!("{held:?}"))
                        .or_insert(0u64) += 1;
                }
            })
        })
        .collect();
    let mutator = rt.spawn("mutator", mutate);
    wait(rt, limit);
    stop.store(true, Ordering::Relaxed);
    for m in mappers {
        rt.join(m);
    }
    rt.join(mutator);
    Arc::try_unwrap(got).unwrap().into_inner().unwrap()
}

#[test]
fn mapping_a_page_of_one_tree_never_marks_a_tree_changing_beside_it() {
    let (_dir, rt, pager, [a, b, c]) = real_trees("beside");
    for i in 0..400 {
        a.insert(&*pager, &entry(i)).unwrap();
        c.insert(&*pager, &entry(i)).unwrap();
    }
    let damaged = *c.pages(&*pager).unwrap().last().unwrap();
    pager.damage(damaged, Damage::Checksum);
    let limit = Duration::from_millis(1200);
    let mutate = churn_tree(&rt, limit, &pager, a, b);
    let got = {
        let p = pager;
        mapped_under_churn(&rt, limit, mutate, move || {
            marked(p.holders_of(damaged, Phase::Running, |h| {
                h.walk(relation(Quarantine::Index(41)), || a.walk(&*p));
                h.walk(relation(Quarantine::Index(43)), || c.walk(&*p));
            }))
        })
    };
    assert_eq!(1, got.len(), "{got:?}");
    assert!(got.contains_key("Some({Index(43)})"), "{got:?}");
}

/// Delete and insert the entries of `a`, with `b` taking the pages freed, until `limit` from now.
fn churn_tree(
    rt: &OsRuntime,
    limit: Duration,
    p: &Arc<Pager<OsRuntime>>,
    a: BTree,
    b: BTree,
) -> impl FnOnce() + Send + 'static {
    let deadline = rt.now() + limit;
    let (rt, p) = (rt.clone(), p.clone());
    move || {
        let mut n = 0;
        while rt.now() < deadline {
            for i in 0..400 {
                a.delete(&*p, &entry(i)).unwrap();
                if i % 2 == 0 {
                    b.insert(&*p, &entry(n)).unwrap();
                    n += 1;
                }
            }
            for i in 0..400 {
                a.insert(&*p, &entry(i)).unwrap();
            }
        }
    }
}

#[test]
fn a_walk_of_a_changing_tree_never_stops_on_damage() {
    let (_dir, rt, pager, [a, b, _]) = real_trees("walk");
    for i in 0..400 {
        a.insert(&*pager, &entry(i)).unwrap();
    }
    let limit = Duration::from_millis(1200);
    let mutate = churn_tree(&rt, limit, &pager, a, b);
    let got = {
        let p = pager;
        mapped_under_churn(&rt, limit, mutate, move || a.walk(&*p).map(|_| ()))
    };
    assert!(!got.contains_key("Err(Damage)"), "{got:?}");
}

#[test]
fn mapping_a_page_of_a_changing_tree_finds_that_tree_or_nothing() {
    let (_dir, rt, pager, [a, _, _]) = real_trees("within");
    for i in 0..400 {
        a.insert(&*pager, &entry(i)).unwrap();
    }
    let pages = a.pages(&*pager).unwrap();
    let damaged = pages[pages.len() - pages.len() / 2];
    pager.damage(damaged, Damage::Checksum);
    let deadline = rt.now() + Duration::from_millis(300);
    let mutate = {
        let p = pager.clone();
        let rt = rt.clone();
        move || {
            while rt.now() < deadline {
                for i in 0..400 {
                    match a.delete(&*p, &entry(i)) {
                        Ok(_) | Err(btree::Error::Page(_)) => {}
                        Err(e) => panic!("delete {i}: {e:?}"),
                    }
                }
                for i in 0..400 {
                    match a.insert(&*p, &entry(i)) {
                        Ok(_) | Err(btree::Error::Page(_)) => {}
                        Err(e) => panic!("insert {i}: {e:?}"),
                    }
                }
            }
        }
    };
    let got = {
        let p = pager;
        mapped_under_churn(&rt, Duration::from_millis(300), mutate, move || {
            marked(p.holders_of(damaged, Phase::Running, |h| {
                h.walk(relation(Quarantine::Index(41)), || a.walk(&*p));
            }))
        })
    };
    assert!(!got.contains_key("Some({Database})"), "{got:?}");
    assert!(got.contains_key("Some({Index(41)})"), "{got:?}");
}

const LISTS: &str = r"module app
add module app
add type U = { id: Key us, name: Text }
add relation us : U key id generate sequence
add type L = { id: Key ls, s: Text }
add relation ls : L key id generate sequence
  index (s)
";

/// Insert and delete rows of `ls`, by `writer`, and vacuum `ls`, until `stop`. Fail on any error
/// but a serialization failure.
fn churn_lists(db: &Database, writer: i64, stop: &AtomicBool) {
    let run = |text: &str| match db.query_text(text, &[], &CallOptions::default()) {
        Ok(_) => {}
        Err(e) if e.is_serialization() => {}
        Err(e) => panic!("{text:.60}: {e:?}"),
    };
    let mut round = 0;
    while !stop.load(Ordering::Relaxed) {
        round += 1;
        let keys = (0..50).map(|i| writer * 1_000_000 + round * 1000 + i);
        let pad = "x".repeat(1500);
        let mut insert = String::new();
        let mut delete = String::new();
        for k in keys {
            insert.push_str(&format!(
                "insert ls {{ id: ls.key({k}), s: \"{k:08}{pad}\" }}\n"
            ));
            delete.push_str(&format!("delete l in ls where key l == ls.key({k})\n"));
        }
        run(&insert);
        run(&delete);
        run("vacuum ls");
    }
}

/// Returns a scratch directory labelled `name`, a runtime on real threads over it and a database
/// in it. The database must not outlive the directory.
fn real_lists(name: &str) -> (Scratch, OsRuntime, Database) {
    let dir = Scratch::new("quarantine", name);
    let rt = OsRuntime::new(dir.path(), Backend::Pool { threads: 4 });
    let options = OpenOptions {
        create: Some(crate::CreateOptions::default()),
        workers: Some(2),
        ..OpenOptions::default()
    };
    let db = Database::open_in(rt.clone(), "f.adt", &Log::new([LISTS]), &options).unwrap();
    (dir, rt, db)
}

#[test]
fn mapping_a_damaged_heap_page_never_marks_an_index_that_is_changing() {
    let (_dir, rt, db) = real_lists("engine");
    db.query_text(
        r#"insert us { id: us.key(1), name: "a" }"#,
        &[],
        &CallOptions::default(),
    )
    .unwrap();
    let t = db.testing();
    let page = t.pages(t.heap("app::us", "U").unwrap()).unwrap()[0];
    db.testing()
        .plant(Plant::Refuse {
            page,
            damage: Damage::Checksum,
        })
        .unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let writers: Vec<_> = (0..3)
        .map(|writer| {
            let (db, stop) = (db.clone(), stop.clone());
            rt.spawn(&format!("writer {writer}"), move || {
                churn_lists(&db, writer, &stop);
            })
        })
        .collect();
    let mappers: Vec<_> = (0..48)
        .map(|i| {
            let (db, stop) = (db.clone(), stop.clone());
            rt.spawn(&format!("mapper {i}"), move || {
                while !stop.load(Ordering::Relaxed) {
                    let met = PageError {
                        pid: page,
                        damage: Damage::Checksum,
                    };
                    let planted = db.testing().plant(Plant::FailOn(met)).unwrap();
                    let e = planted.refused().expect("a page error is refused");
                    assert!(
                        matches!(&e, crate::write::Error::Quarantined(what) if what == "app::us.U"),
                        "{e:?}"
                    );
                }
            })
        })
        .collect();
    wait(&rt, Duration::from_millis(1100));
    stop.store(true, Ordering::Relaxed);
    for t in writers.into_iter().chain(mappers) {
        rt.join(t);
    }
    let heap = t.mark_of(t.heap("app::us", "U").unwrap()).unwrap();
    assert_eq!(BTreeSet::from([heap]), marks(&db));
}

/// Run `body` with a page of a fresh pager that no walk holds.
fn with_unheld_page(seed_no: u64, body: impl FnOnce(&Pager<SimRuntime>, Pid)) {
    let mut sim = Sim::new(seed_no, Config::default());
    create(&mut sim);
    with_pager(&mut sim, |p| {
        let pid = p
            .allocate(1, crate::storage::PageKind::Quarantine, 0)
            .unwrap();
        body(p, pid);
    });
}

/// Returns what holds `pid` when one walk of `object` gives each of `script` in turn, its last
/// again and again, and how many walks ran.
fn scripted(
    p: &Pager<SimRuntime>,
    pid: Pid,
    script: &[Result<Vec<Pid>, Stopped>],
) -> (Option<BTreeSet<Quarantine>>, usize) {
    let mut walks = 0;
    let held = p.holders_of(pid, Phase::Running, |h| {
        h.walk(relation(Quarantine::Index(7)), || {
            walks += 1;
            script[(walks - 1).min(script.len() - 1)].clone()
        });
    });
    (marked(held), walks)
}

#[test]
fn a_walk_that_never_follows_its_tree_leaves_the_page_unmapped() {
    with_unheld_page(60, |p, pid| {
        let (held, walks) = scripted(p, pid, &[Err(Stopped::Shape)]);
        assert_eq!(None, held);
        assert!((2..=64).contains(&walks), "{walks}");
    });
}

#[test]
fn a_walk_that_stops_on_a_shape_is_run_again() {
    with_unheld_page(61, |p, pid| {
        let index = Some(BTreeSet::from([Quarantine::Index(7)]));
        let script = [Err(Stopped::Shape), Err(Stopped::Shape), Ok(vec![pid])];
        assert_eq!((index, 3), scripted(p, pid, &script));
        let script = [Err(Stopped::Shape), Ok(Vec::new())];
        let database = Some(BTreeSet::from([Quarantine::Database]));
        assert_eq!((database, 2), scripted(p, pid, &script));
    });
}

#[test]
fn a_walk_marks_its_object_only_when_damage_stops_it_twice_in_a_row() {
    with_unheld_page(62, |p, pid| {
        let index = Some(BTreeSet::from([Quarantine::Index(7)]));
        let database = Some(BTreeSet::from([Quarantine::Database]));
        let script = [Err(Stopped::Damage(pid))];
        assert_eq!((index.clone(), 2), scripted(p, pid, &script));
        let script = [Err(Stopped::Damage(pid)), Ok(Vec::new())];
        assert_eq!((database, 2), scripted(p, pid, &script));
        let script = [
            Err(Stopped::Damage(pid)),
            Err(Stopped::Shape),
            Err(Stopped::Damage(pid)),
            Err(Stopped::Damage(pid)),
        ];
        assert_eq!((index, 4), scripted(p, pid, &script));
        let script = [Err(Stopped::Damage(pid)), Err(Stopped::Shape)];
        assert_eq!(None, scripted(p, pid, &script).0);
    });
}

#[test]
fn a_walk_stopped_by_io_is_not_run_again_and_leaves_the_page_unmapped() {
    with_unheld_page(63, |p, pid| {
        let (held, walks) = scripted(p, pid, &[Err(Stopped::Io)]);
        assert_eq!((None, 1), (held, walks));
    });
}

#[test]
fn a_walk_of_an_object_whose_damage_marks_nothing_stopped_by_damage_leaves_the_page_unmapped() {
    with_unheld_page(66, |p, pid| {
        let other = Pid(pid.0 + 1);
        let held = p.holders_of(pid, Phase::Running, |h| {
            h.walk(Holder::Retired, || Err(Stopped::Damage(other)));
        });
        assert_eq!(None, held);
    });
}

/// Returns the marks a page fault on `held` sets once the database runs.
fn marked(held: Option<BTreeSet<Holder>>) -> Option<BTreeSet<Quarantine>> {
    held.map(|h| policy::marks(Fault::Page(Damage::Checksum), h, Phase::Running))
}

fn relation(object: Quarantine) -> Holder {
    Holder::Relation {
        object,
        engine_module: false,
    }
}

fn mark_named(db: &Database<SimRuntime>, relation: &str, ctor: &str) -> Quarantine {
    let t = db.testing();
    t.mark_of(t.heap(relation, ctor).unwrap()).unwrap()
}

type Files = BTreeMap<String, Vec<u8>>;

/// Returns `files` after a pager writing pages whole opens them and writes a byte of `pid`, then
/// a close, or a crash once its log is durable.
fn written_whole(files: Files, pid: Pid, crash: bool) -> Files {
    let mut sim = Sim::with_files(98, Config::default(), files);
    let body = |rt: SimRuntime| {
        let options = Options {
            page_size: API_PAGE as u32,
            threads: 2,
            atomic_page_writes: true,
            ..Options::new(FILE)
        };
        let (p, _) = Pager::open(rt, &options).unwrap();
        p.update(&[pid], |pages| pages[0][API_PAGE / 2] ^= 1)
            .unwrap();
        p.flush_log();
        p
    };
    if crash {
        adtb_harness::common::run_then_crash(&mut sim, |rt, _| drop(body(rt.clone())));
    } else {
        finished(&mut sim, |rt| body(rt).close());
    }
    sim.files()
}

#[test]
fn a_lost_write_recovery_finds_is_marked_by_an_open_that_succeeds_and_refuses_the_next() {
    let mut sim = Sim::new(98, Config::default());
    let (pid, users) = process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let pid = heap_page(&db, "app::users", "User");
        let users = mark_named(&db, "app::users", "User");
        db.close().unwrap();
        (pid, users)
    });
    let first = sim.files();
    let written = written_whole(first.clone(), pid, false);
    let mut lost = written_whole(written, pid, true);
    DATA.replace(&mut lost, pid, DATA.page(&first, pid));
    let mut sim = Sim::with_files(98, Config::default(), lost);
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert!(
            matches!(
                db.testing().probe().records().pages().get(&pid),
                Some(Damage::Stale { .. })
            ),
            "recovery records the lost write"
        );
        assert_eq!(BTreeSet::from([users]), marks(&db));
        db.close().unwrap();
    });
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert_eq!(BTreeSet::from([users]), marks(&db));
        assert_eq!(
            "app::users.User",
            failed_on(&db, "from u in users select u.id")
        );
        db.close().unwrap();
    });
}
