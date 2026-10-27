use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use adtb::heap::{Fate, FieldType, Header, Heap, HeapNo, LargeStore, walk};
use adtb::key::{Scalar, Tid, Value};
use adtb::runtime::Threads;
use adtb::storage::{Options, PageError, PageKind, Pager, Pid};
use adtb_sim::{Config, Sim, SimRuntime};

#[path = "support/harness.rs"]
mod harness;

use harness::{PAGE, finished};

const WRITERS: usize = 4;
const ROWS: i64 = 480;
const FANOUT: u16 = 4;
const HEAP_ID: u32 = 10;
const LARGE_ID: u32 = 9;
const DELETER: u64 = 2;

fn options(pool_frames: usize) -> Options {
    Options {
        threads: WRITERS + 1,
        ..harness::options(pool_frames)
    }
}

fn row_type() -> Vec<FieldType> {
    vec![
        FieldType::Scalar(Scalar::I64),
        FieldType::Scalar(Scalar::Bytes),
    ]
}

fn row(key: i64, round: u8) -> Vec<Value> {
    vec![Value::I64(key), Value::Bytes(vec![round; 200])]
}

struct Db {
    pager: Pager<SimRuntime>,
    heap: Heap,
}

/// Fill a heap whose directory has three levels, then delete and prune every other row so each
/// page has room for a heap-only update of each row left. Returns the TIDs of the rows left.
fn build(rt: SimRuntime, pool_frames: usize) -> (Db, Vec<(i64, Tid)>) {
    let pager = Pager::create(rt, &options(pool_frames)).expect("create");
    let large = LargeStore::create(&pager, LARGE_ID).expect("large store");
    let heap = Heap::create_with_fanout(&pager, HEAP_ID, row_type(), large, FANOUT).unwrap();
    let mut kept = Vec::new();
    for key in 0..ROWS {
        let tid = heap.insert(&pager, 1, &row(key, 0)).expect("insert");
        if key % 2 == 0 {
            kept.push((key, tid));
        } else {
            heap.delete(&pager, tid, DELETER).expect("delete");
        }
    }
    let judge = |_: &Pager<SimRuntime>, h: &Header| {
        Ok(if h.xmax == DELETER {
            Fate::Remove
        } else {
            Fate::Keep
        })
    };
    for pid in heap.pages(&pager).unwrap() {
        let dead = heap.dead_slots(&pager, pid, judge).unwrap();
        heap.prune(&pager, pid, judge, &dead, false).unwrap();
    }
    assert!(
        heap.pages(&pager).unwrap().len() > usize::from(FANOUT).pow(2),
        "the directory has three levels"
    );
    (Db { pager, heap }, kept)
}

/// Counts of what happened while [`update_all`] ran.
struct Updates {
    /// Updates during which the directory root was latched exclusively or evicted.
    latched: usize,
    /// Updates begun while another writer's update was in flight.
    overlapped: usize,
}

/// Update every row of `kept` once, writer `w` taking the `w`th quarter of them, so the writers
/// work on pages under different directory leaves.
fn update_all(rt: &SimRuntime, db: &Arc<Db>, kept: &[(i64, Tid)], seed: u64) -> Updates {
    let latched = Arc::new(AtomicUsize::new(0));
    let in_flight = Arc::new(AtomicUsize::new(0));
    let overlapped = Arc::new(AtomicUsize::new(0));
    let share = kept.len().div_ceil(WRITERS);
    let writers: Vec<_> = kept
        .chunks(share)
        .enumerate()
        .map(|(w, rows)| {
            let (db, rows) = (db.clone(), rows.to_vec());
            let (latched, in_flight, overlapped) =
                (latched.clone(), in_flight.clone(), overlapped.clone());
            rt.spawn(&format!("writer {w}"), move || {
                for (key, tid) in rows {
                    let root = db.pager.optimistic(db.heap.root()).unwrap();
                    let xid = 10 + w as u64;
                    if in_flight.fetch_add(1, Ordering::Relaxed) > 0 {
                        overlapped.fetch_add(1, Ordering::Relaxed);
                    }
                    let u = db.heap.update(&db.pager, tid, xid, &row(key, 1), &[]);
                    in_flight.fetch_sub(1, Ordering::Relaxed);
                    let u = u.unwrap_or_else(|e| panic!("seed {seed}: update of row {key}: {e:?}"));
                    assert!(u.heap_only, "seed {seed}: row {key} moved off its page");
                    if !root.validate() {
                        latched.fetch_add(1, Ordering::Relaxed);
                    }
                }
            })
        })
        .collect();
    for w in writers {
        rt.join(w);
    }
    Updates {
        latched: latched.load(Ordering::Relaxed),
        overlapped: overlapped.load(Ordering::Relaxed),
    }
}

#[test]
fn heap_only_updates_from_many_threads_leave_the_directory_root_unlatched() {
    for seed in 0..4 {
        let mut sim = Sim::new(
            seed,
            Config {
                switch_rate: 0.3,
                ..Config::default()
            },
        );
        finished(&mut sim, |rt| {
            let (db, kept) = build(rt.clone(), 1024);
            let db = Arc::new(db);
            let pages = db.heap.pages(&db.pager).unwrap();
            let updates = update_all(&rt, &db, &kept, seed);
            eprintln!(
                "seed {seed}: {} of {} updates overlapped another",
                updates.overlapped,
                kept.len()
            );
            assert!(
                updates.overlapped > 100,
                "seed {seed}: {} updates overlapped another",
                updates.overlapped
            );
            assert_eq!(
                0, updates.latched,
                "seed {seed}: a write that adds no page latched the root"
            );
            assert_eq!(pages, db.heap.pages(&db.pager).unwrap(), "seed {seed}");
            assert_eq!(
                Vec::<String>::new(),
                db.heap.verify(&db.pager).unwrap(),
                "seed {seed}"
            );
            for (key, tid) in kept {
                let v = db.heap.read(&db.pager, tid).unwrap().unwrap();
                let next = v
                    .header
                    .next
                    .unwrap_or_else(|| panic!("seed {seed}: updated row {key} has no successor"));
                let after = db.heap.read(&db.pager, next).unwrap().unwrap();
                assert_eq!(row(key, 1), after.row, "seed {seed}: row {key}");
            }
        });
    }
}

/// The page ids one visibility map leaf of 4096-byte pages covers.
const VM_SPAN: u64 = (PAGE as u64 - 32) * 8;
const ROUNDS: usize = 40;

fn all_visible(_: &Pager<SimRuntime>, _: &Header) -> Result<Fate, PageError> {
    Ok(Fate::AllVisible)
}

/// Allocate pages until the next one free has an id past the first visibility map leaf, three in
/// each update. Returns them.
fn fill_low_ids(pager: &Pager<SimRuntime>) -> Vec<Pid> {
    let mut filler = Vec::new();
    while filler.last().is_none_or(|p: &Pid| p.0 < VM_SPAN + 64) {
        let mut new = pager.reserve();
        let pids: Vec<Pid> = (0..3)
            .map(|_| new.page(1, PageKind::HeapSlotted, 999).unwrap())
            .collect();
        pager.update_with(new, &pids, |_| {}).unwrap();
        filler.extend(pids);
    }
    filler
}

/// Make a heap whose visibility map root is a leaf, with all-visible pages below the leaf's span
/// and one page past it. Returns the heap, the rows on its all-visible pages and the page past
/// the span.
fn heap_across_the_span(
    pager: &Pager<SimRuntime>,
    large: LargeStore,
    id: u32,
    filler: &mut Vec<Pid>,
) -> (Heap, Vec<Tid>, Pid) {
    for _ in 0..8 {
        pager.free(filler.remove(0)).unwrap();
    }
    let heap = Heap::create(pager, id, row_type(), large).unwrap();
    let mut low = Vec::new();
    let high = loop {
        let t = heap.insert(pager, 1, &row(low.len() as i64, 0)).unwrap();
        if t.page() >= VM_SPAN {
            break Pid(t.page());
        }
        low.push(t);
    };
    for pid in heap.pages(pager).unwrap() {
        if pid != high {
            heap.prune(pager, pid, all_visible, &[], true).unwrap();
            assert!(heap.all_visible(pager, pid).unwrap());
        }
    }
    (heap, low, high)
}

#[test]
fn clearing_bits_while_the_visibility_map_root_grows_keeps_the_map_whole() {
    let mut sim = Sim::new(
        5,
        Config {
            switch_rate: 0.3,
            ..Config::default()
        },
    );
    finished(&mut sim, |rt| {
        let pager = Pager::create(
            rt.clone(),
            &Options {
                atomic_page_writes: true,
                segment_size: 4 << 20,
                ..options(64)
            },
        )
        .expect("create");
        let large = LargeStore::create(&pager, LARGE_ID).expect("large store");
        let mut filler = fill_low_ids(&pager);
        let mut overlapped = 0;
        for round in 0..ROUNDS {
            let (heap, low, high) =
                heap_across_the_span(&pager, large, 50 + round as u32, &mut filler);
            let heap = Arc::new(heap);
            let growing = Arc::new(AtomicBool::new(false));
            let during = Arc::new(AtomicUsize::new(0));
            let (p, h, g) = (pager.clone(), heap.clone(), growing.clone());
            let grower = rt.spawn("grower", move || {
                g.store(true, Ordering::Relaxed);
                h.prune(&p, high, all_visible, &[], true).unwrap();
                g.store(false, Ordering::Relaxed);
            });
            let (p, h, g, d) = (pager.clone(), heap.clone(), growing.clone(), during.clone());
            let clearer = rt.spawn("clearer", move || {
                for t in low {
                    let before = g.load(Ordering::Relaxed);
                    h.delete(&p, t, 3).expect("delete");
                    if before && g.load(Ordering::Relaxed) {
                        d.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
            rt.join(grower);
            rt.join(clearer);
            overlapped += usize::from(during.load(Ordering::Relaxed) > 0);
            assert_eq!(
                Vec::<String>::new(),
                heap.verify(&pager).unwrap(),
                "round {round}"
            );
            assert_eq!(vec![high], heap_marked(&pager, &heap), "round {round}");
        }
        eprintln!("{overlapped} of {ROUNDS} rounds cleared a bit while the root grew");
        assert!(
            overlapped > 10,
            "{overlapped} rounds cleared a bit while the root grew"
        );
    });
}

/// Returns the pages of `heap` the visibility map marks all-visible.
fn heap_marked(pager: &Pager<SimRuntime>, heap: &Heap) -> Vec<Pid> {
    heap.pages(pager)
        .unwrap()
        .into_iter()
        .filter(|&pid| heap.all_visible(pager, pid).unwrap())
        .collect()
}

const HOT_ROUNDS: u64 = 400;

fn dead_once_deleted(_: &Pager<SimRuntime>, h: &Header) -> Result<Fate, PageError> {
    Ok(if h.xmax == 0 {
        Fate::Keep
    } else {
        Fate::Remove
    })
}

#[test]
fn a_walk_beside_pruning_reaches_the_newest_version_of_its_row_and_no_other_row() {
    for seed in 0..4 {
        let mut sim = Sim::new(
            seed,
            Config {
                switch_rate: 0.3,
                ..Config::default()
            },
        );
        finished(&mut sim, |rt| {
            let pager = Pager::create(rt.clone(), &options(64)).expect("create");
            let large = LargeStore::create(&pager, LARGE_ID).expect("large store");
            let heap =
                Heap::create_with_fanout(&pager, HEAP_ID, row_type(), large, FANOUT).unwrap();
            let head = heap.insert(&pager, 2, &row(0, 0)).expect("insert");
            let db = Arc::new(Db { pager, heap });
            let stop = Arc::new(AtomicBool::new(false));
            let walks = Arc::new(AtomicUsize::new(0));
            let reader = {
                let (db, stop, walks) = (db.clone(), stop.clone(), walks.clone());
                rt.spawn("reader", move || {
                    while !stop.load(Ordering::Relaxed) {
                        let newest = walk(&db.pager, HeapNo(HEAP_ID), &[HeapNo(HEAP_ID)], head, |l| {
                            assert!(
                                l.header.xmin.is_multiple_of(2),
                                "seed {seed}: the walk from {head:?} reached {:?}, written by filler {}",
                                l.tid,
                                l.header.xmin
                            );
                            l.header.xmax == 0
                        })
                        .unwrap();
                        assert!(
                            newest.is_some(),
                            "seed {seed}: the walk from {head:?} ended before the newest version"
                        );
                        walks.fetch_add(1, Ordering::Relaxed);
                    }
                })
            };
            let page = Pid(head.page());
            let mut at = head;
            for r in 1..=HOT_ROUNDS {
                let u = db
                    .heap
                    .update(&db.pager, at, 2 * r + 2, &row(0, r as u8), &[0])
                    .unwrap();
                assert!(
                    u.heap_only,
                    "seed {seed}: round {r} moved the row off its page"
                );
                at = u.tid;
                db.heap
                    .prune(&db.pager, page, dead_once_deleted, &[], false)
                    .unwrap();
                let filler = db.heap.insert(&db.pager, 2 * r + 3, &row(1, 0)).unwrap();
                db.heap.delete(&db.pager, filler, 2 * r + 3).unwrap();
                rt.yield_now();
            }
            stop.store(true, Ordering::Relaxed);
            rt.join(reader);
            assert!(
                walks.load(Ordering::Relaxed) > 100,
                "seed {seed}: {} walks",
                walks.load(Ordering::Relaxed)
            );
        });
    }
}

const KEPT_LARGE: usize = 60;

fn large_row(key: i64) -> Vec<Value> {
    vec![Value::I64(key), Value::Bytes(vec![key as u8; 6000])]
}

#[test]
fn large_values_written_beside_a_sweep_freeing_store_pages_stay_whole() {
    for seed in 0..4 {
        let mut sim = Sim::new(
            seed,
            Config {
                switch_rate: 0.3,
                ..Config::default()
            },
        );
        finished(&mut sim, |rt| {
            let pager = Pager::create(rt.clone(), &options(256)).expect("create");
            let large = LargeStore::create(&pager, LARGE_ID).expect("large store");
            let heap =
                Heap::create_with_fanout(&pager, HEAP_ID, row_type(), large, FANOUT).unwrap();
            let db = Arc::new(Db { pager, heap });
            let stop = Arc::new(AtomicBool::new(false));
            let freed = Arc::new(AtomicUsize::new(0));
            let churner = {
                let (db, stop, freed) = (db.clone(), stop.clone(), freed.clone());
                rt.spawn("churner", move || {
                    let mut xid = 1;
                    while !stop.load(Ordering::Relaxed) {
                        let tid = db.heap.insert(&db.pager, xid, &large_row(-1)).unwrap();
                        db.heap.delete(&db.pager, tid, xid).unwrap();
                        db.heap
                            .prune(&db.pager, Pid(tid.page()), dead_once_deleted, &[], false)
                            .unwrap();
                        let n = large
                            .sweep(&db.pager, |x| Ok(!x.is_multiple_of(2)))
                            .unwrap();
                        freed.fetch_add(n, Ordering::Relaxed);
                        xid += 2;
                    }
                })
            };
            let kept: Vec<(i64, Tid)> = (0..KEPT_LARGE as i64)
                .map(|key| {
                    let tid = db.heap.insert(&db.pager, 2, &large_row(key)).unwrap();
                    rt.yield_now();
                    (key, tid)
                })
                .collect();
            stop.store(true, Ordering::Relaxed);
            rt.join(churner);
            assert!(
                freed.load(Ordering::Relaxed) > KEPT_LARGE,
                "seed {seed}: the sweep freed {} chunks",
                freed.load(Ordering::Relaxed)
            );
            for (key, tid) in kept {
                let v = db.heap.read(&db.pager, tid).unwrap().unwrap();
                assert_eq!(large_row(key), v.row, "seed {seed}: row {key}");
            }
        });
    }
}
