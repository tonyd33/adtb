use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ops::Bound;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adtb::btree::BTree;
use adtb::runtime::{Entropy, Threads};
use adtb::storage::{Options, Pager, Pid};
use adtb_sim::{Config, Faults, Outcome, Sim, SimRuntime};

const PAGE: usize = 4096;
const OWNER: u32 = 41;
const NAME: &str = "app.adt";
const SCAN: usize = 24;

fn options(pool_frames: usize) -> Options {
    Options {
        page_size: PAGE as u32,
        log_block: 512,
        segment_size: 256 * 1024,
        pool_frames,
        ..Options::new(NAME)
    }
}

fn below(rt: &SimRuntime, n: u64) -> u64 {
    rt.next_u64() % n
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

/// Run random operations over keys below `keys`, recording each in `history`.
///
/// Most operations go to the first `hot` keys, so threads meet on keys as well
/// as on pages.
fn worker(
    rt: &SimRuntime,
    pager: &Pager<SimRuntime>,
    tree: BTree,
    history: &History,
    keys: u32,
    hot: u32,
    steps: usize,
) {
    for _ in 0..steps {
        let id = if rt.next_u64().is_multiple_of(2) {
            below(rt, u64::from(hot)) as u32
        } else {
            below(rt, u64::from(keys)) as u32
        };
        let e = entry(id);
        let call = history.now();
        match below(rt, 10) {
            0..=4 => {
                let r = tree.insert(pager, &e).expect("insert");
                history.record(id, Kind::Insert, r, call, history.now());
            }
            5 | 6 => {
                let r = tree.delete(pager, &e).expect("delete");
                history.record(id, Kind::Delete, r, call, history.now());
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
                    found.insert(id_of(&x));
                    if found.len() == SCAN {
                        last = id_of(&x);
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
/// the tree's structure and contents. Returns the events recorded and the
/// pairs of events on one key that overlapped in time.
fn concurrent_run(seed: u64, threads: usize, steps: usize, pool: usize) -> (usize, usize) {
    let mut sim = Sim::new(seed, Config::default());
    let history = Arc::new(History::default());
    let h = history.clone();
    let (keys, hot) = (1500, 48);
    let outcome = sim.run(move |rt| {
        let pager = Arc::new(Pager::create(rt.clone(), &options(pool)).expect("create"));
        let tree = BTree::create(&pager, OWNER).expect("create tree");
        let workers: Vec<_> = (0..threads)
            .map(|i| {
                let (pager, rt2, h) = (pager.clone(), rt.clone(), h.clone());
                rt.spawn(&format!("worker {i}"), move || {
                    worker(&rt2, &pager, tree, &h, keys, hot, steps)
                })
            })
            .collect();
        for w in workers {
            rt.join(w);
        }
        let entries = tree.verify(&pager).expect("verify");
        assert_eq!(tree.repair(&pager), Ok(0));
        let call = h.now();
        let present: BTreeSet<u32> = entries.iter().map(|e| id_of(e)).collect();
        for id in 0..keys {
            h.record(id, Kind::Contains, present.contains(&id), call, call);
        }
        assert!(tree.height(&pager).unwrap() >= 2);
    });
    assert_eq!(Outcome::Finished, outcome);
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
    let mut overlaps = 0;
    for seed in 0..3 {
        overlaps += concurrent_run(seed, 6, 500, 32).1;
    }
    assert!(overlaps >= 100, "only {overlaps} overlapping pairs");
}

#[test]
fn concurrent_operations_through_a_roomy_pool_are_linearizable_by_key() {
    concurrent_run(7, 4, 600, 512);
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
/// crashes. Each thread owns the keys whose low byte is its number.
#[derive(Default)]
struct World {
    root: Option<Pid>,
    /// Each thread's operations begun, an insert when true, in order.
    ops: Vec<Vec<(bool, u32)>>,
    /// Every operation of a thread before this one is durable.
    durable: Vec<usize>,
}

const THREADS: usize = 4;

fn crash_worker(rt: &SimRuntime, pager: &Pager<SimRuntime>, world: &Mutex<World>, t: usize) {
    let tree = BTree::open(world.lock().unwrap().root.unwrap(), OWNER);
    for _ in 0..200 {
        let insert = below(rt, 10) < 7;
        let id = (below(rt, 400) as u32) << 8 | t as u32;
        world.lock().unwrap().ops[t].push((insert, id));
        if insert {
            tree.insert(pager, &entry(id)).expect("insert");
        } else {
            tree.delete(pager, &entry(id)).expect("delete");
        }
        match below(rt, 30) {
            0..=2 => {
                pager.flush_log();
                let mut w = world.lock().unwrap();
                w.durable[t] = w.ops[t].len();
            }
            3 => {
                pager.flush_pages(1 + below(rt, 4) as usize);
            }
            4 => pager.checkpoint(),
            _ => {}
        }
    }
}

fn crash_run(rt: SimRuntime, world: &Arc<Mutex<World>>) {
    let (pager, _) = Pager::open(rt.clone(), &options(20)).expect("open");
    let pager = Arc::new(pager);
    let workers: Vec<_> = (0..THREADS)
        .map(|t| {
            let (pager, rt2, w) = (pager.clone(), rt.clone(), world.clone());
            rt.spawn(&format!("worker {t}"), move || {
                crash_worker(&rt2, &pager, &w, t)
            })
        })
        .collect();
    for w in workers {
        rt.join(w);
    }
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

/// Returns the crashes of `rounds` crashed runs of several threads.
fn threaded_crash_loop(seed: u64, rounds: usize) -> usize {
    let mut sim = Sim::new(
        seed,
        Config {
            faults: Faults {
                torn_p: 0.5,
                ..Faults::default()
            },
            ..Config::default()
        },
    );
    let world = Arc::new(Mutex::new(World {
        ops: vec![Vec::new(); THREADS],
        durable: vec![0; THREADS],
        ..World::default()
    }));
    let w = world.clone();
    let outcome = sim.run(move |rt| {
        let pager = Pager::create(rt, &options(20)).expect("create");
        let tree = BTree::create(&pager, OWNER).expect("create tree");
        pager.flush_log();
        w.lock().unwrap().root = Some(tree.root());
    });
    assert_eq!(Outcome::Finished, outcome);
    let mut crashes = 0;
    for round in 0..rounds {
        let at = sim.now()
            + Duration::from_micros(2_000 + (seed * 7919 + round as u64 * 104_729) % 150_000);
        sim.crash_at(at);
        let w = world.clone();
        if sim.run(move |rt| crash_run(rt, &w)) == Outcome::Crashed {
            crashes += 1;
        }
        let root = world.lock().unwrap().root.unwrap();
        let mut found = None;
        let outcome = sim.run(|rt| {
            let (pager, _) = Pager::open(rt, &options(20)).expect("reopen");
            let tree = BTree::open(root, OWNER);
            let entries = tree
                .verify(&pager)
                .unwrap_or_else(|e| panic!("seed {seed} round {round}: {e:?}"));
            tree.repair(&pager).expect("repair");
            assert_eq!(tree.repair(&pager), Ok(0), "seed {seed} round {round}");
            assert_eq!(tree.verify(&pager).as_ref(), Ok(&entries));
            pager.flush_log();
            found = Some(entries);
        });
        assert_eq!(Outcome::Finished, outcome);
        let ids: Vec<u32> = found.unwrap().iter().map(|e| id_of(e)).collect();
        let mut w = world.lock().unwrap();
        for t in 0..THREADS {
            let mine: BTreeSet<u32> = ids
                .iter()
                .copied()
                .filter(|id| (id & 0xFF) as usize == t)
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
    crashes
}

#[test]
fn threads_crashed_together_recover_a_prefix_of_each_threads_operations() {
    let crashes: usize = (0..4).map(|seed| threaded_crash_loop(seed, 5)).sum();
    assert!(crashes >= 10, "only {crashes} crashes");
}

/// Fill a tree with the even keys below `2 * n`, then run `retain` dropping
/// the even multiples of three beside threads inserting the odd keys, which
/// split the leaves the walk passes.
fn retain_beside_inserts(seed: u64, n: u32, inserters: u32) {
    let mut sim = Sim::new(seed, Config::default());
    let outcome = sim.run(move |rt| {
        let pager = Arc::new(Pager::create(rt.clone(), &options(32)).expect("create"));
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
        let (p, d, i) = (pager.clone(), deleted.clone(), inserted.clone());
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
    assert_eq!(Outcome::Finished, outcome);
}

#[test]
fn retain_beside_concurrent_inserts_judges_every_entry_present_before_it() {
    for seed in 0..4 {
        retain_beside_inserts(seed, 600, 3);
    }
}
