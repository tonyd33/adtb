use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use adtb::heap::{Fate, FieldType, Header, Heap, LargeStore};
use adtb::key::{Scalar, Tid, Value};
use adtb::runtime::Threads;
use adtb::storage::{Options, PageError, PageKind, Pager, Pid};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};

const PAGE: usize = 4096;
const WRITERS: usize = 4;
const ROWS: i64 = 480;
const FANOUT: u16 = 4;
const HEAP_ID: u32 = 10;
const LARGE_ID: u32 = 9;
const DELETER: u64 = 2;

fn options(pool_frames: usize) -> Options {
    Options {
        page_size: PAGE as u32,
        log_block: 512,
        segment_size: 256 * 1024,
        pool_frames,
        threads: WRITERS + 1,
        atomic_page_writes: false,
        ..Options::new("app.adt")
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

/// Update every row of `kept` once, writer `w` taking the `w`th quarter of them, so the writers
/// work on pages under different directory leaves. Returns the number of updates during which
/// the directory root was latched exclusively or evicted.
fn update_all(rt: &SimRuntime, db: &Arc<Db>, kept: &[(i64, Tid)]) -> usize {
    let latched = Arc::new(AtomicUsize::new(0));
    let share = kept.len().div_ceil(WRITERS);
    let writers: Vec<_> = kept
        .chunks(share)
        .enumerate()
        .map(|(w, rows)| {
            let (db, rows, latched) = (db.clone(), rows.to_vec(), latched.clone());
            rt.spawn(&format!("writer {w}"), move || {
                for (key, tid) in rows {
                    let root = db.pager.optimistic(db.heap.root()).unwrap();
                    let xid = 10 + w as u64;
                    let u = db.heap.update(&db.pager, tid, xid, &row(key, 1), &[]);
                    assert!(u.expect("update").heap_only, "row {key} moved off its page");
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
    latched.load(Ordering::Relaxed)
}

fn finished(sim: &mut Sim, body: impl FnOnce(SimRuntime)) {
    assert_eq!(Outcome::Finished, sim.run(body));
}

#[test]
fn heap_only_updates_from_many_threads_leave_the_directory_root_unlatched() {
    for seed in 0..4 {
        let mut sim = Sim::new(seed, Config::default());
        finished(&mut sim, |rt| {
            let (db, kept) = build(rt.clone(), 1024);
            let db = Arc::new(db);
            let pages = db.heap.pages(&db.pager).unwrap();
            let latched = update_all(&rt, &db, &kept);
            assert_eq!(
                0, latched,
                "seed {seed}: a write that adds no page latched the root"
            );
            assert_eq!(pages, db.heap.pages(&db.pager).unwrap());
            assert_eq!(Vec::<String>::new(), db.heap.verify(&db.pager).unwrap());
            for (key, tid) in kept {
                let v = db.heap.read(&db.pager, tid).unwrap().unwrap();
                let next = v.header.next.expect("an updated row has a successor");
                let after = db.heap.read(&db.pager, next).unwrap().unwrap();
                assert_eq!(row(key, 1), after.row);
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
        let pager = Arc::new(pager);
        for round in 0..ROUNDS {
            let (heap, low, high) =
                heap_across_the_span(&pager, large, 50 + round as u32, &mut filler);
            let heap = Arc::new(heap);
            let (p, h) = (pager.clone(), heap.clone());
            let grower = rt.spawn("grower", move || {
                h.prune(&p, high, all_visible, &[], true).unwrap();
            });
            let (p, h) = (pager.clone(), heap.clone());
            let clearer = rt.spawn("clearer", move || {
                for t in low {
                    h.delete(&p, t, 3).expect("delete");
                }
            });
            rt.join(grower);
            rt.join(clearer);
            assert_eq!(
                Vec::<String>::new(),
                heap.verify(&pager).unwrap(),
                "round {round}"
            );
            assert_eq!(vec![high], heap_marked(&pager, &heap), "round {round}");
        }
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
