use std::collections::{BTreeMap, BTreeSet};
use std::ops::{Bound, Range};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::btree::Index;
use crate::heap::{FieldType, Flags, Heap, HeapNo, walk};
use crate::key::{CtorId, Dir, Entry, KeySchema, KeyType, Scalar, Tid, Value};
use crate::mvcc::{Scope, Snapshot, Status, Transactions, Xid};
use crate::runtime::{Entropy, Threads};
use crate::storage::{Pager, Pid};
use crate::vacuum::{self, Phase, Progress, Sum};
use adtb_sim::{Config, Ledger, Outcome, Sim, SimRuntime};
use adtb_sys::runtime::Faults;

use crate::test_world as harness;
use crate::test_world::rows;
use adtb_harness::crash;
use adtb_harness::seeds;

use crate::test_world::fixture::{Layout, Parts, Storage};
use crash::{Reopen, Rounds, crash_loop};
use harness::{finished, options};
use rows::{APPLE, Live, Op, Rows, key_of, replay};
use seeds::{share, sweep};

const FRAMES: usize = 96;

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

/// One database shared by every thread of a process.
struct Db {
    pager: Pager<SimRuntime>,
    txns: Transactions<SimRuntime>,
    heaps: Vec<Heap>,
    index: Index,
    ids: [HeapNo; 3],
}

impl Db {
    fn create(rt: &SimRuntime) -> (Db, Layout) {
        let mut s = Storage::create(rt, &options(FRAMES));
        s.commit_log(rt);
        let large = s.large();
        for h in 0..3 {
            s.heap(row_type(h), large, 4);
        }
        s.index(schema(), ctors());
        s.pager().flush_log();
        let layout = s.layout();
        (Db::of(s), layout)
    }

    fn of(s: Storage) -> Db {
        let (pager, txns, heaps, mut indexes) = s.split();
        let ids = [0, 1, 2].map(|h| heaps[h].id());
        Db {
            pager,
            txns: txns.expect("the world's commit log"),
            heaps,
            index: indexes.pop().expect("the world's index"),
            ids,
        }
    }

    fn sum(&self) -> Sum<'_> {
        Sum {
            heaps: (0..3).map(|h| (CtorId(h as u32), &self.heaps[h])).collect(),
            indexes: vec![&self.index],
        }
    }

    fn vacuum(&self, budget: usize, progress: &Progress) {
        vacuum::vacuum(
            &self.pager,
            &self.txns,
            &self.sum(),
            budget,
            progress,
            &mut || true,
        )
        .expect("vacuum");
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
                    .reader(&self.pager, snapshot.clone(), None)
                    .sees(&v.header.writers)
                    .expect("sees")
                    .visible()
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
            walk(&self.pager, self.ids[h], &self.ids, e.tid, |l| {
                if l.tid.page() != e.tid.page() || !first && !l.flags.contains(Flags::HEAP_ONLY) {
                    return true;
                }
                first = false;
                seen.push((l.tid, l.header.writers));
                false
            })
            .expect("walk");
            for (t, writers) in seen {
                if self
                    .txns
                    .reader(&self.pager, snapshot.clone(), None)
                    .sees(&writers)
                    .expect("sees")
                    .visible()
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
                    .reader(&self.pager, fresh.clone(), None)
                    .sees(&v.header.writers)
                    .unwrap()
                    .visible();
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
        Parts::new(
            &self.pager,
            Some(&self.txns),
            &self.heaps,
            std::slice::from_ref(&self.index),
            &[],
        )
        .verify()
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
            db.heaps[heap].delete(&db.pager, tid, xid).expect("delete");
        }
        Op::Retag { key, to, row } => {
            let (heap, tid, _) = live[key].clone();
            let t = db.heaps[heap]
                .retag(&db.pager, tid, xid, &db.heaps[*to], row)
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
    roots: Option<Layout>,
    /// Each transaction's operations, by the writer that ran it.
    ops: BTreeMap<Xid, Vec<Op>>,
    /// Where each commit is recorded as submitted before it is made.
    ledger: Ledger,
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
            world.lock().unwrap().ledger.submit(xid.0);
            db.txns.commit(&db.pager, xid).expect("commit");
            live = mine;
            unacked.push(xid);
        }
        if rt.next_u64().is_multiple_of(4) {
            db.pager.flush_log();
            let ledger = world.lock().unwrap().ledger.clone();
            for x in unacked.drain(..) {
                ledger.acknowledge(x.0);
            }
        }
    }
    db.pager.flush_log();
    let ledger = world.lock().unwrap().ledger.clone();
    for x in unacked {
        ledger.acknowledge(x.0);
    }
}

/// Hold snapshots across vacuums and check each reads the same rows by scan and by index every
/// time.
fn reader(rt: &SimRuntime, db: &Db, world: &Mutex<World>, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        let scope = if rt.next_u64().is_multiple_of(2) {
            Scope::Every
        } else {
            Scope::Heaps(BTreeSet::from([db.ids[(rt.next_u64() % 3) as usize].0]))
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
            progress.rows().iter().all(|s| s.phase == Phase::Done),
            "a finished vacuum shows every heap done"
        );
        pause(rt, 500 + rt.next_u64() % 3000);
    }
}

/// Watch the progress of a vacuum as it runs.
fn watcher(
    rt: &SimRuntime,
    ids: [HeapNo; 3],
    progress: &Progress,
    stop: &AtomicBool,
    seen: &AtomicUsize,
) {
    while !stop.load(Ordering::Relaxed) {
        for step in progress.rows() {
            assert!((0.0..=1.0).contains(&step.done), "{step:?}");
            assert!(ids.contains(&HeapNo(step.heap)), "{step:?}");
            if !matches!(step.phase, Phase::Scan | Phase::Done) {
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
                .reader(&db.pager, s.clone(), None)
                .sees(&v.header.writers)
                .unwrap()
                .visible()
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
fn inspect(db: &Db, world: &World) -> Reopen {
    let mut problems = Vec::new();
    let mut committed = BTreeSet::new();
    for &x in world.ops.keys() {
        if db.txns.status(&db.pager, x).unwrap() == Status::Committed {
            committed.insert(x);
        }
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
    Reopen::new(committed.iter().map(|x| x.0).collect(), problems)
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
            let (db, _) = Db::create(&rt);
            let ids = db.ids;
            let db = Arc::new(db);
            let progress = Arc::new(Progress::new());
            let stop = Arc::new(AtomicBool::new(false));
            let seen = Arc::new(AtomicUsize::new(0));
            let watch = {
                let (rt2, p, s, n) = (rt.clone(), progress.clone(), stop.clone(), seen.clone());
                rt.spawn("watcher", move || watcher(&rt2, ids, &p, &s, &n))
            };
            process(rt.clone(), db.clone(), w.clone(), 120, progress);
            stop.store(true, Ordering::Relaxed);
            rt.join(watch);
            let problems = inspect(&db, &w.lock().unwrap()).problems().to_vec();
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

/// Returns the rounds that crashed and those that crashed while a vacuum ran.
#[must_use]
fn crash_rounds(seed: u64, faults: Faults, rounds: usize) -> (usize, usize) {
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
        let (db, roots) = Db::create(&rt);
        db.pager.checkpoint();
        w.lock().unwrap().roots = Some(roots);
    });
    let rounds = Rounds::new(rounds, 3000, 60_000);
    let out = crash_loop(
        &mut sim,
        seed,
        faults,
        rounds,
        |r| {
            world.lock().unwrap().ledger = r.ledger().clone();
            let w = world.clone();
            let crashed = r.crash(move |rt| {
                let roots = w.lock().unwrap().roots.clone().unwrap();
                let Some(db) = Storage::reopen(&rt, &roots).ok().map(Db::of) else {
                    return;
                };
                process(rt, Arc::new(db), w, 60, Arc::new(Progress::new()));
            });
            let mut w = world.lock().unwrap();
            if crashed {
                w.crashed_vacuuming += usize::from(w.vacuuming);
            }
            w.vacuuming = false;
        },
        |rt, _| {
            let roots = world.lock().unwrap().roots.clone().unwrap();
            let db = Db::of(Storage::reopen(&rt, &roots).expect("the database reopens"));
            inspect(&db, &world.lock().unwrap())
        },
    );
    let crashed_vacuuming = world.lock().unwrap().crashed_vacuuming;
    (out.crashed(), crashed_vacuuming)
}

#[test]
fn crashes_beside_concurrent_vacuum_recover_clean() {
    const SEEDS: Range<u64> = 0..6;
    let (crashes, during): (Vec<usize>, Vec<usize>) = sweep(SEEDS)
        .map(|seed| {
            crash_rounds(
                seed,
                Faults {
                    torn_p: 0.5,
                    ..Faults::none(512)
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
    let (crashes, _) = crash_rounds(
        13,
        Faults {
            torn_p: 0.5,
            ..Faults::none(512)
        },
        3,
    );
    seeds::assert_reached("a crash at seed 13", crashes, 1);
}
