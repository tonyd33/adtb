use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::ops::Bound;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adtb::btree::{BTree, Error, Index, SumKey, tag_prefix};
use adtb::key::{
    CtorId, CtorType, Dir, Entry, KeySchema, KeyType, OrderKey, Scalar, SumType, Tid, Value,
};
use adtb::runtime::Entropy;
use adtb::storage::{Options, Pager, Pid};
use adtb_sim::{Config, Faults, Outcome, Sim, SimRuntime};

const PAGE: usize = 4096;
const OWNER: u32 = 41;
const NAME: &str = "app.adt";
const SCAN: usize = 64;

fn options() -> Options {
    Options {
        page_size: PAGE as u32,
        log_block: 512,
        segment_size: 256 * 1024,
        pool_frames: 64,
        ..Options::new(NAME)
    }
}

/// Run `body` over a new database in a fresh simulation.
fn with_pager(seed: u64, body: impl FnOnce(&SimRuntime, &mut Pager<SimRuntime>)) {
    let mut sim = Sim::new(seed, Config::default());
    let outcome = sim.run(|rt| {
        let mut pager = Pager::create(rt.clone(), &options()).expect("create");
        body(&rt, &mut pager);
    });
    assert_eq!(Outcome::Finished, outcome);
}

fn below(rt: &SimRuntime, n: usize) -> usize {
    (rt.next_u64() % n as u64) as usize
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

fn gen_bound(rt: &SimRuntime, max_len: usize) -> Bound<Vec<u8>> {
    match below(rt, 3) {
        0 => Bound::Unbounded,
        1 => Bound::Included(gen_entry(rt, max_len)),
        _ => Bound::Excluded(gen_entry(rt, max_len)),
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
    pager: &mut Pager<SimRuntime>,
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

fn model_run(seed: u64, ops: usize, expected_height: u16) {
    with_pager(seed, |rt, pager| {
        let tree = BTree::create(pager, OWNER).expect("create tree");
        let max = BTree::max_entry_len(PAGE);
        let mut model: BTreeSet<Vec<u8>> = BTreeSet::new();
        for op in 0..ops {
            match below(rt, 20) {
                0..=11 => {
                    let e = gen_entry(rt, max);
                    let fresh = !model.contains(&e);
                    assert_eq!(tree.insert(pager, &e), Ok(fresh), "insert {e:x?}");
                    model.insert(e);
                }
                12..=15 => {
                    let e = if !model.is_empty() && rt.next_u64().is_multiple_of(4) {
                        let lo = gen_entry(rt, max);
                        let first = model.range(lo..).next().or(model.iter().next());
                        first.cloned().unwrap()
                    } else {
                        gen_entry(rt, max)
                    };
                    let present = model.remove(&e);
                    assert_eq!(tree.delete(pager, &e), Ok(present), "delete {e:x?}");
                    assert_eq!(tree.contains(pager, &e), Ok(false));
                }
                16 | 17 => {
                    let (lo, hi) = (gen_bound(rt, max), gen_bound(rt, max));
                    if !valid_range(&lo, &hi) {
                        continue;
                    }
                    let expected: Vec<Vec<u8>> = model
                        .range((lo.clone(), hi.clone()))
                        .take(SCAN)
                        .cloned()
                        .collect();
                    let mut cursor = tree.range(
                        lo.as_ref().map(Vec::as_slice),
                        hi.as_ref().map(Vec::as_slice),
                    );
                    let mut found = Vec::new();
                    while found.len() < SCAN
                        && let Some(e) = cursor.next(pager).expect("scan")
                    {
                        found.push(e);
                    }
                    assert_eq!(found, expected, "{lo:x?} {hi:x?}");
                }
                18 => {
                    let p = gen_entry(rt, 6);
                    let expected: Vec<Vec<u8>> = model
                        .range(p.clone()..)
                        .take_while(|e| e.starts_with(&p))
                        .cloned()
                        .collect();
                    let found = tree.prefix(&p).collect(pager).expect("prefix");
                    assert_eq!(found, expected, "prefix {p:x?}");
                }
                _ => {
                    let e = gen_entry(rt, max);
                    assert_eq!(tree.contains(pager, &e), Ok(model.contains(&e)));
                }
            }
            if op % 2000 == 0 {
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
            let mut e = i.to_be_bytes().to_vec();
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
    let outcome = sim.run(move |rt| {
        let mut pager = Pager::create(rt, &options()).unwrap();
        let tree = BTree::create(&mut pager, OWNER).unwrap();
        for i in 0..2000_u32 {
            tree.insert(&mut pager, &[i.to_be_bytes().as_slice(), &[7; 60]].concat())
                .unwrap();
        }
        pager.flush_log();
        *r.lock().unwrap() = tree.root();
    });
    assert_eq!(Outcome::Finished, outcome);
    let root = *root.lock().unwrap();
    let outcome = sim.run(move |rt| {
        let (mut pager, _) = Pager::open(rt, &options()).unwrap();
        let tree = BTree::open(root, OWNER);
        assert_eq!(tree.verify(&mut pager).unwrap().len(), 2000);
        assert!(
            tree.contains(
                &mut pager,
                &[&1999_u32.to_be_bytes()[..], &[7; 60]].concat()
            )
            .unwrap()
        );
        let other = BTree::open(root, OWNER + 1);
        assert_eq!(other.verify(&mut pager), Err(Error::Malformed(root)));
    });
    assert_eq!(Outcome::Finished, outcome);
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
        let _ = index.insert(
            pager,
            &Entry {
                key: vec![Value::I64(1)],
                ctor: CtorId(2),
                tid: tid(1, 1),
            },
        );
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
        let index_for = |pager: &mut Pager<SimRuntime>, owner: u32, how: SumKey| {
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
}

fn crash_options() -> Options {
    Options {
        pool_frames: 8,
        ..options()
    }
}

fn crash_workload(rt: SimRuntime, world: &Mutex<World>, steps: usize) {
    let (mut pager, _) = Pager::open(rt.clone(), &crash_options()).expect("open");
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
        if insert {
            tree.insert(&mut pager, &e).expect("insert");
        } else {
            tree.delete(&mut pager, &e).expect("delete");
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

fn crash_loop(seed: u64, rounds: usize) -> usize {
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
    let world = Arc::new(Mutex::new(World::default()));
    let w = world.clone();
    let outcome = sim.run(move |rt| {
        let mut pager = Pager::create(rt, &crash_options()).expect("create");
        let tree = BTree::create(&mut pager, OWNER).expect("create tree");
        pager.flush_log();
        w.lock().unwrap().root = Some(tree.root());
    });
    assert_eq!(Outcome::Finished, outcome);
    let mut crashes = 0;
    for round in 0..rounds {
        let at = sim.now()
            + Duration::from_micros(2_000 + (seed * 7919 + round as u64 * 104_729) % 200_000);
        sim.crash_at(at);
        let w = world.clone();
        if sim.run(move |rt| crash_workload(rt, &w, 600)) == Outcome::Crashed {
            crashes += 1;
        }
        let root = world.lock().unwrap().root.unwrap();
        let found = Arc::new(Mutex::new(None));
        let f = found.clone();
        let outcome = sim.run(move |rt| {
            let (mut pager, _) = Pager::open(rt, &crash_options()).expect("reopen");
            *f.lock().unwrap() = Some(BTree::open(root, OWNER).verify(&mut pager));
        });
        assert_eq!(Outcome::Finished, outcome);
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
    crashes
}

#[test]
fn a_crashed_tree_recovers_a_prefix_of_its_operations() {
    let mut crashes = 0;
    for seed in 0..6 {
        crashes += crash_loop(seed, 6);
    }
    assert!(crashes >= 10, "only {crashes} crashes");
}
