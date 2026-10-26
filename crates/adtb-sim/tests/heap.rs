use std::collections::{BTreeMap, BTreeSet};
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use adtb::batch::{Batch, BatchBuilder, Column, Schema};
use adtb::heap::{CtorType, Error, FieldType, Flags, Heap, LargeStore, Link, Updated, walk};
use adtb::key::{CtorId, Scalar, Tid, Value};
use adtb::runtime::Entropy;
use adtb::storage::{self, Options, PageKind, Pager, Pid};
use adtb::verify::{Inventory, Sum};
use adtb_sim::{Config, Faults, Outcome, Sim, SimRuntime};

mod common;

const PAGE: usize = 4096;
const NAME: &str = "app.adt";
const FATAL: &str = "fatal I/O error";

const EMAIL: usize = 0;
const GOOGLE: usize = 1;
const APPLE: usize = 2;
const HEAP_IDS: [u32; 3] = [10, 11, 12];
const LARGE_ID: u32 = 9;

fn options() -> Options {
    Options {
        page_size: PAGE as u32,
        log_block: 512,
        segment_size: 256 * 1024,
        pool_frames: 32,
        atomic_page_writes: false,
        ..Options::new(NAME)
    }
}

fn quiet() -> Faults {
    Faults {
        lose_unsynced: true,
        delay_completions: true,
        ..Faults::default()
    }
}

fn scalar(s: Scalar) -> FieldType {
    FieldType::Scalar(s)
}

fn row_type(heap: usize) -> Vec<FieldType> {
    match heap {
        APPLE => vec![scalar(Scalar::I64), scalar(Scalar::I32)],
        _ => vec![
            scalar(Scalar::I64),
            scalar(Scalar::Text),
            scalar(Scalar::Bytes),
        ],
    }
}

fn blob(r: u64) -> Vec<u8> {
    let len = if r.is_multiple_of(10) {
        2000 + (r % 8000) as usize
    } else {
        (r % 200) as usize
    };
    (0..len).map(|i| (r as usize + i * 31) as u8).collect()
}

fn make_row(heap: usize, key: i64, r: u64) -> Vec<Value> {
    match heap {
        APPLE => vec![Value::I64(key), Value::I32(r as i32)],
        _ => vec![
            Value::I64(key),
            Value::Text(format!("{key}-{}", r % 1000)),
            Value::Bytes(blob(r)),
        ],
    }
}

fn key_of(row: &[Value]) -> i64 {
    match row[0] {
        Value::I64(k) => k,
        ref v => panic!("key field holds {v:?}"),
    }
}

#[derive(Clone, Copy, Debug)]
struct Roots {
    large: Pid,
    heaps: [Pid; 3],
}

struct Db {
    pager: Pager<SimRuntime>,
    heaps: Vec<Heap>,
}

impl Db {
    fn create(rt: SimRuntime, fanout: u16) -> (Db, Roots) {
        let pager = Pager::create(rt, &options()).expect("create");
        let large = LargeStore::create(&pager, LARGE_ID).expect("large store");
        let heaps: Vec<Heap> = (0..3)
            .map(|h| {
                Heap::create_with_fanout(&pager, HEAP_IDS[h], row_type(h), large, fanout)
                    .expect("heap")
            })
            .collect();
        let roots = Roots {
            large: large.root(),
            heaps: [heaps[0].root(), heaps[1].root(), heaps[2].root()],
        };
        pager.flush_log();
        (Db { pager, heaps }, roots)
    }

    fn open(rt: SimRuntime, roots: Roots) -> Option<Db> {
        let (pager, _) = Pager::open(rt, &options()).ok()?;
        let large = LargeStore::open(LARGE_ID, roots.large);
        let heaps = (0..3)
            .map(|h| Heap::open(&pager, HEAP_IDS[h], roots.heaps[h], row_type(h), large).unwrap())
            .collect();
        Some(Db { pager, heaps })
    }
}

fn pair(heaps: &mut [Heap], a: usize, b: usize) -> (&mut Heap, &mut Heap) {
    assert_ne!(a, b);
    if a < b {
        let (l, r) = heaps.split_at_mut(b);
        (&mut l[a], &mut r[0])
    } else {
        let (l, r) = heaps.split_at_mut(a);
        (&mut r[0], &mut l[b])
    }
}

#[derive(Clone, Debug)]
enum Op {
    Insert {
        key: i64,
        heap: usize,
        row: Vec<Value>,
    },
    Update {
        key: i64,
        row: Vec<Value>,
    },
    Delete {
        key: i64,
    },
    Retag {
        key: i64,
        to: usize,
        row: Vec<Value>,
    },
}

/// Each key's heap, the TID of its live version and its row.
type Live = BTreeMap<i64, (usize, Tid, Vec<Value>)>;

fn generate(next: &mut impl FnMut() -> u64, live: &Live, next_key: &mut i64) -> Op {
    let r = next();
    if live.is_empty() || r % 10 < 4 {
        let key = *next_key;
        *next_key += 1;
        let heap = (next() % 3) as usize;
        return Op::Insert {
            key,
            heap,
            row: make_row(heap, key, next()),
        };
    }
    let (&key, (heap, _, old)) = live
        .iter()
        .nth((next() % live.len() as u64) as usize)
        .unwrap();
    match r % 10 {
        4..=6 => Op::Update {
            key,
            row: make_row(*heap, key, next()),
        },
        7 => Op::Delete { key },
        _ => {
            let to = (heap + 1 + (next() % 2) as usize) % 3;
            let mut row = make_row(to, key, next());
            if to != APPLE && *heap != APPLE && !next().is_multiple_of(3) {
                row[2] = old[2].clone();
            }
            Op::Retag { key, to, row }
        }
    }
}

fn perform(db: &mut Db, live: &mut Live, op: &Op, xid: u64) -> Result<Option<Updated>, Error> {
    let pager = &mut db.pager;
    match op {
        Op::Insert { key, heap, row } => {
            let tid = db.heaps[*heap].insert(pager, xid, row)?;
            live.insert(*key, (*heap, tid, row.clone()));
            Ok(None)
        }
        Op::Update { key, row } => {
            let (heap, tid, _) = live[key].clone();
            let u = db.heaps[heap].update(pager, tid, xid, row, &[0])?;
            live.insert(*key, (heap, u.tid, row.clone()));
            Ok(Some(u))
        }
        Op::Delete { key } => {
            let (heap, tid, _) = live.remove(key).unwrap();
            db.heaps[heap].delete(pager, tid, xid)?;
            Ok(None)
        }
        Op::Retag { key, to, row } => {
            let (heap, tid, _) = live[key].clone();
            let (from, into) = pair(&mut db.heaps, heap, *to);
            let t = from.retag(pager, tid, xid, into, row)?;
            live.insert(*key, (*to, t, row.clone()));
            Ok(None)
        }
    }
}

fn replay(state: &mut BTreeMap<i64, (usize, Vec<Value>)>, op: &Op) {
    match op {
        Op::Insert { key, heap, row } => {
            state.insert(*key, (*heap, row.clone()));
        }
        Op::Update { key, row } => state.get_mut(key).unwrap().1 = row.clone(),
        Op::Delete { key } => {
            state.remove(key);
        }
        Op::Retag { key, to, row } => {
            state.insert(*key, (*to, row.clone()));
        }
    }
}

/// Returns the live version of every key, taking as committed the xids `committed` accepts, and
/// any key found live twice.
fn current(db: &mut Db, committed: impl Fn(u64) -> bool) -> (Live, Vec<String>) {
    let mut out = Live::new();
    let mut problems = Vec::new();
    for (h, heap) in db.heaps.iter().enumerate() {
        heap.scan(&db.pager, |v| {
            let deleted = v.header.xmax != 0 && committed(v.header.xmax);
            if committed(v.header.xmin) && !deleted {
                let key = key_of(&v.row);
                if let Some(other) = out.insert(key, (h, v.tid, v.row)) {
                    problems.push(format!(
                        "key {key} is live at {:?} and {:?}",
                        other.1, v.tid
                    ));
                }
            }
        })
        .expect("scan");
    }
    (out, problems)
}

/// Returns what is wrong with the flags and successors of every version.
fn check_links(db: &mut Db) -> Vec<String> {
    let mut problems = Vec::new();
    for heap in &db.heaps {
        let mut versions = Vec::new();
        heap.scan(&db.pager, |v| versions.push(v)).expect("scan");
        for v in versions {
            let next = v
                .header
                .next
                .map(|n| walk(&db.pager, n, |_| true).unwrap().unwrap());
            let (moved, hot) = (
                v.flags.contains(Flags::MOVED),
                v.flags.contains(Flags::HOT_UPDATED),
            );
            let ok = match next {
                None => !moved && !hot,
                Some(n) if moved => n.heap != heap.id() && n.flags.contains(Flags::RETAGGED_IN),
                Some(n) if hot => {
                    n.tid.page() == v.tid.page() && n.flags.contains(Flags::HEAP_ONLY)
                }
                Some(n) => n.heap == heap.id() && !n.flags.contains(Flags::HEAP_ONLY),
            };
            if !ok {
                problems.push(format!("{:?} flagged {:?} names {next:?}", v.tid, v.flags));
            }
        }
    }
    problems
}

fn verify(db: &mut Db) -> Vec<String> {
    let mut problems = Vec::new();
    for heap in &db.heaps {
        problems.extend(heap.verify(&db.pager).expect("verify"));
    }
    problems
}

fn rows(live: &Live) -> BTreeMap<i64, (usize, Vec<Value>)> {
    live.iter()
        .map(|(k, (h, _, r))| (*k, (*h, r.clone())))
        .collect()
}

fn finished(sim: &mut Sim, body: impl FnOnce(SimRuntime)) {
    assert_eq!(Outcome::Finished, sim.run(body));
}

#[test]
fn inserts_updates_deletes_and_retags_match_a_model() {
    for seed in 0..3 {
        let mut sim = Sim::new(seed, Config::default());
        finished(&mut sim, |rt| {
            let (mut db, _) = Db::create(rt.clone(), 4);
            let mut live = Live::new();
            let mut first: BTreeMap<i64, Tid> = BTreeMap::new();
            let mut next_key = 0;
            let mut hot = 0;
            let mut cold = 0;
            let mut next = || rt.next_u64();
            for xid in 1..=1500 {
                let op = generate(&mut next, &live, &mut next_key);
                match perform(&mut db, &mut live, &op, xid).expect("perform") {
                    Some(u) if u.heap_only => hot += 1,
                    Some(_) => cold += 1,
                    None => {}
                }
                if let Op::Insert { key, .. } = op {
                    first.insert(key, live[&key].1);
                }
                db.pager.commit(xid, &[], |_| {}).unwrap();
                if xid % 250 == 0 {
                    let (found, dupes) = current(&mut db, |_| true);
                    assert_eq!(Vec::<String>::new(), dupes);
                    assert_eq!(rows(&live), rows(&found), "seed {seed} xid {xid}");
                    assert_eq!(live, found);
                    assert_eq!(Vec::<String>::new(), verify(&mut db));
                    assert_eq!(Vec::<String>::new(), check_links(&mut db));
                }
            }
            for (key, (heap, tid, _)) in &live {
                let end = walk(&db.pager, first[key], |l| l.header.next.is_none())
                    .unwrap()
                    .expect("chain ends");
                assert_eq!((HEAP_IDS[*heap], *tid), (end.heap, end.tid), "key {key}");
            }
            assert!(
                hot > 0 && cold > 0,
                "{hot} heap-only updates, {cold} others"
            );
            let pages = db.heaps[EMAIL].pages(&db.pager).unwrap().len();
            assert!(pages > 4, "{pages} pages, so one directory level");
        });
    }
}

#[test]
fn a_chain_runs_across_pages_and_through_a_retag_and_back() {
    let mut sim = Sim::new(1, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(rt, 4);
        let pager = &mut db.pager;
        let row = |n: usize| {
            vec![
                Value::I64(1),
                Value::Text("a".repeat(n)),
                Value::Bytes(vec![7; n]),
            ]
        };
        let first = db.heaps[EMAIL].insert(pager, 1, &row(1)).unwrap();
        let mut at = first;
        let mut pages = BTreeSet::new();
        for i in 0..30 {
            at = db.heaps[EMAIL]
                .update(pager, at, 2 + i, &row(10 + 15 * i as usize), &[0])
                .unwrap()
                .tid;
            pages.insert(at.page());
        }
        assert!(pages.len() > 3, "{pages:?}");
        let (email, google) = pair(&mut db.heaps, EMAIL, GOOGLE);
        let there = email.retag(pager, at, 50, google, &row(5)).unwrap();
        let back = google.retag(pager, there, 50, email, &row(6)).unwrap();
        let mut links: Vec<Link> = Vec::new();
        assert_eq!(
            None,
            walk(pager, first, |l| {
                links.push(*l);
                false
            })
            .unwrap()
        );
        assert_eq!(33, links.len());
        let tail = &links[30..];
        assert_eq!(at, tail[0].tid);
        assert_eq!(Some(there), tail[0].header.next);
        assert!(tail[0].flags.contains(Flags::MOVED));
        assert_eq!(50, tail[0].header.xmax);
        assert_eq!((there, HEAP_IDS[GOOGLE]), (tail[1].tid, tail[1].heap));
        assert!(
            tail[1]
                .flags
                .contains(Flags::MOVED.with(Flags::RETAGGED_IN))
        );
        assert_eq!(Some(back), tail[1].header.next);
        assert_eq!((back, HEAP_IDS[EMAIL]), (tail[2].tid, tail[2].heap));
        assert_eq!(Flags::RETAGGED_IN, tail[2].flags);
        assert_eq!((50, 0, None), {
            let h = tail[2].header;
            (h.xmin, h.xmax, h.next)
        });
        let read = db.heaps[EMAIL].read(pager, back).unwrap().unwrap();
        assert_eq!(row(6), read.row);
        for w in links.windows(2) {
            assert_eq!(Some(w[1].tid), w[0].header.next);
        }
        let newest = walk(pager, first, |l| l.header.next.is_none()).unwrap();
        assert_eq!(Some(back), newest.map(|l| l.tid));
    });
}

#[test]
fn a_heap_only_update_stays_on_its_page_and_flags_both_versions() {
    let mut sim = Sim::new(2, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(rt, 4);
        let pager = &mut db.pager;
        let heap = &mut db.heaps[APPLE];
        let t = heap
            .insert(pager, 1, &[Value::I64(1), Value::I32(0)])
            .unwrap();
        let u = heap
            .update(pager, t, 2, &[Value::I64(1), Value::I32(1)], &[0])
            .unwrap();
        assert!(u.heap_only);
        assert_eq!(t.page(), u.tid.page());
        let old = heap.read(pager, t).unwrap().unwrap();
        let new = heap.read(pager, u.tid).unwrap().unwrap();
        assert_eq!(Flags::HOT_UPDATED, old.flags);
        assert_eq!(Flags::HEAP_ONLY, new.flags);
        assert_eq!((2, Some(u.tid)), (old.header.xmax, old.header.next));
        let keyed = heap
            .update(pager, u.tid, 3, &[Value::I64(2), Value::I32(1)], &[0])
            .unwrap();
        assert!(!keyed.heap_only);
        let new = heap.read(pager, u.tid).unwrap().unwrap();
        assert_eq!(Flags::HEAP_ONLY, new.flags);
        assert_eq!(
            Flags::NONE,
            heap.read(pager, keyed.tid).unwrap().unwrap().flags
        );
        let unindexed = heap
            .update(pager, keyed.tid, 4, &[Value::I64(2), Value::I32(5)], &[])
            .unwrap();
        assert!(unindexed.heap_only);
        heap.delete(pager, unindexed.tid, 5).unwrap();
        let gone = heap.read(pager, unindexed.tid).unwrap().unwrap();
        assert_eq!((5, None), (gone.header.xmax, gone.header.next));
    });
}

#[test]
fn an_aborted_successor_is_replaced_by_the_next_writer() {
    let mut sim = Sim::new(3, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(rt, 4);
        let pager = &mut db.pager;
        let row = vec![Value::I64(1), Value::Text("x".into()), Value::Bytes(vec![])];
        let t = db.heaps[EMAIL].insert(pager, 1, &row).unwrap();
        let (email, google) = pair(&mut db.heaps, EMAIL, GOOGLE);
        email.retag(pager, t, 2, google, &row).unwrap();
        pager.abort(2, &[], |_| {}).unwrap();
        let u = email.update(pager, t, 3, &row, &[0]).unwrap();
        let old = email.read(pager, t).unwrap().unwrap();
        assert_eq!((3, Some(u.tid)), (old.header.xmax, old.header.next));
        assert!(!old.flags.contains(Flags::MOVED));
        email.delete(pager, t, 4).unwrap();
        let old = email.read(pager, t).unwrap().unwrap();
        assert_eq!(
            (4, None, Flags::NONE),
            (old.header.xmax, old.header.next, old.flags)
        );
    });
}

#[test]
fn large_values_go_out_of_line_and_a_retag_keeps_their_pointers() {
    let mut sim = Sim::new(4, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(rt, 64);
        let pager = &mut db.pager;
        let large = db.heaps[EMAIL].large();
        let big: Vec<u8> = (0..50_000u32).map(|i| (i * 7 + i / 300) as u8).collect();
        let row = |text: &str, bytes: &[u8]| {
            vec![
                Value::I64(1),
                Value::Text(text.into()),
                Value::Bytes(bytes.to_vec()),
            ]
        };
        let t = db.heaps[EMAIL].insert(pager, 1, &row("a", &big)).unwrap();
        let stored = large.pages(pager).unwrap().len();
        assert!(stored >= 50_000 / PAGE, "{stored} pages");
        assert_eq!(1, db.heaps[EMAIL].pages(pager).unwrap().len());
        assert_eq!(
            row("a", &big),
            db.heaps[EMAIL].read(pager, t).unwrap().unwrap().row
        );
        let (email, google) = pair(&mut db.heaps, EMAIL, GOOGLE);
        let moved = email.retag(pager, t, 2, google, &row("b", &big)).unwrap();
        assert_eq!(stored, large.pages(pager).unwrap().len());
        assert_eq!(
            row("b", &big),
            google.read(pager, moved).unwrap().unwrap().row
        );
        let u = google
            .update(pager, moved, 3, &row("c", &big), &[0])
            .unwrap();
        assert_eq!(stored, large.pages(pager).unwrap().len());
        let mut other = big.clone();
        other[49_999] ^= 1;
        let v = google
            .update(pager, u.tid, 4, &row("c", &other), &[0])
            .unwrap();
        assert!(large.pages(pager).unwrap().len() > stored);
        assert_eq!(
            row("c", &other),
            google.read(pager, v.tid).unwrap().unwrap().row
        );
        assert_eq!(row("a", &big), email.read(pager, t).unwrap().unwrap().row);
        let mid = vec![9u8; 900];
        let w = google.insert(pager, 5, &row("mid", &mid)).unwrap();
        assert_eq!(
            row("mid", &mid),
            google.read(pager, w).unwrap().unwrap().row
        );
    });
}

#[test]
fn rows_too_large_for_a_page_are_refused() {
    let mut sim = Sim::new(5, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(rt, 4);
        let pager = &mut db.pager;
        let large = db.heaps[EMAIL].large();
        let wide = vec![scalar(Scalar::Decimal(0)); 300];
        assert!(matches!(
            Heap::create(pager, 20, wide, large),
            Err(Error::TooLarge { .. })
        ));
        let many = vec![scalar(Scalar::Text); 400];
        let heap = Heap::create(pager, 21, many, large).unwrap();
        let row = vec![Value::Text("y".repeat(30)); 400];
        assert!(matches!(
            heap.insert(pager, 1, &row),
            Err(Error::TooLarge { .. })
        ));
        assert!(heap.pages(pager).unwrap().is_empty());
    });
}

#[test]
fn the_directory_splits_and_its_root_stays() {
    let mut sim = Sim::new(6, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(rt, 3);
        let root = db.heaps[EMAIL].root();
        let mut tids = Vec::new();
        for key in 0..200 {
            let row = vec![
                Value::I64(key),
                Value::Text(String::new()),
                Value::Bytes(vec![key as u8; 900]),
            ];
            tids.push((db.heaps[EMAIL].insert(&db.pager, 1, &row).unwrap(), row));
            assert_eq!(Vec::<String>::new(), verify(&mut db), "after key {key}");
        }
        assert_eq!(root, db.heaps[EMAIL].root());
        let pages = db.heaps[EMAIL].pages(&db.pager).unwrap();
        assert!(
            pages.len() > 27,
            "{} pages, so no fourth level",
            pages.len()
        );
        assert_eq!(pages.len(), pages.iter().collect::<BTreeSet<_>>().len());
        for (tid, row) in tids {
            let v = db.heaps[EMAIL].read(&db.pager, tid).unwrap().unwrap();
            assert_eq!(row, v.row);
        }
    });
}

#[test]
fn a_reopened_heap_finds_room_through_its_directory() {
    let mut sim = Sim::new(7, Config::default());
    finished(&mut sim, |rt| {
        let (db, roots) = Db::create(rt.clone(), 3);
        for key in 0..40 {
            let row = vec![
                Value::I64(key),
                Value::Text(String::new()),
                Value::Bytes(vec![1; 900]),
            ];
            db.heaps[EMAIL].insert(&db.pager, 1, &row).unwrap();
        }
        let pages = db.heaps[EMAIL].pages(&db.pager).unwrap().len();
        let reopened = Heap::open(
            &db.pager,
            HEAP_IDS[EMAIL],
            roots.heaps[EMAIL],
            row_type(EMAIL),
            db.heaps[EMAIL].large(),
        )
        .unwrap();
        let small = vec![
            Value::I64(99),
            Value::Text(String::new()),
            Value::Bytes(vec![2; 10]),
        ];
        let t = reopened.insert(&db.pager, 2, &small).unwrap();
        assert_eq!(pages, reopened.pages(&db.pager).unwrap().len());
        assert_eq!(small, reopened.read(&db.pager, t).unwrap().unwrap().row);
    });
}

#[test]
fn a_heap_scan_builds_a_batch_that_reads_back_its_rows() {
    let mut sim = Sim::new(8, Config::default());
    finished(&mut sim, |rt| {
        let (db, _) = Db::create(rt, 4);
        let ctor = |id, fields| CtorType {
            id: CtorId(id),
            fields,
        };
        let row_type = vec![
            scalar(Scalar::I64),
            FieldType::Sum(vec![ctor(1, vec![]), ctor(2, vec![scalar(Scalar::Text)])]),
            FieldType::Sum(vec![
                ctor(1040, vec![scalar(Scalar::I32)]),
                ctor(1041, vec![scalar(Scalar::Text), scalar(Scalar::I64)]),
                ctor(1042, vec![]),
            ]),
        ];
        let large = db.heaps[EMAIL].large();
        let heap = Heap::create(&db.pager, 20, row_type.clone(), large).unwrap();
        let rows: Vec<Vec<Value>> = (0..300i64)
            .map(|k| {
                let note = match k % 3 {
                    0 => Value::Sum {
                        ctor: 0,
                        fields: vec![],
                    },
                    _ => Value::Sum {
                        ctor: 1,
                        fields: vec![Value::Text(format!("n{k}"))],
                    },
                };
                let detail = match k % 4 {
                    0 => Value::Sum {
                        ctor: 0,
                        fields: vec![Value::I32(k as i32)],
                    },
                    1 => Value::Sum {
                        ctor: 1,
                        fields: vec![Value::Text("t".repeat((k % 7) as usize)), Value::I64(-k)],
                    },
                    _ => Value::Sum {
                        ctor: 2,
                        fields: vec![],
                    },
                };
                vec![Value::I64(k), note, detail]
            })
            .collect();
        for r in &rows {
            heap.insert(&db.pager, 1, r).unwrap();
        }
        let mut b = BatchBuilder::new(Schema::of_heap(&row_type));
        heap.scan(&db.pager, |v| b.push_row(&v.row)).unwrap();
        let batch: Batch = b.finish();
        assert_eq!(rows.len(), batch.len());
        let mut read: Vec<Vec<Value>> = (0..batch.len()).map(|i| batch.row(i)).collect();
        read.sort_by_key(|r| key_of(r));
        assert_eq!(rows, read);
        assert!(matches!(batch.columns()[1], Column::Nullable { .. }));
        assert!(matches!(batch.columns()[2], Column::Union { .. }));
    });
}

fn counter_type(note: bool) -> Vec<FieldType> {
    let mut t = vec![scalar(Scalar::I64), scalar(Scalar::I32)];
    if note {
        t.push(scalar(Scalar::Text));
    }
    t
}

#[test]
fn an_old_format_page_is_read_as_it_is_and_converted_on_insert() {
    let mut sim = Sim::new(8, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(rt, 4);
        let pager = &mut db.pager;
        let large = db.heaps[EMAIL].large();
        let mut heap = Heap::create(pager, 30, counter_type(false), large).unwrap();
        let old: Vec<Tid> = (0..20)
            .map(|k| {
                heap.insert(pager, 1, &[Value::I64(k), Value::I32(-1)])
                    .unwrap()
            })
            .collect();
        let page = Pid(old[0].page());
        assert_eq!(
            PageKind::HeapFixed as u16,
            storage::kind(&pager.read(page).unwrap())
        );
        let v = heap.add_format(
            counter_type(true),
            Box::new(|mut row| {
                row.push(Value::Text("n/a".into()));
                row
            }),
        );
        assert_eq!(Ok(2), v);
        let upgraded = |k: i64| vec![Value::I64(k), Value::I32(-1), Value::Text("n/a".into())];
        assert_eq!(upgraded(3), heap.read(pager, old[3]).unwrap().unwrap().row);
        heap.delete(pager, old[4], 2).unwrap();
        assert_eq!(1, storage::format(&pager.read(page).unwrap()));
        let fresh = vec![Value::I64(100), Value::I32(5), Value::Text("new".into())];
        let t = heap.insert(pager, 3, &fresh).unwrap();
        assert_eq!(page.0, t.page());
        let p = &pager.read(page).unwrap().to_vec();
        assert_eq!(
            (2, PageKind::HeapSlotted as u16),
            (storage::format(p), storage::kind(p))
        );
        for (k, tid) in old.iter().enumerate() {
            let v = heap.read(pager, *tid).unwrap().unwrap();
            assert_eq!(upgraded(k as i64), v.row);
            assert_eq!(if k == 4 { 2 } else { 0 }, v.header.xmax);
        }
        assert_eq!(fresh, heap.read(pager, t).unwrap().unwrap().row);
        assert!(heap.verify(pager).unwrap().is_empty());
    });
}

#[test]
fn a_page_whose_rows_outgrow_it_keeps_its_format_and_takes_no_insert() {
    let mut sim = Sim::new(9, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(rt, 4);
        let pager = &mut db.pager;
        let large = db.heaps[EMAIL].large();
        let mut heap = Heap::create(pager, 31, counter_type(false), large).unwrap();
        let mut old: Vec<Tid> = Vec::new();
        for k in 0.. {
            let t = heap
                .insert(pager, 1, &[Value::I64(k), Value::I32(0)])
                .unwrap();
            if !old.is_empty() && t.page() != old[0].page() {
                break;
            }
            old.push(t);
        }
        let first = Pid(old[0].page());
        heap.add_format(
            counter_type(true),
            Box::new(|mut row| {
                row.push(Value::Text("z".repeat(60)));
                row
            }),
        )
        .unwrap();
        for k in 0..30 {
            let t = heap
                .insert(
                    pager,
                    2,
                    &[
                        Value::I64(1000 + k),
                        Value::I32(1),
                        Value::Text(String::new()),
                    ],
                )
                .unwrap();
            assert_ne!(first.0, t.page());
        }
        assert_eq!(1, storage::format(&pager.read(first).unwrap()));
        let v = heap.read(pager, old[7]).unwrap().unwrap();
        assert_eq!(Value::Text("z".repeat(60)), v.row[2]);
        let u = heap
            .update(
                pager,
                old[7],
                3,
                &[Value::I64(7), Value::I32(9), Value::Text(String::new())],
                &[0],
            )
            .unwrap();
        assert!(!u.heap_only);
        assert_eq!(1, storage::format(&pager.read(first).unwrap()));
        assert!(heap.verify(pager).unwrap().is_empty());
    });
}

fn silence_fatal_panics() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let default = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let msg = info
                .payload()
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| info.payload().downcast_ref::<&str>().copied())
                .unwrap_or("");
            if !msg.starts_with(FATAL) {
                default(info);
            }
        }));
    });
}

fn run_process<F: FnOnce(SimRuntime)>(sim: &mut Sim, body: F) -> Outcome {
    match panic::catch_unwind(AssertUnwindSafe(|| sim.run(body))) {
        Ok(o) => o,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_default();
            if msg.starts_with(FATAL) {
                Outcome::Crashed
            } else {
                panic::resume_unwind(payload)
            }
        }
    }
}

/// What the client knows, kept outside the simulated process so it survives crashes.
struct World {
    roots: Roots,
    next_xid: u64,
    next_key: i64,
    ops: BTreeMap<u64, Op>,
    committed: BTreeSet<u64>,
    acked: BTreeSet<u64>,
    in_doubt: BTreeSet<u64>,
}

fn workload(rt: SimRuntime, world: &Mutex<World>, steps: usize) {
    let roots = world.lock().unwrap().roots;
    let Some(mut db) = Db::open(rt.clone(), roots) else {
        return;
    };
    let committed = world.lock().unwrap().committed.clone();
    let (mut live, _) = current(&mut db, |x| committed.contains(&x));
    let mut unacked = Vec::new();
    let ack = |db: &mut Db, unacked: &mut Vec<u64>| {
        db.pager.flush_log();
        let mut w = world.lock().unwrap();
        for x in unacked.drain(..) {
            w.in_doubt.remove(&x);
            w.committed.insert(x);
            w.acked.insert(x);
        }
    };
    let mut next = || rt.next_u64();
    for _ in 0..steps {
        match next() % 20 {
            0..=14 => {
                let (op, xid) = {
                    let mut w = world.lock().unwrap();
                    let op = generate(&mut next, &live, &mut w.next_key);
                    w.next_xid += 1;
                    let xid = w.next_xid;
                    w.ops.insert(xid, op.clone());
                    w.in_doubt.insert(xid);
                    (op, xid)
                };
                perform(&mut db, &mut live, &op, xid).expect("perform");
                db.pager.commit(xid, &[], |_| {}).unwrap();
                unacked.push(xid);
            }
            15..=16 => {
                let n = 1 + (next() % 6) as usize;
                db.pager.flush_pages(n);
            }
            17 => db.pager.checkpoint(),
            _ => ack(&mut db, &mut unacked),
        }
    }
    ack(&mut db, &mut unacked);
}

/// Reopen the database and check it against every committed operation; settle the commits left
/// in doubt by what the log holds.
fn inspect(rt: SimRuntime, world: &Mutex<World>) -> Vec<String> {
    let roots = world.lock().unwrap().roots;
    let Some(mut db) = Db::open(rt, roots) else {
        return vec!["open failed".into()];
    };
    let v = db.pager.verify();
    let mut problems: Vec<String> = v.findings.iter().map(|f| format!("{f:?}")).collect();
    let logged: BTreeSet<u64> = v.commits.into_iter().collect();
    let mut w = world.lock().unwrap();
    for x in w.acked.iter().filter(|x| !logged.contains(x)) {
        problems.push(format!("acknowledged {x} is not in the log"));
    }
    let doubt: Vec<u64> = std::mem::take(&mut w.in_doubt).into_iter().collect();
    for x in doubt {
        if logged.contains(&x) {
            w.committed.insert(x);
        }
    }
    let mut expected = BTreeMap::new();
    for (xid, op) in &w.ops {
        if w.committed.contains(xid) {
            replay(&mut expected, op);
        }
    }
    let committed = w.committed.clone();
    drop(w);
    let (found, dupes) = current(&mut db, |x| committed.contains(&x));
    problems.extend(dupes);
    if rows(&found) != expected {
        let keys: BTreeSet<&i64> = expected.keys().chain(found.keys()).collect();
        for k in keys {
            let got = found.get(k).map(|(h, _, r)| (*h, r.clone()));
            if got.as_ref() != expected.get(k) {
                problems.push(format!(
                    "key {k}: expected {:?}, found {:?}",
                    expected.get(k).map(|e| e.0),
                    got.map(|g| g.0)
                ));
            }
        }
    }
    problems.extend(verify(&mut db));
    let inventory = Inventory {
        sums: vec![Sum {
            heaps: (0..3).map(|h| (CtorId(h as u32), &db.heaps[h])).collect(),
            indexes: Vec::new(),
        }],
        ..Inventory::default()
    };
    problems.extend(common::verify_after_crash(&db.pager, &inventory));
    problems
}

fn crash_loop(seed: u64, faults: Faults, rounds: usize) -> usize {
    silence_fatal_panics();
    let mut sim = Sim::new(
        seed,
        Config {
            faults: quiet(),
            ..Config::default()
        },
    );
    let mut roots = None;
    finished(&mut sim, |rt| {
        let (db, r) = Db::create(rt, 3);
        db.pager.checkpoint();
        roots = Some(r);
    });
    let world = Arc::new(Mutex::new(World {
        roots: roots.unwrap(),
        next_xid: 0,
        next_key: 0,
        ops: BTreeMap::new(),
        committed: BTreeSet::new(),
        acked: BTreeSet::new(),
        in_doubt: BTreeSet::new(),
    }));
    let mut crashes = 0;
    for round in 0..rounds {
        sim.set_faults(faults.clone());
        let at = sim.now()
            + Duration::from_micros(2000 + (seed * 7919 + round as u64 * 104_729) % 80_000);
        sim.crash_at(at);
        let w = world.clone();
        if run_process(&mut sim, |rt| workload(rt, &w, 300)) == Outcome::Crashed {
            crashes += 1;
        }
        sim.set_faults(quiet());
        let mut problems = Vec::new();
        let w = world.clone();
        finished(&mut sim, |rt| problems = inspect(rt, &w));
        assert!(
            problems.is_empty(),
            "seed {seed} round {round}: {problems:#?}"
        );
    }
    let w = world.lock().unwrap();
    eprintln!(
        "seed {seed}: {crashes} crashes, {} ops, {} committed, {} acknowledged",
        w.ops.len(),
        w.committed.len(),
        w.acked.len()
    );
    assert!(!w.acked.is_empty(), "seed {seed} acknowledged nothing");
    crashes
}

#[test]
fn crashes_with_torn_writes_keep_every_committed_change() {
    let crashes: usize = (0..8)
        .map(|seed| {
            crash_loop(
                seed,
                Faults {
                    torn_p: 0.5,
                    ..quiet()
                },
                4,
            )
        })
        .sum();
    assert!(crashes > 10, "{crashes} crashes");
}

#[test]
fn failed_writes_crash_and_recover_every_committed_change() {
    let crashes: usize = (100..106)
        .map(|seed| {
            crash_loop(
                seed,
                Faults {
                    torn_p: 0.3,
                    fail_p: 0.003,
                    ..quiet()
                },
                4,
            )
        })
        .sum();
    assert!(crashes > 5, "{crashes} crashes");
}
