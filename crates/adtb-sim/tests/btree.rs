use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::{Bound, Range};
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use adtb::btree::build::BuildError;
use adtb::btree::{BTree, Cursor, Error, Index, SumKey, tag_prefix};
use adtb::key::{
    CtorId, CtorType, Dir, Entry, KeySchema, KeyType, OrderKey, Scalar, SumType, Tid, Value,
};
use adtb::runtime::{
    Clock, Completion, Entropy, Instant, Io, Op, Runtime, ThreadId, Threads, Ticket,
};
use adtb::storage::{Options, Pager, Pid};
use adtb::verify::Inventory;
use adtb_sim::{Config, Faults, Outcome, Recording, Sim, SimIo, SimRuntime};

use adtb_simkit::crash;
use adtb_simkit::draw;
use adtb_simkit::harness;
use adtb_simkit::recovery;
use adtb_simkit::seeds;

use crash::crash_after;
use draw::below;
use harness::{PAGE, finished, options};
use recovery::verify_after_crash;
use seeds::{share, sweep};

const FRAMES: usize = 64;

const OWNER: u32 = 41;
const SCAN: usize = 64;

/// Run `body` over a new database in a fresh simulation.
fn with_pager(seed: u64, body: impl FnOnce(&SimRuntime, &Pager<SimRuntime>)) {
    let mut sim = Sim::new(seed, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options(FRAMES)).expect("create");
        body(&rt, &pager);
    });
}

/// Returns an entry drawn from few bytes, so that entries share long prefixes.
fn gen_entry(rt: &SimRuntime, max_len: usize) -> Vec<u8> {
    let len = if rt.next_u64().is_multiple_of(50) {
        max_len - below(rt, 4)
    } else {
        below(rt, 300.min(max_len))
    };
    let alphabet = [0x00, 0x01, 0x61, 0x62, 0xFE, 0xFF];
    let mut e: Vec<u8> = (0..len)
        .map(|i| {
            if i < 3 {
                alphabet[below(rt, alphabet.len())]
            } else {
                rt.next_u64() as u8
            }
        })
        .collect();
    if len > 0 && rt.next_u64().is_multiple_of(3) {
        e[len - 1] = 0xFF;
    }
    e
}

const STEM: usize = 280;

/// Returns an entry of `gen_entry`, or, two times in three, one whose first
/// `STEM` bytes repeat its first 3 and whose last at least 8 are random.
fn gen_stemmed(rt: &SimRuntime, max_len: usize) -> Vec<u8> {
    let mut e = gen_entry(rt, max_len);
    if rt.next_u64().is_multiple_of(3) || e.len() < 3 {
        return e;
    }
    let len = e.len().max(STEM + 8).min(max_len);
    e.resize(len, 0);
    for i in 3..len {
        e[i] = if i < STEM {
            e[i % 3]
        } else {
            rt.next_u64() as u8
        };
    }
    e
}

fn gen_bound(rt: &SimRuntime, max_len: usize) -> Bound<Vec<u8>> {
    match below(rt, 3) {
        0 => Bound::Unbounded,
        1 => Bound::Included(gen_stemmed(rt, max_len)),
        _ => Bound::Excluded(gen_stemmed(rt, max_len)),
    }
}

fn valid_range(lo: &Bound<Vec<u8>>, hi: &Bound<Vec<u8>>) -> bool {
    match (lo, hi) {
        (Bound::Included(l), Bound::Included(h)) => l <= h,
        (Bound::Included(l) | Bound::Excluded(l), Bound::Excluded(h) | Bound::Included(h)) => l < h,
        _ => true,
    }
}

fn scan(
    pager: &Pager<SimRuntime>,
    tree: &BTree,
    lo: &Bound<Vec<u8>>,
    hi: &Bound<Vec<u8>>,
) -> Vec<Vec<u8>> {
    tree.range(
        lo.as_ref().map(Vec::as_slice),
        hi.as_ref().map(Vec::as_slice),
    )
    .collect(pager)
    .expect("scan")
}

/// A runtime that counts the page reads its I/O queues submit.
#[derive(Clone)]
struct Counted {
    rt: SimRuntime,
    reads: Arc<AtomicUsize>,
}

struct CountedIo {
    io: SimIo,
    reads: Arc<AtomicUsize>,
}

impl Clock for Counted {
    fn now(&self) -> Instant {
        self.rt.now()
    }

    fn wall(&self) -> SystemTime {
        self.rt.wall()
    }
}

impl Threads for Counted {
    fn spawn<F>(&self, name: &str, body: F) -> ThreadId
    where
        F: FnOnce() + Send + 'static,
    {
        self.rt.spawn(name, body)
    }

    fn current(&self) -> ThreadId {
        self.rt.current()
    }

    fn park(&self) {
        self.rt.park();
    }

    fn unpark(&self, thread: ThreadId) {
        self.rt.unpark(thread);
    }

    fn yield_now(&self) {
        self.rt.yield_now();
    }

    fn join(&self, thread: ThreadId) {
        self.rt.join(thread);
    }
}

impl Entropy for Counted {
    fn next_u64(&self) -> u64 {
        self.rt.next_u64()
    }
}

impl Runtime for Counted {
    type Io = CountedIo;

    fn io(&self) -> CountedIo {
        CountedIo {
            io: self.rt.io(),
            reads: self.reads.clone(),
        }
    }
}

impl Io for CountedIo {
    fn submit(&mut self, op: Op) -> Ticket {
        if matches!(op, Op::Read { .. }) {
            self.reads.fetch_add(1, Relaxed);
        }
        self.io.submit(op)
    }

    fn poll(&mut self) -> Option<Completion> {
        self.io.poll()
    }
}

const WINDOW: usize = 2000;

/// Run a random workload of `ops` operations against a `BTreeSet` model.
///
/// Every window of operations must average at most one page read per level,
/// with the pool far smaller than the tree.
fn model_run(seed: u64, ops: usize, expected_height: u16) {
    let mut sim = Sim::new(seed, Config::default());
    finished(&mut sim, |rt| {
        let reads = Arc::new(AtomicUsize::new(0));
        let counted = Counted {
            rt: rt.clone(),
            reads: reads.clone(),
        };
        let pager = &mut Pager::create(counted, &options(FRAMES)).expect("create");
        let rt = &rt;
        let tree = BTree::create(pager, OWNER).expect("create tree");
        let max = BTree::max_entry_len(PAGE);
        let mut model: BTreeSet<Vec<u8>> = BTreeSet::new();
        let mut window_reads = 0;
        for op in 0..ops {
            let before = reads.load(Relaxed);
            match below(rt, 20) {
                0..=11 => {
                    let e = gen_stemmed(rt, max);
                    let fresh = !model.contains(&e);
                    assert_eq!(tree.insert(pager, &e), Ok(fresh), "insert {e:x?}");
                    model.insert(e);
                }
                12..=15 => {
                    let e = if !model.is_empty() && rt.next_u64().is_multiple_of(4) {
                        let lo = gen_stemmed(rt, max);
                        let first = model.range(lo..).next().or(model.iter().next());
                        first.cloned().unwrap()
                    } else {
                        gen_stemmed(rt, max)
                    };
                    let present = model.remove(&e);
                    assert_eq!(tree.delete(pager, &e), Ok(present), "delete {e:x?}");
                    assert_eq!(tree.contains(pager, &e), Ok(false));
                }
                16 | 17 => {
                    let (lo, hi) = (gen_bound(rt, max), gen_bound(rt, max));
                    if valid_range(&lo, &hi) {
                        let expected: Vec<Vec<u8>> = model
                            .range((lo.clone(), hi.clone()))
                            .take(SCAN)
                            .cloned()
                            .collect();
                        let cursor = tree.range(
                            lo.as_ref().map(Vec::as_slice),
                            hi.as_ref().map(Vec::as_slice),
                        );
                        assert_eq!(first_scan(pager, cursor), expected, "{lo:x?} {hi:x?}");
                    }
                }
                18 => {
                    let p = gen_entry(rt, 6);
                    let expected: Vec<Vec<u8>> = model
                        .range(p.clone()..)
                        .take_while(|e| e.starts_with(&p))
                        .take(SCAN)
                        .cloned()
                        .collect();
                    let found = first_scan(pager, tree.prefix(&p));
                    assert_eq!(found, expected, "prefix {p:x?}");
                }
                _ => {
                    let e = gen_stemmed(rt, max);
                    assert_eq!(tree.contains(pager, &e), Ok(model.contains(&e)));
                }
            }
            window_reads += reads.load(Relaxed) - before;
            if (op + 1) % WINDOW == 0 {
                let height = usize::from(tree.height(pager).unwrap());
                assert!(
                    window_reads <= (height + 1) * WINDOW,
                    "{window_reads} page reads in the {WINDOW} operations to {op}, height {height}"
                );
                window_reads = 0;
                let all: Vec<Vec<u8>> = model.iter().cloned().collect();
                assert_eq!(tree.verify(pager).expect("verify"), all, "op {op}");
            }
        }
        let all: Vec<Vec<u8>> = model.iter().cloned().collect();
        assert_eq!(tree.verify(pager).expect("verify"), all);
        assert!(
            tree.height(pager).unwrap() >= expected_height,
            "height {:?} with {} entries",
            tree.height(pager),
            model.len()
        );
    });
}

/// Returns the first `SCAN` entries of `cursor`.
fn first_scan<R: Runtime>(pager: &Pager<R>, mut cursor: Cursor) -> Vec<Vec<u8>> {
    let mut found = Vec::new();
    while found.len() < SCAN
        && let Some(e) = cursor.next(pager).expect("scan")
    {
        found.push(e);
    }
    found
}

#[test]
fn random_operations_match_a_btreemap() {
    model_run(1, 12_000, 3);
}

#[test]
fn random_operations_match_a_btreemap_on_other_seeds() {
    for seed in 2..5 {
        model_run(seed, 5_000, 2);
    }
}

#[test]
fn ascending_and_descending_inserts_split_cleanly() {
    with_pager(9, |_, pager| {
        let up = BTree::create(pager, OWNER).unwrap();
        let down = BTree::create(pager, OWNER + 1).unwrap();
        let entry = |i: u32| {
            let mut e = vec![0xAB; 112];
            e.extend_from_slice(&i.to_be_bytes());
            e.resize(120, 0xAB);
            e
        };
        for i in 0..3000 {
            assert_eq!(up.insert(pager, &entry(i)), Ok(true));
            assert_eq!(down.insert(pager, &entry(2999 - i)), Ok(true));
        }
        let all: Vec<Vec<u8>> = (0..3000).map(entry).collect();
        assert_eq!(up.verify(pager).unwrap(), all);
        assert_eq!(down.verify(pager).unwrap(), all);
        assert!(up.height(pager).unwrap() >= 2);
        for i in (0..3000).step_by(2) {
            assert_eq!(up.delete(pager, &entry(i)), Ok(true));
        }
        let odd: Vec<Vec<u8>> = (0..3000).filter(|i| i % 2 == 1).map(entry).collect();
        assert_eq!(up.verify(pager).unwrap(), odd);
        let from = entry(1001);
        let to = entry(1011);
        let got = scan(pager, &up, &Bound::Excluded(from), &Bound::Included(to));
        assert_eq!(got, [1003, 1005, 1007, 1009, 1011].map(entry));
    });
}

#[test]
fn oversized_entries_are_refused() {
    with_pager(10, |_, pager| {
        let tree = BTree::create(pager, OWNER).unwrap();
        let max = BTree::max_entry_len(PAGE);
        assert_eq!(tree.insert(pager, &vec![1; max]), Ok(true));
        assert_eq!(
            tree.insert(pager, &vec![1; max + 1]),
            Err(Error::TooLong { len: max + 1, max })
        );
    });
}

#[test]
fn a_tree_reopens_from_its_root() {
    let mut sim = Sim::new(11, Config::default());
    let root = Arc::new(Mutex::new(Pid(0)));
    let r = root.clone();
    finished(&mut sim, move |rt| {
        let pager = Pager::create(rt, &options(FRAMES)).unwrap();
        let tree = BTree::create(&pager, OWNER).unwrap();
        for i in 0..2000_u32 {
            tree.insert(&pager, &[i.to_be_bytes().as_slice(), &[7; 60]].concat())
                .unwrap();
        }
        pager.flush_log();
        *r.lock().unwrap() = tree.root();
    });
    let root = *root.lock().unwrap();
    finished(&mut sim, move |rt| {
        let (pager, _) = Pager::open(rt, &options(FRAMES)).unwrap();
        let tree = BTree::open(root, OWNER);
        assert_eq!(tree.verify(&pager).unwrap().len(), 2000);
        assert!(
            tree.contains(&pager, &[&1999_u32.to_be_bytes()[..], &[7; 60]].concat())
                .unwrap()
        );
        let other = BTree::open(root, OWNER + 1);
        assert_eq!(other.verify(&pager), Err(Error::Malformed(root)));
    });
}

fn tid(page: u64, slot: u16) -> Tid {
    Tid::new(page, slot).unwrap()
}

fn i64_schema() -> KeySchema {
    KeySchema::new(vec![(KeyType::Scalar(Scalar::I64), Dir::Asc)])
}

#[test]
fn entries_order_by_key_then_constructor_then_tid() {
    with_pager(20, |rt, pager| {
        let ctors: BTreeSet<CtorId> = [0, 1, 239, 240, 70_000].map(CtorId).into();
        let index = Index::create(pager, OWNER, i64_schema(), ctors.clone()).unwrap();
        let ctor_list: Vec<CtorId> = ctors.iter().copied().collect();
        let mut model: BTreeSet<(i64, CtorId, Tid)> = BTreeSet::new();
        for _ in 0..4000 {
            let key = [i64::MIN, -300, -1, 0, 1, 2, 77, i64::MAX][below(rt, 8)];
            let ctor = ctor_list[below(rt, ctor_list.len())];
            let t = tid(below(rt, 1 << 20) as u64, below(rt, 300) as u16);
            let entry = Entry {
                key: vec![Value::I64(key)],
                ctor,
                tid: t,
            };
            assert_eq!(
                index.insert(pager, &entry),
                Ok(model.insert((key, ctor, t)))
            );
        }
        let to_entry = |(k, c, t): &(i64, CtorId, Tid)| Entry {
            key: vec![Value::I64(*k)],
            ctor: *c,
            tid: *t,
        };
        let all: Vec<Entry> = model.iter().map(to_entry).collect();
        assert_eq!(
            index.range(pager, Bound::Unbounded, Bound::Unbounded),
            Ok(all)
        );
        for key in [i64::MIN, -1, 0, 77, i64::MAX, 5] {
            let of_key: Vec<Entry> = model.iter().filter(|e| e.0 == key).map(to_entry).collect();
            assert_eq!(index.probe(pager, &[Value::I64(key)]), Ok(of_key));
            for &ctor in &ctors {
                let of_ctor: Vec<Entry> = model
                    .iter()
                    .filter(|e| e.0 == key && e.1 == ctor)
                    .map(to_entry)
                    .collect();
                assert_eq!(
                    index.probe_ctor(pager, &[Value::I64(key)], ctor),
                    Ok(of_ctor)
                );
            }
        }
        let between: Vec<Entry> = model
            .iter()
            .filter(|e| e.0 > -300 && e.0 <= 2)
            .map(to_entry)
            .collect();
        assert_eq!(
            index.range(
                pager,
                Bound::Excluded(&[Value::I64(-300)]),
                Bound::Included(&[Value::I64(2)])
            ),
            Ok(between)
        );
        let first = model.iter().next().map(to_entry).unwrap();
        assert_eq!(index.delete(pager, &first), Ok(true));
        assert_eq!(index.delete(pager, &first), Ok(false));
    });
}

#[test]
fn the_identities_key_index_reads_one_key_as_one_range() {
    with_pager(21, |_, pager| {
        let (email, google, apple) = (CtorId(0), CtorId(1), CtorId(2));
        let index =
            Index::create(pager, OWNER, i64_schema(), [email, google, apple].into()).unwrap();
        let entries = [
            (8, google, tid(4, 2)),
            (7, google, tid(4, 1)),
            (7, email, tid(12, 3)),
            (7, apple, tid(1, 1)),
            (9, email, tid(2, 2)),
        ]
        .map(|(k, c, t)| Entry {
            key: vec![Value::I64(k)],
            ctor: c,
            tid: t,
        });
        for e in &entries {
            index.insert(pager, e).unwrap();
        }
        let seven = index.probe(pager, &[Value::I64(7)]).unwrap();
        let ctors: Vec<CtorId> = seven.iter().map(|e| e.ctor).collect();
        assert_eq!(ctors, [email, google, apple]);
        let google_seven = index.probe_ctor(pager, &[Value::I64(7)], google).unwrap();
        assert_eq!(google_seven, [entries[1].clone()]);
    });
}

#[test]
#[should_panic(expected = "is not in the index's constructor set")]
fn an_entry_of_a_constructor_outside_the_set_is_refused() {
    with_pager(22, |_, pager| {
        let index = Index::create(pager, OWNER, i64_schema(), [CtorId(1)].into()).unwrap();
        index
            .insert(
                pager,
                &Entry {
                    key: vec![Value::I64(1)],
                    ctor: CtorId(2),
                    tid: tid(1, 1),
                },
            )
            .unwrap();
    });
}

/// `type Status ordered = Draft | Sent { at: Instant } | Paid { at: Instant, ref: I64 }`.
fn status() -> SumType {
    let keys = OrderKey::spread(3);
    let fields = [
        vec![],
        vec![KeyType::Scalar(Scalar::Instant)],
        vec![
            KeyType::Scalar(Scalar::Instant),
            KeyType::Scalar(Scalar::I64),
        ],
    ];
    SumType::new(
        keys.into_iter()
            .zip(fields)
            .map(|(order_key, fields)| CtorType { order_key, fields })
            .collect(),
    )
    .unwrap()
}

fn gen_status(rt: &SimRuntime) -> Value {
    let at = Value::Instant(below(rt, 50) as i64 - 25);
    match below(rt, 3) {
        0 => Value::Sum {
            ctor: 0,
            fields: vec![],
        },
        1 => Value::Sum {
            ctor: 1,
            fields: vec![at],
        },
        _ => Value::Sum {
            ctor: 2,
            fields: vec![at, Value::I64(below(rt, 1000) as i64)],
        },
    }
}

fn cmp_status(a: &Value, b: &Value) -> Ordering {
    let (Value::Sum { ctor: c, fields: f }, Value::Sum { ctor: d, fields: g }) = (a, b) else {
        unreachable!()
    };
    let key = |fs: &[Value]| -> Vec<i64> {
        fs.iter()
            .map(|v| match v {
                Value::Instant(x) | Value::I64(x) => *x,
                _ => unreachable!(),
            })
            .collect()
    };
    c.cmp(d).then_with(|| key(f).cmp(&key(g)))
}

#[test]
fn value_level_sum_fields_index_whole_by_tag_and_by_constructor_field() {
    with_pager(23, |rt, pager| {
        let sum = status();
        let row = CtorId(0);
        let index_for = |pager: &Pager<SimRuntime>, owner: u32, how: SumKey| {
            let schema = KeySchema::new(vec![(how.key_type(&sum), Dir::Asc)]);
            Index::create(pager, owner, schema, [row].into()).unwrap()
        };
        let whole = index_for(pager, 1, SumKey::Whole);
        let tag = index_for(pager, 2, SumKey::Tag);
        let paid_ref = SumKey::Field { ctor: 2, field: 1 };
        let by_ref = index_for(pager, 3, paid_ref);
        let mut rows: Vec<(Tid, Value)> = Vec::new();
        for slot in 0..1500 {
            let t = tid(100 + slot / 50, (slot % 50) as u16);
            let v = gen_status(rt);
            for (index, how) in [
                (&whole, SumKey::Whole),
                (&tag, SumKey::Tag),
                (&by_ref, paid_ref),
            ] {
                if let Some(c) = how.component(&v) {
                    let e = Entry {
                        key: vec![c],
                        ctor: row,
                        tid: t,
                    };
                    assert_eq!(index.insert(pager, &e), Ok(true));
                }
            }
            rows.push((t, v));
        }

        let mut ordered = rows.clone();
        ordered.sort_by(|a, b| cmp_status(&a.1, &b.1).then(a.0.cmp(&b.0)));
        let scanned: Vec<(Tid, Value)> = whole
            .range(pager, Bound::Unbounded, Bound::Unbounded)
            .unwrap()
            .into_iter()
            .map(|e| (e.tid, e.key[0].clone()))
            .collect();
        assert_eq!(scanned, ordered);

        for ctor in 0..3 {
            let mut holding: Vec<Tid> = rows
                .iter()
                .filter(|(_, v)| matches!(v, Value::Sum { ctor: c, .. } if *c == ctor))
                .map(|(t, _)| *t)
                .collect();
            let prefix = tag_prefix(&sum, ctor, Dir::Asc);
            let mut from_whole: Vec<Tid> = whole
                .scan_prefix(pager, &prefix)
                .unwrap()
                .iter()
                .map(|e| e.tid)
                .collect();
            let from_tag: Vec<Tid> = tag
                .probe(
                    pager,
                    &[Value::Sum {
                        ctor,
                        fields: vec![],
                    }],
                )
                .unwrap()
                .iter()
                .map(|e| e.tid)
                .collect();
            holding.sort();
            assert_eq!(from_tag, holding, "tag {ctor}");
            from_whole.sort();
            assert_eq!(from_whole, holding, "whole {ctor}");
        }

        let lo = Value::Sum {
            ctor: 1,
            fields: vec![Value::Instant(0)],
        };
        let hi = Value::Sum {
            ctor: 2,
            fields: vec![Value::Instant(0), Value::I64(0)],
        };
        let sent_or_paid = whole
            .range(
                pager,
                Bound::Included(std::slice::from_ref(&lo)),
                Bound::Excluded(std::slice::from_ref(&hi)),
            )
            .unwrap();
        let expected: Vec<Tid> = ordered
            .iter()
            .filter(|(_, v)| cmp_status(v, &lo).is_ge() && cmp_status(v, &hi).is_lt())
            .map(|(t, _)| *t)
            .collect();
        let sent_or_paid: Vec<Tid> = sent_or_paid.iter().map(|e| e.tid).collect();
        assert_eq!(sent_or_paid, expected);

        let paid: Vec<(i64, Tid)> = rows
            .iter()
            .filter_map(|(t, v)| match v {
                Value::Sum { ctor: 2, fields } => match fields[1] {
                    Value::I64(r) => Some((r, *t)),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        let all_refs = by_ref
            .range(pager, Bound::Unbounded, Bound::Unbounded)
            .unwrap();
        assert_eq!(all_refs.len(), paid.len());
        let (r, t) = paid[0];
        let found = by_ref.probe(pager, &[Value::I64(r)]).unwrap();
        assert!(found.iter().any(|e| e.tid == t));
        assert!(found.iter().all(|e| e.key == [Value::I64(r)]));
    });
}

/// What the client knows, kept outside the simulated process so it survives crashes.
#[derive(Default)]
struct World {
    root: Option<Pid>,
    /// Each operation begun, an insert when true, in order.
    ops: Vec<(bool, Vec<u8>)>,
    /// Every operation before this one is durable.
    durable: usize,
    /// The recorded events before and after each insert of the last run.
    spans: Vec<(usize, usize)>,
}

fn crash_options() -> Options {
    Options {
        pool_frames: 5,
        ..options(FRAMES)
    }
}

fn crash_workload(rt: SimRuntime, world: &Mutex<World>, steps: usize) {
    let (pager, _) = Pager::open(rt.clone(), &crash_options()).expect("open");
    let tree = BTree::open(world.lock().unwrap().root.unwrap(), OWNER);
    let max = 200;
    for _ in 0..steps {
        let insert = below(&rt, 10) < 7;
        let e = if insert {
            gen_entry(&rt, max)
        } else {
            let w = world.lock().unwrap();
            let inserts: Vec<&Vec<u8>> = w.ops.iter().filter(|o| o.0).map(|o| &o.1).collect();
            if inserts.is_empty() {
                continue;
            }
            inserts[below(&rt, inserts.len())].clone()
        };
        world.lock().unwrap().ops.push((insert, e.clone()));
        let start = rt.recorded();
        if insert {
            tree.insert(&pager, &e).expect("insert");
        } else {
            tree.delete(&pager, &e).expect("delete");
        }
        if insert {
            world.lock().unwrap().spans.push((start, rt.recorded()));
        }
        match below(&rt, 30) {
            0..=2 => {
                pager.flush_log();
                let mut w = world.lock().unwrap();
                w.durable = w.ops.len();
            }
            3 => {
                pager.flush_pages(1 + below(&rt, 4));
            }
            4 => pager.checkpoint(),
            _ => {}
        }
    }
    pager.flush_log();
    let mut w = world.lock().unwrap();
    w.durable = w.ops.len();
}

fn apply(model: &mut BTreeSet<Vec<u8>>, op: &(bool, Vec<u8>)) {
    if op.0 {
        model.insert(op.1.clone());
    } else {
        model.remove(&op.1);
    }
}

/// Returns the number of splits incomplete in the forced state after the
/// first `at` events of `recording`.
fn incomplete_at(recording: &Recording, at: usize, root: Pid, seed: u64) -> usize {
    let files = recording.images(at, 1, seed).swap_remove(0);
    let mut sim = Sim::with_files(seed, Config::default(), files);
    let mut n = 0;
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &crash_options()).expect("reopen");
        n = BTree::open(root, OWNER).repair(&pager).expect("repair");
    });
    n
}

fn crash_config() -> Config {
    Config {
        faults: Faults {
            torn_p: 0.5,
            ..Faults::default()
        },
        ..Config::default()
    }
}

/// Run the workload to its end, recording it, and replace `sim` by the forced
/// state at the last log force inside an operation. When `seek` is set, take
/// the last such force that leaves a split incomplete, if there is one.
fn crash_at_a_flush(sim: &mut Sim, world: &Arc<Mutex<World>>, seed: u64, seek: bool) {
    let durable = {
        let mut w = world.lock().unwrap();
        w.spans.clear();
        w.durable
    };
    sim.record();
    let w = world.clone();
    finished(sim, move |rt| crash_workload(rt, &w, 600));
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
        .find(|&&f| seek && incomplete_at(&recording, f, root, seed) > 0)
        .or(inside.first())
        .or(flushes.last())
        .copied()
        .expect("the workload forces the log");
    let files = recording.images(at, 1, seed).swap_remove(0);
    *sim = Sim::with_files(seed, crash_config(), files);
    w.durable = durable;
}

/// Returns the crashes of `rounds` runs crashed at a time, and the splits
/// repaired after them and after as many runs cut at a log force inside an
/// operation.
///
/// Unless `seen` is set, each cut seeks a force that leaves a split
/// incomplete, until one is repaired.
fn crash_loop(seed: u64, rounds: usize, seen: bool) -> (usize, usize) {
    let mut sim = Sim::new(seed, crash_config());
    let world = Arc::new(Mutex::new(World::default()));
    let w = world.clone();
    finished(&mut sim, move |rt| {
        let pager = Pager::create(rt, &crash_options()).expect("create");
        let tree = BTree::create(&pager, OWNER).expect("create tree");
        pager.flush_log();
        w.lock().unwrap().root = Some(tree.root());
    });
    let mut crashes = 0;
    let repaired = Arc::new(Mutex::new(0));
    for round in 0..2 * rounds {
        let repairs = repaired.clone();
        if round % 2 == 0 {
            let at = sim.now() + crash_after(seed, round as u64, 2_000, 200_000);
            sim.crash_at(at);
            let w = world.clone();
            if sim.run(move |rt| crash_workload(rt, &w, 600)) == Outcome::Crashed {
                crashes += 1;
            }
        } else {
            let seek = !seen && *repaired.lock().unwrap() == 0;
            crash_at_a_flush(&mut sim, &world, seed * 1000 + round as u64, seek);
        }
        let root = world.lock().unwrap().root.unwrap();
        let found = Arc::new(Mutex::new(None));
        let f = found.clone();
        finished(&mut sim, move |rt| {
            let (pager, _) = Pager::open(rt, &crash_options()).expect("reopen");
            let tree = BTree::open(root, OWNER);
            let recovered = tree.verify(&pager);
            if recovered.is_ok() {
                *repairs.lock().unwrap() += tree.repair(&pager).expect("repair");
                assert_eq!(tree.repair(&pager), Ok(0), "seed {seed} round {round}");
                assert_eq!(tree.verify(&pager), recovered);
                let inventory = Inventory {
                    trees: vec![tree],
                    ..Inventory::default()
                };
                let problems = verify_after_crash(&pager, &inventory);
                assert!(
                    problems.is_empty(),
                    "seed {seed} round {round}: {problems:#?}"
                );
                pager.flush_log();
            }
            *f.lock().unwrap() = Some(recovered);
        });
        let entries = found
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .unwrap_or_else(|e| panic!("seed {seed} round {round}: {e:?}"));
        let mut w = world.lock().unwrap();
        let mut model = BTreeSet::new();
        for op in &w.ops[..w.durable] {
            apply(&mut model, op);
        }
        let mut kept = None;
        for k in w.durable..=w.ops.len() {
            if model.iter().eq(entries.iter()) {
                kept = Some(k);
                break;
            }
            if k < w.ops.len() {
                apply(&mut model, &w.ops[k]);
            }
        }
        let kept = kept.unwrap_or_else(|| {
            panic!(
                "seed {seed} round {round}: {} entries match no prefix of {} operations from {}",
                entries.len(),
                w.ops.len(),
                w.durable
            )
        });
        w.ops.truncate(kept);
        w.durable = kept;
    }
    (crashes, *repaired.lock().unwrap())
}

#[test]
fn a_crashed_tree_recovers_a_prefix_of_its_operations() {
    const SEEDS: Range<u64> = 0..6;
    let mut crashes = 0;
    let mut repaired = 0;
    for seed in sweep(SEEDS) {
        let (c, r) = crash_loop(seed, 6, repaired > 0);
        crashes += c;
        repaired += r;
    }
    assert!(crashes >= share(SEEDS, 10), "only {crashes} crashes");
    assert!(
        repaired >= share(SEEDS, 1),
        "no crash left a split incomplete"
    );
}

/// Returns an entry whose first 300 bytes every entry shares, so that
/// separators stay long and the tree grows tall.
fn split_entry(i: u32) -> Vec<u8> {
    let mut e = vec![0x5A; 300];
    e.extend_from_slice(&i.to_be_bytes());
    e.resize(400, 0x5A);
    e
}

const SPLIT_ENTRIES: u32 = 200;

fn split_options() -> Options {
    Options {
        pool_frames: 5,
        ..options(FRAMES)
    }
}

/// Returns what `body` makes of the tree a crash leaves in `files`.
///
/// The recovered tree must verify and hold only inserted entries; once
/// `body` has inserted them all, no split may be left to repair.
fn recover_and(
    files: BTreeMap<String, Vec<u8>>,
    root: Pid,
    at: usize,
    body: impl FnOnce(&BTree, &Pager<SimRuntime>) -> usize + Send,
) -> usize {
    let mut replay = Sim::with_files(at as u64, Config::default(), files);
    let mut found = 0;
    finished(&mut replay, |rt| {
        let (pager, _) = Pager::open(rt, &split_options()).expect("reopen");
        let tree = BTree::open(root, OWNER);
        let entries = tree.verify(&pager).expect("verify the recovered tree");
        let all: Vec<Vec<u8>> = (0..SPLIT_ENTRIES).map(split_entry).collect();
        assert!(
            entries.iter().all(|e| all.contains(e)),
            "flush boundary {at}"
        );
        found = body(&tree, &pager);
        assert_eq!(tree.repair(&pager), Ok(0), "flush boundary {at}");
        assert_eq!(
            tree.verify(&pager).as_ref(),
            Ok(&all),
            "flush boundary {at}"
        );
        let inventory = Inventory {
            trees: vec![tree],
            ..Inventory::default()
        };
        let problems = verify_after_crash(&pager, &inventory);
        assert!(problems.is_empty(), "flush boundary {at}: {problems:#?}");
    });
    found
}

fn insert_all(tree: &BTree, pager: &Pager<SimRuntime>) {
    for i in 0..SPLIT_ENTRIES {
        tree.insert(pager, &split_entry(i)).expect("insert");
    }
}

fn repair_and_insert(tree: &BTree, pager: &Pager<SimRuntime>) -> usize {
    let n = tree.repair(pager).expect("repair");
    insert_all(tree, pager);
    n
}

/// Records scattered inserts through a pool of four frames, so that a split
/// and its parent's update often fall in different log batches.
fn record_splits() -> (Sim, Pid) {
    let mut sim = Sim::new(31, Config::default());
    let mut root = Pid(0);
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt, &split_options()).expect("create");
        root = BTree::create(&pager, OWNER).expect("create tree").root();
        pager.flush_log();
    });
    sim.record();
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &split_options()).expect("open");
        let tree = BTree::open(root, OWNER);
        for i in 0..SPLIT_ENTRIES {
            let e = split_entry(i * 97 % SPLIT_ENTRIES);
            assert_eq!(tree.insert(&pager, &e), Ok(true));
        }
        pager.flush_log();
    });
    (sim, root)
}

#[test]
fn a_repair_pass_or_an_insert_finishes_every_split_a_crash_leaves_incomplete() {
    let (sim, root) = record_splits();
    let recording = sim.recording();
    let mut incomplete = 0;
    for at in recording.flushes() {
        let files = recording.images(at, 1, at as u64).swap_remove(0);
        let found = recover_and(files.clone(), root, at, repair_and_insert);
        if found > 0 {
            incomplete += found;
            recover_and(files, root, at, |tree, pager| {
                insert_all(tree, pager);
                0
            });
        }
    }
    assert!(incomplete >= 5, "{incomplete} incomplete splits");
}

#[test]
fn an_index_keeps_the_entries_of_one_key_as_postings() {
    with_pager(24, |rt, pager| {
        let ctors: BTreeSet<CtorId> = [CtorId(0), CtorId(1), CtorId(300)].into();
        let index = Index::create(pager, OWNER, i64_schema(), ctors.clone()).unwrap();
        let ctor_list: Vec<CtorId> = ctors.iter().copied().collect();
        let mut model: BTreeSet<(i64, CtorId, Tid)> = BTreeSet::new();
        let to_entry = |(k, c, t): &(i64, CtorId, Tid)| Entry {
            key: vec![Value::I64(*k)],
            ctor: *c,
            tid: *t,
        };
        for step in 0..12_000 {
            let key = [-7, 0, 1, 2, 9_000_000_000][below(rt, 5)];
            let ctor = ctor_list[below(rt, ctor_list.len())];
            let t = tid(below(rt, 1 << 30) as u64, below(rt, 400) as u16);
            if below(rt, 10) < 3 && !model.is_empty() {
                let probe = (key, ctor, t);
                let victim = *model
                    .range(probe..)
                    .next()
                    .unwrap_or(model.iter().next().unwrap());
                assert_eq!(index.delete(pager, &to_entry(&victim)), Ok(true));
                model.remove(&victim);
            } else {
                let e = (key, ctor, t);
                assert_eq!(index.insert(pager, &to_entry(&e)), Ok(model.insert(e)));
            }
            if step % 3000 == 0 {
                let all: Vec<Entry> = model.iter().map(to_entry).collect();
                assert_eq!(
                    index.range(pager, Bound::Unbounded, Bound::Unbounded),
                    Ok(all)
                );
            }
        }
        let all: Vec<Entry> = model.iter().map(to_entry).collect();
        assert_eq!(
            index.range(pager, Bound::Unbounded, Bound::Unbounded),
            Ok(all)
        );
        assert_eq!(index.tree().verify(pager).unwrap().len(), model.len());
        for key in [-7, 0, 9_000_000_000, 5] {
            let of_key: Vec<Entry> = model.iter().filter(|e| e.0 == key).map(to_entry).collect();
            assert_eq!(index.probe(pager, &[Value::I64(key)]), Ok(of_key));
        }
        let plain = BTree::create(pager, OWNER + 1).unwrap();
        let mut shuffled: Vec<_> = model.iter().collect();
        for i in (1..shuffled.len()).rev() {
            shuffled.swap(i, below(rt, i + 1));
        }
        for e in shuffled {
            let bytes = i64_schema().encode_entry(&to_entry(e));
            assert_eq!(plain.insert(pager, &bytes), Ok(true));
        }
        let pages = index.tree().pages(pager).unwrap().len();
        let whole = plain.pages(pager).unwrap().len();
        assert!(
            pages * 3 < whole * 2,
            "{pages} pages, {whole} without postings"
        );
    });
}

#[test]
fn a_bulk_build_gives_a_sound_tree_a_retail_insert_can_extend() {
    for (n, seed) in [(0u32, 90), (1, 91), (5000, 92)] {
        let mut sim = Sim::new(seed, Config::default());
        finished(&mut sim, |rt| {
            let pager = Pager::create(rt, &options(64)).unwrap();
            let entries: Vec<Vec<u8>> = (0..n).map(|i| (2 * i).to_be_bytes().to_vec()).collect();
            let mut made = Vec::new();
            let tree = BTree::bulk(&pager, 7, entries.clone(), &mut made, &|| false).unwrap();
            assert_eq!(entries, tree.verify(&pager).unwrap(), "{n} entries");
            let mut pages = tree.pages(&pager).unwrap();
            pages.sort();
            made.sort();
            assert_eq!(made, pages, "{n} entries: every page written is reached");
            if n >= 5000 {
                assert!(tree.height(&pager).unwrap() >= 1, "{n} entries");
                let leaves = pages.len() as u32;
                assert!(
                    leaves < n / 100,
                    "{leaves} pages for {n} entries are filled"
                );
            }
            for i in (1..n.min(2000)).step_by(2) {
                assert!(tree.insert(&pager, &i.to_be_bytes()).unwrap());
                assert!(!tree.insert(&pager, &(2 * i).to_be_bytes()).unwrap());
            }
            tree.verify(&pager).unwrap();
        });
    }
}

#[test]
fn a_bulk_build_stops_between_pages_and_names_what_it_wrote() {
    let mut sim = Sim::new(93, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt, &options(64)).unwrap();
        let entries: Vec<Vec<u8>> = (0..5000u32).map(|i| i.to_be_bytes().to_vec()).collect();
        let mut made = Vec::new();
        let asked = std::cell::Cell::new(0);
        let stop = || {
            asked.set(asked.get() + 1);
            asked.get() == 3
        };
        let got = BTree::bulk(&pager, 7, entries, &mut made, &stop);
        assert_eq!(Some(BuildError::Stopped), got.err());
        assert_eq!(2, made.len(), "every page written is named");
    });
}
