use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ops::{Bound, Range};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};

use adtb::btree::{BTree, Entries};
use adtb::runtime::{Entropy, Threads};
use adtb::storage::{PageKind, Pager, Pid, kind};
use adtb_sim::{Config, Faults, Outcome, Recording, Sim, SimRuntime};

#[path = "support/crash.rs"]
#[expect(dead_code, reason = "this binary injects no failed writes")]
mod crash;
#[path = "support/draw.rs"]
mod draw;
#[path = "support/harness.rs"]
mod harness;
#[path = "support/seeds.rs"]
mod seeds;

use crash::crash_after;
use draw::below;
use harness::{PAGE, finished, options};
use seeds::{share, sweep};

const OWNER: u32 = 41;
const SCAN: usize = 24;

const SWITCH_RATE: f64 = 0.01;
const CRASH_SWITCH_RATE: f64 = 0.3;

fn switching() -> Config {
    Config {
        switch_rate: SWITCH_RATE,
        ..Config::default()
    }
}

/// Returns the entry of key `id`: its big-endian bytes, so entries order as
/// keys do, padded to a length that varies by key.
fn entry(id: u32) -> Vec<u8> {
    let mut e = id.to_be_bytes().to_vec();
    e.resize(4 + (id as usize * 37) % 600, id as u8);
    e
}

fn id_of(entry: &[u8]) -> u32 {
    u32::from_be_bytes(entry[..4].try_into().unwrap())
}

/// How the entries of a run are built from key ids.
#[derive(Clone, Copy, Debug)]
enum Shape {
    /// Whole, as [`entry`] builds them.
    Whole,
    /// Sixteen ids to a four-byte key, each a remainder of that key, so that
    /// leaves keep postings.
    Grouped,
}

impl Shape {
    fn encode(self, id: u32) -> Vec<u8> {
        match self {
            Shape::Whole => entry(id),
            Shape::Grouped => {
                let mut e = (id / 16).to_be_bytes().to_vec();
                e.extend_from_slice(&id.to_be_bytes());
                e.resize(44, id as u8);
                e
            }
        }
    }

    fn decode(self, e: &[u8]) -> u32 {
        match self {
            Shape::Whole => id_of(e),
            Shape::Grouped => u32::from_be_bytes(e[4..8].try_into().unwrap()),
        }
    }
}

impl Entries for Shape {
    fn dead(&mut self, _: &[u8]) -> bool {
        false
    }

    fn key_len(&self, _: &[u8]) -> Option<usize> {
        match self {
            Shape::Whole => None,
            Shape::Grouped => Some(4),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Insert,
    Delete,
    Contains,
}

/// One operation on one key: what it did, what it returned, and the ticks of
/// its call and its return.
#[derive(Clone, Copy, Debug)]
struct Event {
    kind: Kind,
    result: bool,
    call: u64,
    ret: u64,
}

/// What the threads record, shared among them.
#[derive(Default)]
struct History {
    tick: AtomicU64,
    events: Mutex<BTreeMap<u32, Vec<Event>>>,
}

impl History {
    fn now(&self) -> u64 {
        self.tick.fetch_add(1, Relaxed)
    }

    fn record(&self, id: u32, kind: Kind, result: bool, call: u64, ret: u64) {
        let event = Event {
            kind,
            result,
            call,
            ret,
        };
        self.events
            .lock()
            .unwrap()
            .entry(id)
            .or_default()
            .push(event);
    }
}

/// Returns whether the events on one key have a serial order, consistent with
/// their calls and returns, in which each returns what a set would.
fn linearizable(events: &[Event]) -> bool {
    fn search(
        events: &[Event],
        done: &mut Vec<bool>,
        present: bool,
        seen: &mut HashSet<(Vec<bool>, bool)>,
    ) -> bool {
        if done.iter().all(|&d| d) {
            return true;
        }
        if !seen.insert((done.clone(), present)) {
            return false;
        }
        let deadline = (0..events.len())
            .filter(|&i| !done[i])
            .map(|i| events[i].ret)
            .min()
            .unwrap();
        for i in 0..events.len() {
            let e = events[i];
            if done[i] || e.call > deadline {
                continue;
            }
            let next = match e.kind {
                Kind::Insert if e.result != present => true,
                Kind::Delete if e.result == present => false,
                Kind::Contains if e.result == present => present,
                _ => continue,
            };
            done[i] = true;
            if search(events, done, next, seen) {
                return true;
            }
            done[i] = false;
        }
        false
    }
    search(
        events,
        &mut vec![false; events.len()],
        false,
        &mut HashSet::new(),
    )
}

/// How a worker picks the keys it changes.
#[derive(Clone, Copy, Debug)]
enum Keys {
    /// One key at a time, most of them among the first `hot`.
    Hot(u32),
    /// Runs of this many consecutive keys, inserted or deleted together, so
    /// that leaves fill and empty.
    Runs(u32),
}

/// What each worker of a run does.
#[derive(Clone, Copy, Debug)]
struct Work {
    keys: u32,
    how: Keys,
    shape: Shape,
    steps: usize,
}

/// Run random operations over the keys of `work`, recording each in `history`.
fn worker(rt: &SimRuntime, pager: &Pager<SimRuntime>, tree: BTree, history: &History, work: Work) {
    let Work {
        keys,
        how,
        mut shape,
        steps,
    } = work;
    for _ in 0..steps {
        let (id, run) = match how {
            Keys::Hot(hot) if rt.next_u64().is_multiple_of(2) => {
                (below(rt, hot as usize) as u32, 1)
            }
            Keys::Hot(_) => (below(rt, keys as usize) as u32, 1),
            Keys::Runs(n) => (below(rt, (keys / n) as usize) as u32 * n, n),
        };
        let e = shape.encode(id);
        let call = history.now();
        match below(rt, 10) {
            0..=4 => {
                for k in id..id + run {
                    let call = history.now();
                    let r = tree
                        .insert_with(pager, &shape.encode(k), &mut shape)
                        .expect("insert");
                    history.record(k, Kind::Insert, r, call, history.now());
                }
            }
            5 | 6 => {
                for k in id..id + run {
                    let call = history.now();
                    let r = tree.delete(pager, &shape.encode(k)).expect("delete");
                    history.record(k, Kind::Delete, r, call, history.now());
                }
            }
            7 => {
                let r = tree.contains(pager, &e).expect("contains");
                history.record(id, Kind::Contains, r, call, history.now());
            }
            _ => {
                let mut cursor = tree.range(Bound::Included(&e), Bound::Unbounded);
                let mut found = BTreeSet::new();
                let mut last = keys - 1;
                while let Some(x) = cursor.next(pager).expect("scan") {
                    found.insert(shape.decode(&x));
                    if found.len() == SCAN {
                        last = shape.decode(&x);
                        break;
                    }
                }
                let ret = history.now();
                for k in id..=last {
                    history.record(k, Kind::Contains, found.contains(&k), call, ret);
                }
            }
        }
    }
}

/// Run `threads` workers over one tree and check every key's history, then
/// the tree's structure and contents, and that every page allocated is a
/// page of the tree. Returns the events recorded and the pairs of events on
/// one key that overlapped in time.
fn concurrent_run(
    seed: u64,
    threads: usize,
    steps: usize,
    pool: usize,
    how: Keys,
    shape: Shape,
) -> (usize, usize) {
    let mut sim = Sim::new(seed, switching());
    let history = Arc::new(History::default());
    let h = history.clone();
    let keys = 1500;
    finished(&mut sim, move |rt| {
        let pager = Pager::create(rt.clone(), &options(pool)).expect("create");
        let tree = BTree::create(&pager, OWNER).expect("create tree");
        let workers: Vec<_> = (0..threads)
            .map(|i| {
                let (pager, rt2, h) = (pager.clone(), rt.clone(), h.clone());
                rt.spawn(&format!("worker {i}"), move || {
                    let work = Work {
                        keys,
                        how,
                        shape,
                        steps,
                    };
                    worker(&rt2, &pager, tree, &h, work);
                })
            })
            .collect();
        for w in workers {
            rt.join(w);
        }
        let entries = tree.verify(&pager).expect("verify");
        assert_eq!(tree.repair(&pager), Ok(0));
        let call = h.now();
        let present: BTreeSet<u32> = entries.iter().map(|e| shape.decode(e)).collect();
        for id in 0..keys {
            h.record(id, Kind::Contains, present.contains(&id), call, call);
        }
        assert!(tree.height(&pager).unwrap() >= 1);
        let pages: BTreeSet<Pid> = tree.pages(&pager).unwrap().into_iter().collect();
        let allocated: BTreeSet<Pid> = pager.allocated().unwrap().into_iter().collect();
        assert_eq!(
            allocated, pages,
            "seed {seed}: pages allocated outside the tree"
        );
    });
    let events = history.events.lock().unwrap();
    let mut total = 0;
    let mut overlaps = 0;
    for (id, evs) in events.iter() {
        assert!(
            linearizable(evs),
            "seed {seed}: no serial order explains key {id}: {evs:?}"
        );
        total += evs.len();
        for (i, a) in evs.iter().enumerate() {
            overlaps += evs[i + 1..]
                .iter()
                .filter(|b| a.call < b.ret && b.call < a.ret)
                .count();
        }
    }
    (total, overlaps)
}

#[test]
fn concurrent_operations_are_linearizable_by_key() {
    const SEEDS: Range<u64> = 0..3;
    let mut overlaps = 0;
    for seed in sweep(SEEDS) {
        overlaps += concurrent_run(seed, 6, 500, 32, Keys::Hot(48), Shape::Whole).1;
    }
    assert!(
        overlaps >= share(SEEDS, 100),
        "only {overlaps} overlapping pairs"
    );
}

#[test]
fn leaves_emptied_and_refilled_by_many_threads_stay_linearizable_by_key() {
    const SEEDS: Range<u64> = 0..3;
    let mut overlaps = 0;
    for seed in sweep(SEEDS) {
        overlaps += concurrent_run(seed, 5, 120, 32, Keys::Runs(16), Shape::Whole).1;
    }
    assert!(
        overlaps >= share(SEEDS, 100),
        "only {overlaps} overlapping pairs"
    );
}

#[test]
fn postings_changed_by_many_threads_stay_linearizable_by_entry() {
    const SEEDS: Range<u64> = 0..3;
    let mut overlaps = 0;
    for seed in sweep(SEEDS) {
        overlaps += concurrent_run(seed, 5, 120, 32, Keys::Runs(16), Shape::Grouped).1;
        overlaps += concurrent_run(seed + 10, 5, 400, 32, Keys::Hot(48), Shape::Grouped).1;
    }
    assert!(
        overlaps >= share(SEEDS, 100),
        "only {overlaps} overlapping pairs"
    );
}

#[test]
fn concurrent_operations_through_a_roomy_pool_are_linearizable_by_key() {
    concurrent_run(7, 4, 600, 512, Keys::Hot(48), Shape::Whole);
}

#[test]
fn the_checker_refuses_a_history_no_serial_order_explains() {
    let e = |kind, result, call, ret| Event {
        kind,
        result,
        call,
        ret,
    };
    let ok = [
        e(Kind::Insert, true, 0, 5),
        e(Kind::Contains, false, 1, 2),
        e(Kind::Contains, true, 3, 4),
    ];
    assert!(linearizable(&ok));
    let stale = [e(Kind::Insert, true, 0, 1), e(Kind::Contains, false, 2, 3)];
    assert!(!linearizable(&stale));
    let twice = [e(Kind::Insert, true, 0, 3), e(Kind::Insert, true, 1, 2)];
    assert!(!linearizable(&twice));
}

/// What the clients know, kept outside the simulated process so it survives
/// crashes. Each thread owns the keys whose top byte is its number.
#[derive(Default)]
struct World {
    root: Option<Pid>,
    /// Each thread's operations begun, an insert when true, in order.
    ops: Vec<Vec<(bool, u32)>>,
    /// Every operation of a thread before this one is durable.
    durable: Vec<usize>,
    /// The recorded events before and after each insert of the last run.
    spans: Vec<(usize, usize)>,
    /// Each thread's operations made durable in the last run, after how many recorded events.
    forced: Vec<Vec<(usize, usize)>>,
}

const THREADS: usize = 4;
const SEEK: usize = 32;

fn crash_worker(
    rt: &SimRuntime,
    pager: &Pager<SimRuntime>,
    world: &Mutex<World>,
    t: usize,
    mut shape: Shape,
) {
    let tree = BTree::open(world.lock().unwrap().root.unwrap(), OWNER);
    for _ in 0..80 {
        let insert = below(rt, 10) < 6;
        let first = ((t as u32) << 24) | (below(rt, 25) as u32 * 16);
        for id in first..first + 16 {
            world.lock().unwrap().ops[t].push((insert, id));
            let e = shape.encode(id);
            if insert {
                let start = rt.recorded();
                tree.insert_with(pager, &e, &mut shape).expect("insert");
                world.lock().unwrap().spans.push((start, rt.recorded()));
            } else {
                tree.delete(pager, &e).expect("delete");
            }
        }
        match below(rt, 30) {
            0..=2 => {
                pager.flush_log();
                let after = rt.recorded();
                let mut w = world.lock().unwrap();
                let len = w.ops[t].len();
                w.durable[t] = len;
                w.forced[t].push((after, len));
            }
            3 => {
                pager.flush_pages(1 + below(rt, 4));
            }
            4 => pager.checkpoint(),
            _ => {}
        }
    }
}

fn crash_run(rt: SimRuntime, world: &Arc<Mutex<World>>, shape: Shape) {
    let (pager, _) = Pager::open(rt.clone(), &options(24)).expect("open");
    let workers: Vec<_> = (0..THREADS)
        .map(|t| {
            let (pager, rt2, w) = (pager.clone(), rt.clone(), world.clone());
            rt.spawn(&format!("worker {t}"), move || {
                crash_worker(&rt2, &pager, &w, t, shape);
            })
        })
        .collect();
    let done = Arc::new(AtomicBool::new(false));
    let (p, stop, r) = (pager, done.clone(), rt.clone());
    let flusher = rt.spawn("flusher", move || {
        while !stop.load(Relaxed) {
            p.flush_log();
            r.yield_now();
        }
    });
    for w in workers {
        rt.join(w);
    }
    done.store(true, Relaxed);
    rt.join(flusher);
}

/// Returns the length of the prefix of `ops`, at least `from` long, after
/// which a set holds exactly `found`.
fn matching_prefix(ops: &[(bool, u32)], from: usize, found: &BTreeSet<u32>) -> Option<usize> {
    let mut model = BTreeSet::new();
    for &(insert, id) in &ops[..from] {
        if insert {
            model.insert(id);
        } else {
            model.remove(&id);
        }
    }
    for k in from..=ops.len() {
        if &model == found {
            return Some(k);
        }
        if let Some(&(insert, id)) = ops.get(k) {
            if insert {
                model.insert(id);
            } else {
                model.remove(&id);
            }
        }
    }
    None
}

fn crash_config() -> Config {
    Config {
        faults: Faults {
            torn_p: 0.5,
            ..Faults::default()
        },
        switch_rate: CRASH_SWITCH_RATE,
        ..Config::default()
    }
}

/// Returns the number of splits incomplete in the forced state after the
/// first `at` events of `recording`.
fn incomplete_at(recording: &Recording, at: usize, root: Pid, seed: u64) -> usize {
    let files = recording.images(at, 1, seed).swap_remove(0);
    let mut sim = Sim::with_files(seed, Config::default(), files);
    let mut n = 0;
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options(24)).expect("reopen");
        n = BTree::open(root, OWNER).repair(&pager).expect("repair");
    });
    n
}

/// Run the threads to their end, recording them, and replace `sim` by the
/// forced state at a log force inside an insert that leaves a split
/// incomplete, sought among [`SEEK`] such forces spread back from the end of
/// the run, or at the last force inside an insert when none does.
fn crash_at_a_flush(sim: &mut Sim, world: &Arc<Mutex<World>>, seed: u64, shape: Shape) {
    let durable = {
        let mut w = world.lock().unwrap();
        w.spans.clear();
        w.forced.iter_mut().for_each(Vec::clear);
        w.durable.clone()
    };
    sim.record();
    let w = world.clone();
    finished(sim, move |rt| crash_run(rt, &w, shape));
    let recording = sim.recording();
    let flushes = recording.flushes();
    let mut w = world.lock().unwrap();
    let root = w.root.unwrap();
    let inside: Vec<usize> = flushes
        .iter()
        .rev()
        .copied()
        .filter(|&f| w.spans.iter().any(|&(s, e)| s < f && f < e))
        .collect();
    let at = inside
        .iter()
        .step_by(inside.len().div_ceil(SEEK).max(1))
        .find(|&&f| incomplete_at(&recording, f, root, seed) > 0)
        .or(inside.first())
        .or(flushes.last())
        .copied()
        .expect("the threads force the log");
    let files = recording.images(at, 1, seed).swap_remove(0);
    *sim = Sim::with_files(seed, crash_config(), files);
    w.durable = w
        .forced
        .iter()
        .zip(durable)
        .map(|(marks, before)| {
            marks
                .iter()
                .rev()
                .find(|&&(after, _)| after <= at)
                .map_or(before, |&(_, len)| len)
        })
        .collect();
}

/// Returns the rounds that crashed of `rounds` runs crashed at a time, and
/// the rounds whose crash left a split for repair to finish, of those and of
/// `cuts` runs cut at a log force inside an insert.
fn threaded_crash_loop(seed: u64, rounds: usize, cuts: usize, shape: Shape) -> (usize, usize) {
    let mut sim = Sim::new(seed, crash_config());
    let world = Arc::new(Mutex::new(World {
        ops: vec![Vec::new(); THREADS],
        durable: vec![0; THREADS],
        forced: vec![Vec::new(); THREADS],
        ..World::default()
    }));
    let w = world.clone();
    finished(&mut sim, move |rt| {
        let pager = Pager::create(rt, &options(24)).expect("create");
        let tree = BTree::create(&pager, OWNER).expect("create tree");
        pager.flush_log();
        w.lock().unwrap().root = Some(tree.root());
    });
    let (mut crashes, mut repaired) = (0, 0);
    for round in 0..rounds + cuts {
        let cut = round % 2 == 1 && round < 2 * cuts;
        if !cut {
            let at = sim.now() + crash_after(seed, round as u64, 2_000, 150_000);
            sim.crash_at(at);
            let w = world.clone();
            if sim.run(move |rt| crash_run(rt, &w, shape)) == Outcome::Crashed {
                crashes += 1;
            }
        } else {
            crash_at_a_flush(&mut sim, &world, seed * 1000 + round as u64, shape);
        }
        let root = world.lock().unwrap().root.unwrap();
        let mut found = None;
        finished(&mut sim, |rt| {
            let (pager, _) = Pager::open(rt, &options(24)).expect("reopen");
            let tree = BTree::open(root, OWNER);
            let entries = tree
                .verify(&pager)
                .unwrap_or_else(|e| panic!("seed {seed} round {round}: {e:?}"));
            repaired += usize::from(tree.repair(&pager).expect("repair") > 0);
            assert_eq!(tree.repair(&pager), Ok(0), "seed {seed} round {round}");
            assert_eq!(tree.verify(&pager).as_ref(), Ok(&entries));
            let pages: BTreeSet<Pid> = tree.pages(&pager).unwrap().into_iter().collect();
            let allocated: BTreeSet<Pid> = pager.allocated().unwrap().into_iter().collect();
            assert_eq!(allocated, pages, "seed {seed} round {round}: pages leaked");
            pager.flush_log();
            found = Some(entries);
        });
        let ids: Vec<u32> = found.unwrap().iter().map(|e| shape.decode(e)).collect();
        let mut w = world.lock().unwrap();
        for t in 0..THREADS {
            let mine: BTreeSet<u32> = ids
                .iter()
                .copied()
                .filter(|id| (id >> 24) as usize == t)
                .collect();
            let kept = matching_prefix(&w.ops[t], w.durable[t], &mine).unwrap_or_else(|| {
                panic!(
                    "seed {seed} round {round} thread {t}: {} entries match no prefix of {} operations from {}",
                    mine.len(),
                    w.ops[t].len(),
                    w.durable[t]
                )
            });
            w.ops[t].truncate(kept);
            w.durable[t] = kept;
        }
    }
    (crashes, repaired)
}

#[test]
fn threads_crashed_together_recover_a_prefix_of_each_threads_operations() {
    const SEEDS: Range<u64> = 0..4;
    let (mut crashes, mut repaired) = (0, 0);
    for seed in sweep(SEEDS) {
        let shape = if seed % 2 == 0 {
            Shape::Whole
        } else {
            Shape::Grouped
        };
        let (c, r) = threaded_crash_loop(seed, 5, 2, shape);
        crashes += c;
        repaired += r;
    }
    eprintln!("{crashes} crashes, {repaired} left a split to finish");
    assert!(crashes >= share(SEEDS, 5), "only {crashes} crashes");
    assert!(
        repaired >= share(SEEDS, 2),
        "only {repaired} crashes left a split incomplete"
    );
}

/// Fill a tree with the even keys below `2 * n`, then run `retain` dropping
/// the even multiples of three beside threads inserting the odd keys, which
/// split the leaves the walk passes.
fn retain_beside_inserts(seed: u64, n: u32, inserters: u32) {
    let mut sim = Sim::new(seed, switching());
    finished(&mut sim, move |rt| {
        let pager = Pager::create(rt.clone(), &options(32)).expect("create");
        let tree = BTree::create(&pager, OWNER).expect("create tree");
        for k in 0..n {
            assert_eq!(tree.insert(&pager, &entry(2 * k)), Ok(true));
        }
        let dropped = |id: u32| id.is_multiple_of(2) && id.is_multiple_of(3);
        let inserted = Arc::new(AtomicU64::new(0));
        let mut threads: Vec<_> = (0..inserters)
            .map(|t| {
                let (pager, rt2, inserted) = (pager.clone(), rt.clone(), inserted.clone());
                rt.spawn(&format!("inserter {t}"), move || {
                    for k in (t..n).step_by(inserters as usize) {
                        let id = 2 * k + 1;
                        assert_eq!(tree.insert(&pager, &entry(id)), Ok(true));
                        inserted.fetch_add(1, Relaxed);
                        if below(&rt2, 4) == 0 {
                            rt2.yield_now();
                        }
                    }
                })
            })
            .collect();
        let deleted = Arc::new(Mutex::new((0, 0)));
        let (p, d, i) = (pager.clone(), deleted.clone(), inserted);
        threads.push(rt.spawn("retain", move || {
            let before = i.load(Relaxed);
            let n = tree.retain(&p, |e| !dropped(id_of(e))).expect("retain");
            *d.lock().unwrap() = (n, i.load(Relaxed) - before);
        }));
        for t in threads {
            rt.join(t);
        }
        let expected: Vec<Vec<u8>> = (0..2 * n).filter(|&id| !dropped(id)).map(entry).collect();
        assert_eq!(tree.verify(&pager).as_ref(), Ok(&expected), "seed {seed}");
        assert_eq!(tree.repair(&pager), Ok(0));
        let (deleted, during) = *deleted.lock().unwrap();
        assert_eq!(deleted, (0..n).filter(|k| dropped(2 * k)).count());
        assert!(during > 0, "seed {seed}: no insert ran during the walk");
    });
}

#[test]
fn retain_beside_concurrent_inserts_judges_every_entry_present_before_it() {
    for seed in 0..4 {
        retain_beside_inserts(seed, 600, 3);
    }
}

/// Returns the number of leaves among the tree's pages.
fn leaves(pager: &Pager<SimRuntime>, tree: &BTree) -> usize {
    tree.pages(pager)
        .unwrap()
        .into_iter()
        .filter(|&p| kind(&pager.read(p).unwrap()) == PageKind::IndexLeaf as u16)
        .count()
}

fn with_tree(seed: u64, body: impl FnOnce(&Pager<SimRuntime>, BTree) + Send) {
    let mut sim = Sim::new(seed, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt, &options(64)).expect("create");
        let tree = BTree::create(&pager, OWNER).expect("create tree");
        body(&pager, tree);
    });
}

fn allocated(pager: &Pager<SimRuntime>) -> BTreeSet<Pid> {
    pager.allocated().unwrap().into_iter().collect()
}

#[test]
fn ascending_inserts_fill_the_leaves_they_leave_behind() {
    with_tree(30, |pager, tree| {
        let n = 3000;
        for id in 0..n {
            assert_eq!(tree.insert(pager, &entry(id)), Ok(true));
        }
        let bytes: usize = (0..n).map(|id| entry(id).len() + 4).sum();
        let full = bytes.div_ceil(PAGE - 64);
        let got = leaves(pager, &tree);
        assert!(got * 10 <= full * 12, "{got} leaves hold what {full} could");
        let all: Vec<Vec<u8>> = (0..n).map(entry).collect();
        assert_eq!(tree.verify(pager), Ok(all));
    });
}

#[test]
fn emptied_leaves_are_unlinked_freed_and_reused() {
    with_tree(31, |pager, tree| {
        let n = 3000;
        for id in (0..n).rev() {
            assert_eq!(tree.insert(pager, &entry(id)), Ok(true));
        }
        let full = leaves(pager, &tree);
        let before = allocated(pager);
        for id in 0..n {
            assert_eq!(tree.delete(pager, &entry(id)), Ok(true));
        }
        assert_eq!(tree.verify(pager), Ok(Vec::new()));
        let pages: BTreeSet<Pid> = tree.pages(pager).unwrap().into_iter().collect();
        assert_eq!(allocated(pager), pages);
        let parents = pages.len() - leaves(pager, &tree);
        assert!(
            leaves(pager, &tree) <= parents,
            "{} leaves of {full} left under {parents} parents",
            leaves(pager, &tree)
        );
        for id in (0..n).rev() {
            assert_eq!(tree.insert(pager, &entry(id)), Ok(true));
        }
        let after = allocated(pager);
        let fresh = after.difference(&before).count();
        assert!(
            fresh <= 2,
            "{fresh} of {} pages were never freed ones",
            after.len()
        );
        assert_eq!(tree.verify(pager).unwrap().len(), n as usize);
    });
}

#[test]
fn retain_unlinks_the_leaves_it_empties() {
    with_tree(32, |pager, tree| {
        let n = 2000;
        for id in 0..n {
            assert_eq!(tree.insert(pager, &entry(id)), Ok(true));
        }
        let full = leaves(pager, &tree);
        let dropped = |id: u32| (200..1800).contains(&id);
        let deleted = tree.retain(pager, |e| !dropped(id_of(e))).unwrap();
        assert_eq!(deleted, 1600);
        let kept: Vec<Vec<u8>> = (0..n).filter(|&id| !dropped(id)).map(entry).collect();
        assert_eq!(tree.verify(pager), Ok(kept));
        let pages: BTreeSet<Pid> = tree.pages(pager).unwrap().into_iter().collect();
        assert_eq!(allocated(pager), pages);
        assert!(
            leaves(pager, &tree) * 3 < full,
            "{} of {full} leaves left",
            leaves(pager, &tree)
        );
    });
}

/// Entries kept whole, judged dead by a closure.
struct Dead<F>(F);

impl<F: FnMut(&[u8]) -> bool> Entries for Dead<F> {
    fn dead(&mut self, entry: &[u8]) -> bool {
        (self.0)(entry)
    }

    fn key_len(&self, _: &[u8]) -> Option<usize> {
        None
    }
}

#[test]
fn an_insert_into_a_full_leaf_deletes_dead_entries_before_splitting() {
    with_tree(33, |pager, tree| {
        let fixed = |id: u32, tail: u8| {
            let mut e = id.to_be_bytes().to_vec();
            e.resize(200, tail);
            e
        };
        let mut id = 0;
        while leaves(pager, &tree) == 1 {
            tree.insert(pager, &fixed(id, 0)).unwrap();
            id += 1;
        }
        let pages = tree.pages(pager).unwrap().len();
        let dead = |e: &[u8]| e[199] == 0 && id_of(e).is_multiple_of(2);
        for k in 0..5 {
            let e = fixed(2 * k + 1, 1);
            assert_eq!(tree.insert_with(pager, &e, &mut Dead(dead)), Ok(true));
        }
        assert_eq!(tree.pages(pager).unwrap().len(), pages);
        let left = tree.verify(pager).unwrap();
        let expected: Vec<Vec<u8>> = (0..id)
            .flat_map(|i| [fixed(i, 0), fixed(i, 1)])
            .filter(|e| left.contains(e))
            .collect();
        assert_eq!(left, expected);
        assert!(!left.contains(&fixed(0, 0)));
        assert!(left.contains(&fixed(1, 0)));
        for k in 0..5 {
            assert_eq!(tree.contains(pager, &fixed(2 * k + 1, 1)), Ok(true));
        }
    });
}

#[test]
fn a_parents_first_child_is_unlinked_when_it_empties() {
    with_tree(34, |pager, tree| {
        let n = 400;
        for id in 0..n {
            assert_eq!(tree.insert(pager, &entry(id)), Ok(true));
        }
        assert_eq!(tree.height(pager), Ok(1));
        assert!(leaves(pager, &tree) > 20);
        for id in 0..n - 1 {
            assert_eq!(tree.delete(pager, &entry(id)), Ok(true));
        }
        assert_eq!(tree.verify(pager), Ok(vec![entry(n - 1)]));
        assert_eq!(leaves(pager, &tree), 1);
        let pages: BTreeSet<Pid> = tree.pages(pager).unwrap().into_iter().collect();
        assert_eq!(allocated(pager), pages);
    });
}

#[test]
fn first_children_unlink_across_parents() {
    with_tree(35, |pager, tree| {
        let long = |id: u32| {
            let mut e = vec![7; 300];
            e.extend_from_slice(&id.to_be_bytes());
            e
        };
        let n = 3000;
        for id in 0..n {
            assert_eq!(tree.insert(pager, &long(id)), Ok(true));
        }
        assert!(tree.height(pager).unwrap() >= 2);
        for id in 0..n - 1 {
            assert_eq!(tree.delete(pager, &long(id)), Ok(true));
        }
        assert_eq!(tree.verify(pager), Ok(vec![long(n - 1)]));
        let pages: BTreeSet<Pid> = tree.pages(pager).unwrap().into_iter().collect();
        assert_eq!(allocated(pager), pages);
        let inner = pages.len() - leaves(pager, &tree);
        assert!(
            leaves(pager, &tree) < inner,
            "{} leaves under {inner} inner pages",
            leaves(pager, &tree)
        );
    });
}
