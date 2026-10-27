use std::collections::{BTreeMap, BTreeSet};
use std::ops::{Bound, Range};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use adtb::btree::{BTree, Index};
use adtb::heap::{FieldType, Flags, Heap, HeapNo, LargeStore, walk};
use adtb::key::{CtorId, Dir, Entry, KeySchema, KeyType, Scalar, Tid, Value};
use adtb::mvcc::{Scope, Snapshot, Status, Transactions, Xid};
use adtb::runtime::{Entropy, Threads};
use adtb::storage::{Pager, Pid};
use adtb::vacuum::{self, Phase, Progress, Sum};
use adtb::verify::{self, Inventory};
use adtb_sim::{Config, Faults, Outcome, Sim, SimRuntime};

#[path = "support/crash.rs"]
mod crash;
#[path = "support/harness.rs"]
mod harness;
#[path = "support/recovery.rs"]
mod recovery;
#[path = "support/rows.rs"]
#[expect(dead_code, reason = "this binary keeps its own row layout")]
mod rows;
#[path = "support/seeds.rs"]
mod seeds;

use crash::{crash_after, run_process, silence_fatal_panics};
use harness::{finished, options};
use recovery::verify_after_crash;
use rows::{APPLE, HEAP_IDS, LARGE_ID, Live, Op, Rows, key_of, replay};
use seeds::{share, sweep};

const FRAMES: usize = 96;

const INDEX_ID: u32 = 20;
const WRITERS: u64 = 3;
const SWITCH_RATE: f64 = 0.3;

fn row_type(heap: usize) -> Vec<FieldType> {
    match heap {
        APPLE => vec![
            FieldType::Scalar(Scalar::I64),
            FieldType::Scalar(Scalar::I32),
        ],
        _ => vec![
            FieldType::Scalar(Scalar::I64),
            FieldType::Scalar(Scalar::Bytes),
        ],
    }
}

fn make_row(heap: usize, key: i64, r: u64) -> Vec<Value> {
    match heap {
        APPLE => vec![Value::I64(key), Value::I32(r as i32)],
        _ => {
            let len = if r.is_multiple_of(7) {
                1500 + (r % 4000) as usize
            } else {
                (r % 100) as usize
            };
            let bytes = (0..len).map(|i| (r as usize + i * 13) as u8).collect();
            vec![Value::I64(key), Value::Bytes(bytes)]
        }
    }
}

fn schema() -> KeySchema {
    KeySchema::new(vec![(KeyType::Scalar(Scalar::I64), Dir::Asc)])
}

fn ctors() -> BTreeSet<CtorId> {
    BTreeSet::from([CtorId(0), CtorId(1), CtorId(2)])
}

#[derive(Clone, Copy, Debug)]
struct Roots {
    clog: Pid,
    large: Pid,
    heaps: [Pid; 3],
    index: Pid,
}

/// One database shared by every thread of a process.
struct Db {
    pager: Pager<SimRuntime>,
    txns: Transactions<SimRuntime>,
    heaps: Vec<Heap>,
    index: Index,
}

impl Db {
    fn create(rt: SimRuntime) -> (Db, Roots) {
        let pager = Pager::create(rt.clone(), &options(FRAMES)).expect("create");
        let txns = Transactions::create(rt, &pager).expect("commit log");
        let large = LargeStore::create(&pager, LARGE_ID).expect("large store");
        let heaps: Vec<Heap> = (0..3)
            .map(|h| Heap::create_with_fanout(&pager, HEAP_IDS[h], row_type(h), large, 4).unwrap())
            .collect();
        let index = Index::create(&pager, INDEX_ID, schema(), ctors()).expect("index");
        let roots = Roots {
            clog: txns.commit_log().root(),
            large: large.root(),
            heaps: [heaps[0].root(), heaps[1].root(), heaps[2].root()],
            index: index.tree().root(),
        };
        pager.flush_log();
        (
            Db {
                pager,
                txns,
                heaps,
                index,
            },
            roots,
        )
    }

    fn open(rt: SimRuntime, roots: Roots) -> Option<Db> {
        let (pager, _) = Pager::open(rt.clone(), &options(FRAMES)).ok()?;
        let txns = Transactions::open(rt, &pager, roots.clog).ok()?;
        let large = LargeStore::open(LARGE_ID, roots.large);
        let heaps = (0..3)
            .map(|h| Heap::open(&pager, HEAP_IDS[h], roots.heaps[h], row_type(h), large).unwrap())
            .collect();
        let index = Index::open(BTree::open(roots.index, INDEX_ID), schema(), ctors());
        Some(Db {
            pager,
            txns,
            heaps,
            index,
        })
    }

    fn sum(&self) -> Sum<'_> {
        Sum {
            heaps: (0..3).map(|h| (CtorId(h as u32), &self.heaps[h])).collect(),
            indexes: vec![&self.index],
        }
    }

    fn vacuum(&self, budget: usize, progress: &Progress) {
        vacuum::vacuum(&self.pager, &self.txns, &self.sum(), budget, progress).expect("vacuum");
    }

    fn index(&self, heap: usize, key: i64, tid: Tid) {
        let entry = Entry {
            key: vec![Value::I64(key)],
            ctor: CtorId(heap as u32),
            tid,
        };
        assert!(self.index.insert(&self.pager, &entry).expect("index"));
    }

    /// Returns the rows `snapshot` sees, by key, from a scan of every heap; and keys seen twice.
    fn scan_visible(&self, snapshot: &Snapshot) -> (Rows, Vec<i64>) {
        let mut out = Rows::new();
        let mut twice = Vec::new();
        for (h, heap) in self.heaps.iter().enumerate() {
            let mut versions = Vec::new();
            heap.scan(&self.pager, |v| versions.push(v)).expect("scan");
            for v in versions {
                let k = key_of(&v.row);
                if self
                    .txns
                    .sees(&self.pager, snapshot, None, v.header.xmin, v.header.xmax)
                    .expect("sees")
                    && out.insert(k, (h, v.row)).is_some()
                {
                    twice.push(k);
                }
            }
        }
        (out, twice)
    }

    /// Returns the rows `snapshot` sees through the index: each entry's version or a heap-only
    /// successor of it on its page.
    fn index_visible(&self, snapshot: &Snapshot) -> Rows {
        let entries = self
            .index
            .range(&self.pager, Bound::Unbounded, Bound::Unbounded)
            .expect("index scan");
        let mut out = Rows::new();
        for e in entries {
            let h = e.ctor.0 as usize;
            let mut first = true;
            let mut seen = Vec::new();
            walk(&self.pager, HeapNo(HEAP_IDS[h]), e.tid, |l| {
                if l.tid.page() != e.tid.page() || !first && !l.flags.contains(Flags::HEAP_ONLY) {
                    return true;
                }
                first = false;
                seen.push((l.tid, l.header.xmin, l.header.xmax));
                false
            })
            .expect("walk");
            for (t, xmin, xmax) in seen {
                if self
                    .txns
                    .sees(&self.pager, snapshot, None, xmin, xmax)
                    .expect("sees")
                {
                    if let Some(v) = self.heaps[h].read(&self.pager, t).expect("read") {
                        out.insert(key_of(&v.row), (h, v.row));
                    }
                    break;
                }
            }
        }
        out
    }

    /// Returns what is wrong with the all-visible bits of a quiet database: a marked page with
    /// a version a fresh snapshot does not see.
    fn check_visibility_map(&self) -> Vec<String> {
        let (id, fresh) = self.txns.take(Scope::Every).unwrap();
        let mut problems = Vec::new();
        for (h, heap) in self.heaps.iter().enumerate() {
            let mut versions = Vec::new();
            heap.scan(&self.pager, |v| versions.push(v)).expect("scan");
            for v in versions {
                let pid = Pid(v.tid.page());
                let marked = heap.all_visible(&self.pager, pid).unwrap();
                let sees = self
                    .txns
                    .sees(&self.pager, &fresh, None, v.header.xmin, v.header.xmax)
                    .unwrap();
                if marked && !sees {
                    problems.push(format!(
                        "heap {h} {pid:?} is all-visible, {:?} is not",
                        v.tid
                    ));
                }
            }
        }
        self.txns.release(id);
        problems
    }

    fn fresh(&self) -> (Rows, Vec<i64>, Rows) {
        let (id, s) = self.txns.take(Scope::Every).unwrap();
        let (scan, twice) = self.scan_visible(&s);
        let index = self.index_visible(&s);
        self.txns.release(id);
        (scan, twice, index)
    }

    fn verify_file(&self) -> Vec<String> {
        let clog = self.txns.commit_log();
        let inventory = Inventory {
            commit_log: Some(&clog),
            sums: vec![verify::Sum {
                heaps: (0..3).map(|h| (CtorId(h as u32), &self.heaps[h])).collect(),
                indexes: vec![&self.index],
            }],
            ..Inventory::default()
        };
        verify_after_crash(&self.pager, &inventory)
    }
}

/// Let other threads run for about `n` switches.
fn pause(rt: &SimRuntime, n: u64) {
    for _ in 0..=(n / 100) {
        rt.yield_now();
    }
}

/// Run one operation of `xid` against `live`, a writer's own keys.
fn perform(db: &Db, live: &mut Live, op: &Op, xid: Xid) {
    match op {
        Op::Insert { key, heap, row } => {
            let tid =
                vacuum::insert(&db.pager, &db.txns, &db.heaps[*heap], xid, row).expect("insert");
            db.index(*heap, *key, tid);
            live.insert(*key, (*heap, tid, row.clone()));
        }
        Op::Update { key, row } => {
            let (heap, tid, _) = live[key].clone();
            let u = vacuum::update(&db.pager, &db.txns, &db.heaps[heap], tid, xid, row, &[0])
                .expect("update");
            if !u.heap_only {
                db.index(heap, *key, u.tid);
            }
            live.insert(*key, (heap, u.tid, row.clone()));
        }
        Op::Delete { key } => {
            let (heap, tid, _) = live.remove(key).unwrap();
            db.heaps[heap]
                .delete(&db.pager, tid, xid.0)
                .expect("delete");
        }
        Op::Retag { key, to, row } => {
            let (heap, tid, _) = live[key].clone();
            let t = db.heaps[heap]
                .retag(&db.pager, tid, xid.0, &db.heaps[*to], row)
                .expect("retag");
            db.index(*to, *key, t);
            live.insert(*key, (*to, t, row.clone()));
        }
    }
}

fn generate(rt: &SimRuntime, live: &Live, next_key: &mut i64) -> Op {
    let r = rt.next_u64();
    if live.is_empty() || r % 10 < 3 {
        let key = *next_key;
        *next_key += WRITERS as i64;
        let heap = (rt.next_u64() % 3) as usize;
        return Op::Insert {
            key,
            heap,
            row: make_row(heap, key, rt.next_u64()),
        };
    }
    let keys: Vec<i64> = live.keys().copied().collect();
    let key = keys[(rt.next_u64() % keys.len() as u64) as usize];
    let (heap, _, _) = &live[&key];
    match r % 10 {
        3..=6 => Op::Update {
            key,
            row: make_row(*heap, key, rt.next_u64()),
        },
        7 => Op::Delete { key },
        _ => {
            let to = (heap + 1 + (rt.next_u64() % 2) as usize) % 3;
            Op::Retag {
                key,
                to,
                row: make_row(to, key, rt.next_u64()),
            }
        }
    }
}

/// What the clients know, kept outside the simulated process so it survives crashes.
#[derive(Default)]
struct World {
    roots: Option<Roots>,
    /// Each transaction's operations, by the writer that ran it.
    ops: BTreeMap<Xid, Vec<Op>>,
    acked: BTreeSet<Xid>,
    next_keys: BTreeMap<u64, i64>,
    vacuums: usize,
    reads: usize,
    vacuuming: bool,
    /// Crashes that came while a vacuum ran.
    crashed_vacuuming: usize,
}

impl World {
    fn expected(&self, committed: impl Fn(Xid) -> bool) -> Rows {
        let mut state = Rows::new();
        for (x, ops) in &self.ops {
            if committed(*x) {
                for op in ops {
                    replay(&mut state, op);
                }
            }
        }
        state
    }
}

/// Run `steps` transactions of writer `w` over its own keys.
fn writer(rt: &SimRuntime, db: &Db, world: &Mutex<World>, w: u64, mut live: Live, steps: usize) {
    let mut next_key = world
        .lock()
        .unwrap()
        .next_keys
        .get(&w)
        .copied()
        .unwrap_or(w as i64);
    let mut unacked = Vec::new();
    for _ in 0..steps {
        let xid = db.txns.begin(&db.pager).expect("begin");
        let mut mine = live.clone();
        let mut ops = Vec::new();
        for _ in 0..=(rt.next_u64() % 3) {
            let op = generate(rt, &mine, &mut next_key);
            perform(db, &mut mine, &op, xid);
            ops.push(op);
        }
        {
            let mut wd = world.lock().unwrap();
            wd.ops.insert(xid, ops);
            wd.next_keys.insert(w, next_key);
        }
        if rt.next_u64().is_multiple_of(6) {
            db.txns.abort(&db.pager, xid).expect("abort");
        } else {
            db.txns.commit(&db.pager, xid).expect("commit");
            live = mine;
            unacked.push(xid);
        }
        if rt.next_u64().is_multiple_of(4) {
            db.pager.flush_log();
            world.lock().unwrap().acked.extend(unacked.drain(..));
        }
    }
    db.pager.flush_log();
    world.lock().unwrap().acked.extend(unacked);
}

/// Hold snapshots across vacuums and check each reads the same rows by scan and by index every
/// time.
fn reader(rt: &SimRuntime, db: &Db, world: &Mutex<World>, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        let scope = if rt.next_u64().is_multiple_of(2) {
            Scope::Every
        } else {
            Scope::Heaps(BTreeSet::from([HEAP_IDS[(rt.next_u64() % 3) as usize]]))
        };
        let every = scope == Scope::Every;
        let (id, snapshot) = db.txns.take(scope).unwrap();
        let (first, twice) = db.scan_visible(&snapshot);
        assert!(twice.is_empty(), "{snapshot:?} sees keys twice: {twice:?}");
        for _ in 0..3 {
            pause(rt, 200 + rt.next_u64() % 2000);
            if every {
                let (again, _) = db.scan_visible(&snapshot);
                assert_eq!(first, again, "{snapshot:?} reads other rows by scan");
                let index = db.index_visible(&snapshot);
                assert_eq!(first, index, "{snapshot:?} reads other rows by index");
            }
        }
        db.txns.release(id);
        world.lock().unwrap().reads += 1;
    }
}

/// Vacuum while writers and readers run, and check the progress it shows.
fn vacuumer(
    rt: &SimRuntime,
    db: &Db,
    world: &Mutex<World>,
    stop: &AtomicBool,
    progress: &Progress,
) {
    while !stop.load(Ordering::Relaxed) {
        world.lock().unwrap().vacuuming = true;
        db.vacuum(64 + (rt.next_u64() % 2048) as usize, progress);
        let mut w = world.lock().unwrap();
        w.vacuums += 1;
        w.vacuuming = false;
        drop(w);
        assert!(
            progress.rows().is_empty(),
            "a finished vacuum shows progress"
        );
        pause(rt, 500 + rt.next_u64() % 3000);
    }
}

/// Watch the progress of a vacuum as it runs.
fn watcher(rt: &SimRuntime, progress: &Progress, stop: &AtomicBool, seen: &AtomicUsize) {
    while !stop.load(Ordering::Relaxed) {
        for step in progress.rows() {
            assert!((0.0..=1.0).contains(&step.done), "{step:?}");
            assert!(HEAP_IDS.contains(&step.heap), "{step:?}");
            if step.phase != Phase::Scan {
                seen.fetch_add(1, Ordering::Relaxed);
            }
        }
        rt.yield_now();
    }
}

/// Returns each writer's committed newest versions, read from a fresh snapshot.
fn live_of(db: &Db) -> BTreeMap<u64, Live> {
    let (id, s) = db.txns.take(Scope::Every).unwrap();
    let mut out: BTreeMap<u64, Live> = BTreeMap::new();
    for (h, heap) in db.heaps.iter().enumerate() {
        let mut versions = Vec::new();
        heap.scan(&db.pager, |v| versions.push(v)).expect("scan");
        for v in versions {
            if db
                .txns
                .sees(&db.pager, &s, None, v.header.xmin, v.header.xmax)
                .unwrap()
            {
                let k = key_of(&v.row);
                out.entry(k as u64 % WRITERS)
                    .or_default()
                    .insert(k, (h, v.tid, v.row));
            }
        }
    }
    db.txns.release(id);
    out
}

/// Run writers, readers and a vacuum together in one process for `steps` transactions each.
fn process(
    rt: SimRuntime,
    db: Arc<Db>,
    world: Arc<Mutex<World>>,
    steps: usize,
    progress: Arc<Progress>,
) {
    let stop = Arc::new(AtomicBool::new(false));
    let mut lives = live_of(&db);
    let writers: Vec<_> = (0..WRITERS)
        .map(|w| {
            let (rt2, db, world) = (rt.clone(), db.clone(), world.clone());
            let live = lives.remove(&w).unwrap_or_default();
            rt.spawn(&format!("writer {w}"), move || {
                writer(&rt2, &db, &world, w, live, steps);
            })
        })
        .collect();
    let others: Vec<_> = (0..3)
        .map(|i| {
            let (rt2, db, world, stop) = (rt.clone(), db.clone(), world.clone(), stop.clone());
            let progress = progress.clone();
            rt.spawn(&format!("other {i}"), move || match i {
                0 => vacuumer(&rt2, &db, &world, &stop, &progress),
                _ => reader(&rt2, &db, &world, &stop),
            })
        })
        .collect();
    for w in writers {
        rt.join(w);
    }
    stop.store(true, Ordering::Relaxed);
    for t in others {
        rt.join(t);
    }
}

/// Returns what is wrong with a quiet database: its committed rows by scan and by index against
/// the model, its all-visible bits, and the whole-file verify, before and after a last vacuum.
fn inspect(db: &Db, world: &World) -> Vec<String> {
    let mut problems = Vec::new();
    let mut committed = BTreeSet::new();
    for &x in world.ops.keys() {
        if db.txns.status(&db.pager, x).unwrap() == Status::Committed {
            committed.insert(x);
        }
    }
    for x in world.acked.iter().filter(|x| !committed.contains(x)) {
        problems.push(format!("acknowledged {x:?} did not commit"));
    }
    let expected = world.expected(|x| committed.contains(&x));
    for round in ["before", "after"] {
        problems.extend(
            db.verify_file()
                .into_iter()
                .map(|p| format!("{round}: {p}")),
        );
        let (scan, twice, index) = db.fresh();
        if scan != expected || !twice.is_empty() {
            problems.push(format!(
                "{round}: the scan disagrees with the committed rows"
            ));
        }
        if index != expected {
            problems.push(format!(
                "{round}: the index disagrees with the committed rows"
            ));
        }
        problems.extend(db.check_visibility_map());
        db.vacuum(usize::MAX, &Progress::new());
    }
    problems
}

#[test]
fn vacuum_runs_beside_writers_and_readers_holding_snapshots() {
    for seed in 0..4 {
        let mut sim = Sim::new(
            seed,
            Config {
                switch_rate: SWITCH_RATE,
                ..Config::default()
            },
        );
        let world = Arc::new(Mutex::new(World::default()));
        let w = world.clone();
        let outcome = sim.run(move |rt| {
            let (db, _) = Db::create(rt.clone());
            let db = Arc::new(db);
            let progress = Arc::new(Progress::new());
            let stop = Arc::new(AtomicBool::new(false));
            let seen = Arc::new(AtomicUsize::new(0));
            let watch = {
                let (rt2, p, s, n) = (rt.clone(), progress.clone(), stop.clone(), seen.clone());
                rt.spawn("watcher", move || watcher(&rt2, &p, &s, &n))
            };
            process(rt.clone(), db.clone(), w.clone(), 120, progress);
            stop.store(true, Ordering::Relaxed);
            rt.join(watch);
            let problems = inspect(&db, &w.lock().unwrap());
            assert!(problems.is_empty(), "seed {seed}: {problems:#?}");
            assert!(
                seen.load(Ordering::Relaxed) > 0,
                "seed {seed}: no progress past scanning was seen"
            );
        });
        assert_eq!(Outcome::Finished, outcome);
        let w = world.lock().unwrap();
        eprintln!(
            "seed {seed}: {} transactions, {} vacuums, {} snapshot reads",
            w.ops.len(),
            w.vacuums,
            w.reads
        );
        assert!(w.vacuums > 3 && w.reads > 3, "seed {seed}");
    }
}

#[expect(
    clippy::print_stderr,
    reason = "the crash loop reports its coverage on stderr"
)]
/// Returns the rounds that crashed and those that crashed while a vacuum ran.
fn crash_loop(seed: u64, faults: Faults, rounds: usize) -> (usize, usize) {
    silence_fatal_panics();
    let mut sim = Sim::new(
        seed,
        Config {
            switch_rate: SWITCH_RATE,
            ..Config::default()
        },
    );
    let world = Arc::new(Mutex::new(World::default()));
    let w = world.clone();
    finished(&mut sim, move |rt| {
        let (db, roots) = Db::create(rt);
        db.pager.checkpoint();
        w.lock().unwrap().roots = Some(roots);
    });
    let mut crashes = 0;
    for round in 0..rounds {
        sim.set_faults(faults.clone());
        let at = sim.now() + crash_after(seed, round as u64, 3000, 60_000);
        sim.crash_at(at);
        let w = world.clone();
        let outcome = run_process(&mut sim, move |rt| {
            let roots = w.lock().unwrap().roots.unwrap();
            let Some(db) = Db::open(rt.clone(), roots) else {
                return;
            };
            process(rt, Arc::new(db), w, 60, Arc::new(Progress::new()));
        });
        if outcome == Outcome::Crashed {
            crashes += 1;
            let mut w = world.lock().unwrap();
            w.crashed_vacuuming += usize::from(w.vacuuming);
        }
        world.lock().unwrap().vacuuming = false;
        sim.set_faults(Faults::default());
        let w = world.clone();
        let mut problems = Vec::new();
        finished(&mut sim, |rt| {
            let roots = w.lock().unwrap().roots.unwrap();
            let db = Db::open(rt, roots).expect("the database reopens");
            problems = inspect(&db, &w.lock().unwrap());
        });
        assert!(
            problems.is_empty(),
            "seed {seed} round {round}: {problems:#?}"
        );
    }
    let w = world.lock().unwrap();
    eprintln!(
        "seed {seed}: {crashes} crashes, {} during vacuum, {} transactions, {} acknowledged, {} vacuums",
        w.crashed_vacuuming,
        w.ops.len(),
        w.acked.len(),
        w.vacuums
    );
    (crashes, w.crashed_vacuuming)
}

#[test]
fn crashes_beside_concurrent_vacuum_recover_clean() {
    const SEEDS: Range<u64> = 0..6;
    let (crashes, during): (Vec<usize>, Vec<usize>) = sweep(SEEDS)
        .map(|seed| {
            crash_loop(
                seed,
                Faults {
                    torn_p: 0.5,
                    ..Faults::default()
                },
                3,
            )
        })
        .unzip();
    let (crashes, during): (usize, usize) = (crashes.iter().sum(), during.iter().sum());
    assert!(crashes > share(SEEDS, 8), "{crashes} crashes");
    assert!(
        during >= share(SEEDS, 3),
        "{during} of {crashes} crashes came during a vacuum"
    );
}

#[test]
fn crash_beside_vacuum_at_seed_13_recovers_clean() {
    crash_loop(
        13,
        Faults {
            torn_p: 0.5,
            ..Faults::default()
        },
        3,
    );
}
