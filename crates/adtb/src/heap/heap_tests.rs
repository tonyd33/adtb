use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::batch::{Batch, BatchBuilder, Column, Schema};
use crate::heap::handle::Updated;
use crate::heap::{
    CtorType, Error, FieldType, Flags, Header, Heap, HeapNo, Link, max_fanout, walk,
};
use crate::key::{CtorId, Scalar, Tid, Value};
use crate::mvcc::{Fate, Writers, Xid};
use crate::runtime::{Entropy, Threads};
use crate::storage::{self, Options, PageError, PageKind, Pager, Pid};
use crate::verify::{Inventory, Sum};
use adtb_sim::{Config, Fate as LedgerFate, Ledger, Sim, SimRuntime};
use adtb_sys::runtime::Faults;

use crate::test_world as harness;
use crate::test_world::recovery;
use crate::test_world::rows;
use adtb_harness::crash;
use adtb_harness::seeds;

use crate::test_world::fixture::{Layout, Parts, Storage};
use crash::{Reopen, Rounds, crash_loop};
use harness::{PAGE, finished, options};
use recovery::verify_after_crash;
use rows::{APPLE, EMAIL, Live, Op, Rows, key_of, pair, replay, row_type};
use seeds::{share, sweep};

const GOOGLE: usize = 1;
const FRAMES: usize = 32;

fn scalar(s: Scalar) -> FieldType {
    FieldType::Scalar(s)
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

struct Db {
    pager: Pager<SimRuntime>,
    heaps: Vec<Heap>,
    ids: [HeapNo; 3],
}

impl Db {
    fn create(rt: &SimRuntime, fanout: u16) -> (Db, Layout) {
        let mut s = Storage::create(rt, &options(FRAMES));
        let large = s.large();
        for h in 0..3 {
            s.heap(row_type(h), large, fanout);
        }
        s.pager().flush_log();
        let layout = s.layout();
        (Db::of(s), layout)
    }

    fn of(s: Storage) -> Db {
        let (pager, _, heaps, _) = s.split();
        let ids = [0, 1, 2].map(|h| heaps[h].id());
        Db { pager, heaps, ids }
    }

    /// Returns what the whole-file `verify` finds, freeing leaked pages first.
    fn verify_file(&self) -> Vec<String> {
        Parts::new(&self.pager, None, &self.heaps, &[], &[]).verify()
    }
}

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
            let tid = db.heaps[*heap].insert(pager, Xid(xid), row)?;
            live.insert(*key, (*heap, tid, row.clone()));
            Ok(None)
        }
        Op::Update { key, row } => {
            let (heap, tid, _) = live[key].clone();
            let u = db.heaps[heap].update(pager, tid, Xid(xid), row, &[0])?;
            live.insert(*key, (heap, u.tid, row.clone()));
            Ok(Some(u))
        }
        Op::Delete { key } => {
            let (heap, tid, _) = live.remove(key).unwrap();
            db.heaps[heap].delete(pager, tid, Xid(xid))?;
            Ok(None)
        }
        Op::Retag { key, to, row } => {
            let (heap, tid, _) = live[key].clone();
            let (from, into) = pair(&mut db.heaps, heap, *to);
            let t = from.retag(pager, tid, Xid(xid), into, row)?;
            live.insert(*key, (*to, t, row.clone()));
            Ok(None)
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
            let deleted = v.header.writers.deleter().is_some_and(|x| committed(x.0));
            if committed(v.header.writers.inserter().0) && !deleted {
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
            let next = v.header.next.map(|n| {
                db.heaps
                    .iter()
                    .find_map(|h| {
                        walk(&db.pager, h.id(), &db.heaps, n, |_, _| {
                            Ok::<_, PageError>(true)
                        })
                        .unwrap()
                    })
                    .map(|(l, _)| l)
                    .expect("a successor in a heap of the sum")
            });
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

fn rows(live: &Live) -> Rows {
    live.iter()
        .map(|(k, (h, _, r))| (*k, (*h, r.clone())))
        .collect()
}

#[test]
fn inserts_updates_deletes_and_retags_match_a_model() {
    for seed in 0..3 {
        let mut sim = Sim::new(seed, Config::default());
        finished(&mut sim, |rt| {
            let (mut db, _) = Db::create(&rt, 4);
            let mut live = Live::new();
            let mut first: BTreeMap<i64, (u32, Tid)> = BTreeMap::new();
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
                    first.insert(key, (db.ids[live[&key].0].0, live[&key].1));
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
                let (h, t) = first[key];
                let (end, _) = walk(&db.pager, HeapNo(h), &db.heaps, t, |l, _| {
                    Ok::<_, PageError>(l.header.next.is_none())
                })
                .unwrap()
                .expect("chain ends");
                assert_eq!((db.ids[*heap], *tid), (end.heap, end.tid), "key {key}");
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
        let (mut db, _) = Db::create(&rt, 4);
        let pager = &mut db.pager;
        let row = |n: usize| {
            vec![
                Value::I64(1),
                Value::Text("a".repeat(n)),
                Value::Bytes(vec![7; n]),
            ]
        };
        let first = db.heaps[EMAIL].insert(pager, Xid(1), &row(1)).unwrap();
        let mut at = first;
        let mut pages = BTreeSet::new();
        for i in 0..30 {
            at = db.heaps[EMAIL]
                .update(pager, at, Xid(2 + i), &row(10 + 15 * i as usize), &[0])
                .unwrap()
                .tid;
            pages.insert(at.page());
        }
        assert!(pages.len() > 3, "{pages:?}");
        let (email, google) = pair(&mut db.heaps, EMAIL, GOOGLE);
        let there = email.retag(pager, at, Xid(50), google, &row(5)).unwrap();
        let back = google.retag(pager, there, Xid(50), email, &row(6)).unwrap();
        let mut links: Vec<Link> = Vec::new();
        assert_eq!(
            None,
            walk(pager, db.ids[EMAIL], &db.heaps, first, |l, _| {
                links.push(*l);
                Ok::<_, PageError>(false)
            })
            .unwrap()
        );
        assert_eq!(33, links.len());
        let tail = &links[30..];
        assert_eq!(at, tail[0].tid);
        assert_eq!(Some(there), tail[0].header.next);
        assert!(tail[0].flags.contains(Flags::MOVED));
        assert_eq!(Some(Xid(50)), tail[0].header.writers.deleter());
        assert_eq!((there, db.ids[GOOGLE]), (tail[1].tid, tail[1].heap));
        assert!(
            tail[1]
                .flags
                .contains(Flags::MOVED.with(Flags::RETAGGED_IN))
        );
        assert_eq!(Some(back), tail[1].header.next);
        assert_eq!((back, db.ids[EMAIL]), (tail[2].tid, tail[2].heap));
        assert_eq!(Flags::RETAGGED_IN, tail[2].flags);
        assert_eq!(Header::new(Xid(50)), tail[2].header);
        let read = db.heaps[EMAIL].read(pager, back).unwrap().unwrap();
        assert_eq!(row(6), read.row);
        for w in links.windows(2) {
            assert_eq!(Some(w[1].tid), w[0].header.next);
        }
        let newest = walk(pager, db.ids[EMAIL], &db.heaps, first, |l, _| {
            Ok::<_, PageError>(l.header.next.is_none())
        })
        .unwrap();
        assert_eq!(Some((back, row(6))), newest.map(|(l, r)| (l.tid, r)));
    });
}

#[test]
fn a_heap_only_update_stays_on_its_page_and_flags_both_versions() {
    let mut sim = Sim::new(2, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(&rt, 4);
        let pager = &mut db.pager;
        let heap = &mut db.heaps[APPLE];
        let t = heap
            .insert(pager, Xid(1), &[Value::I64(1), Value::I32(0)])
            .unwrap();
        let u = heap
            .update(pager, t, Xid(2), &[Value::I64(1), Value::I32(1)], &[0])
            .unwrap();
        assert!(u.heap_only);
        assert_eq!(t.page(), u.tid.page());
        let old = heap.read(pager, t).unwrap().unwrap();
        let new = heap.read(pager, u.tid).unwrap().unwrap();
        assert_eq!(Flags::HOT_UPDATED, old.flags);
        assert_eq!(Flags::HEAP_ONLY, new.flags);
        assert_eq!(
            (Some(Xid(2)), Some(u.tid)),
            (old.header.writers.deleter(), old.header.next)
        );
        let keyed = heap
            .update(pager, u.tid, Xid(3), &[Value::I64(2), Value::I32(1)], &[0])
            .unwrap();
        assert!(!keyed.heap_only);
        let new = heap.read(pager, u.tid).unwrap().unwrap();
        assert_eq!(Flags::HEAP_ONLY, new.flags);
        assert_eq!(
            Flags::NONE,
            heap.read(pager, keyed.tid).unwrap().unwrap().flags
        );
        let unindexed = heap
            .update(
                pager,
                keyed.tid,
                Xid(4),
                &[Value::I64(2), Value::I32(5)],
                &[],
            )
            .unwrap();
        assert!(unindexed.heap_only);
        heap.delete(pager, unindexed.tid, Xid(5)).unwrap();
        let gone = heap.read(pager, unindexed.tid).unwrap().unwrap();
        assert_eq!(
            (Some(Xid(5)), None),
            (gone.header.writers.deleter(), gone.header.next)
        );
    });
}

#[test]
fn a_hot_link_to_a_heap_only_version_of_another_writer_ends_the_chain() {
    let mut sim = Sim::new(2, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(&rt, 4);
        let pager = &mut db.pager;
        let heap = &mut db.heaps[APPLE];
        let row = vec![Value::I64(1), Value::I32(0)];
        let t = heap.insert(pager, Xid(1), &row).unwrap();
        let d = heap
            .insert(pager, Xid(1), &[Value::I64(2), Value::I32(0)])
            .unwrap();
        let e = heap
            .update(pager, d, Xid(2), &[Value::I64(2), Value::I32(1)], &[0])
            .unwrap();
        assert!(e.heap_only && e.tid.page() == t.page(), "{e:?}");
        heap.delete(pager, t, Xid(3)).unwrap();
        heap.plant_next(pager, t, e.tid).unwrap();
        assert_eq!(
            None,
            walk(pager, heap.id(), &db.heaps, t, |l, _| {
                Ok::<_, PageError>(l.tid == e.tid)
            })
            .unwrap()
        );
        let heap = &db.heaps[APPLE];
        assert_eq!(Some(vec![row]), heap.chain_rows(pager, t).unwrap());
        let hot: Vec<Tid> = heap
            .hot_run(pager, t)
            .unwrap()
            .iter()
            .map(|l| l.tid)
            .collect();
        assert_eq!(vec![t], hot);
        let mut roots = BTreeMap::new();
        heap.scan_rooted(pager, Pid(t.page()), |v, r| {
            roots.insert(v.tid, r);
        })
        .unwrap();
        assert_eq!(
            BTreeMap::from([(t, vec![t]), (d, vec![d]), (e.tid, vec![d])]),
            roots
        );
        assert_eq!(
            vec![format!(
                "slot {} is hot-updated but names no heap-only successor",
                t.slot()
            )],
            heap.broken_links(pager, Pid(t.page())).unwrap()
        );
    });
}

#[test]
fn an_aborted_successor_is_replaced_by_the_next_writer() {
    let mut sim = Sim::new(3, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(&rt, 4);
        let pager = &mut db.pager;
        let row = vec![Value::I64(1), Value::Text("x".into()), Value::Bytes(vec![])];
        let t = db.heaps[EMAIL].insert(pager, Xid(1), &row).unwrap();
        let (email, google) = pair(&mut db.heaps, EMAIL, GOOGLE);
        email.retag(pager, t, Xid(2), google, &row).unwrap();
        pager.abort(2, &[], |_| {}).unwrap();
        let u = email.update(pager, t, Xid(3), &row, &[0]).unwrap();
        let old = email.read(pager, t).unwrap().unwrap();
        assert_eq!(
            (Some(Xid(3)), Some(u.tid)),
            (old.header.writers.deleter(), old.header.next)
        );
        assert!(!old.flags.contains(Flags::MOVED));
        email.delete(pager, t, Xid(4)).unwrap();
        let old = email.read(pager, t).unwrap().unwrap();
        assert_eq!(
            (Some(Xid(4)), None, Flags::NONE),
            (old.header.writers.deleter(), old.header.next, old.flags)
        );
    });
}

#[test]
fn large_values_go_out_of_line_and_a_retag_keeps_their_pointers() {
    let mut sim = Sim::new(4, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(&rt, 64);
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
        let t = db.heaps[EMAIL]
            .insert(pager, Xid(1), &row("a", &big))
            .unwrap();
        let stored = large.pages(pager).unwrap().len();
        assert!(stored >= 50_000 / PAGE, "{stored} pages");
        assert_eq!(1, db.heaps[EMAIL].pages(pager).unwrap().len());
        assert_eq!(
            row("a", &big),
            db.heaps[EMAIL].read(pager, t).unwrap().unwrap().row
        );
        let (email, google) = pair(&mut db.heaps, EMAIL, GOOGLE);
        let moved = email
            .retag(pager, t, Xid(2), google, &row("b", &big))
            .unwrap();
        assert_eq!(stored, large.pages(pager).unwrap().len());
        assert_eq!(
            row("b", &big),
            google.read(pager, moved).unwrap().unwrap().row
        );
        let u = google
            .update(pager, moved, Xid(3), &row("c", &big), &[0])
            .unwrap();
        assert_eq!(stored, large.pages(pager).unwrap().len());
        let mut other = big.clone();
        other[49_999] ^= 1;
        let v = google
            .update(pager, u.tid, Xid(4), &row("c", &other), &[0])
            .unwrap();
        assert!(large.pages(pager).unwrap().len() > stored);
        assert_eq!(
            row("c", &other),
            google.read(pager, v.tid).unwrap().unwrap().row
        );
        assert_eq!(row("a", &big), email.read(pager, t).unwrap().unwrap().row);
        let mid = vec![9u8; 900];
        let w = google.insert(pager, Xid(5), &row("mid", &mid)).unwrap();
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
        let (mut db, _) = Db::create(&rt, 4);
        let pager = &mut db.pager;
        let large = db.heaps[EMAIL].large();
        let over = |e: Result<_, Error>| match e {
            Err(Error::TooLarge { len, limit }) => assert!(len > limit, "{len} over {limit}"),
            other => panic!("not too large: {other:?}"),
        };
        let wide = vec![scalar(Scalar::Decimal(0)); 300];
        over(Heap::create(pager, 20, wide.clone(), large).map(|_| ()));
        over(crate::heap::fits(pager.page_size(), wide));
        let narrow = vec![scalar(Scalar::Decimal(0)); 100];
        assert_eq!(Ok(()), crate::heap::fits(pager.page_size(), narrow));
        let empties = vec![scalar(Scalar::Text); 4000];
        over(Heap::create(pager, 22, empties, large).map(|_| ()));
        let many = vec![scalar(Scalar::Text); 400];
        let heap = Heap::create(pager, 21, many, large).unwrap();
        let row = vec![Value::Text("y".repeat(30)); 400];
        over(heap.insert(pager, Xid(1), &row).map(|_| ()));
        assert!(heap.pages(pager).unwrap().is_empty());
        assert!(
            large.pages(pager).unwrap().is_empty(),
            "a refused row writes no large value"
        );
    });
}

#[test]
fn the_directory_splits_and_its_root_stays() {
    let mut sim = Sim::new(6, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(&rt, 3);
        let root = db.heaps[EMAIL].root();
        let mut tids = Vec::new();
        for key in 0..200 {
            let row = vec![
                Value::I64(key),
                Value::Text(String::new()),
                Value::Bytes(vec![key as u8; 900]),
            ];
            tids.push((
                db.heaps[EMAIL].insert(&db.pager, Xid(1), &row).unwrap(),
                row,
            ));
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
fn an_insert_past_filled_pages_finds_room_under_another_directory_entry() {
    let mut sim = Sim::new(12, Config::default());
    finished(&mut sim, |rt| {
        let (db, _) = Db::create(&rt, 2);
        let heap = &db.heaps[EMAIL];
        let wide = |key: i64| {
            vec![
                Value::I64(key),
                Value::Text(String::new()),
                Value::Bytes(vec![key as u8; 900]),
            ]
        };
        let mut tids = Vec::new();
        for key in 0.. {
            tids.push(heap.insert(&db.pager, Xid(1), &wide(key)).unwrap());
            if heap.pages(&db.pager).unwrap().len() == 8 {
                break;
            }
        }
        let pages = heap.pages(&db.pager).unwrap();
        let on = |pid: Pid| tids.iter().filter(move |t| t.page() == pid.0).copied();
        let mut gone: Vec<Tid> = on(pages[7]).collect();
        gone.extend(on(pages[0]).take(2));
        gone.extend(on(pages[5]).take(2));
        for &t in &gone {
            heap.delete(&db.pager, t, Xid(2)).unwrap();
        }
        let judge = |w: &Writers| {
            Ok(if w.deleted_by(Xid(2)) {
                Fate::Remove
            } else {
                Fate::Keep
            })
        };
        for pid in [pages[0], pages[5], pages[7]] {
            let dead = heap.dead_slots(&db.pager, pid, judge).unwrap();
            heap.prune(&db.pager, pid, judge, &dead, false).unwrap();
        }
        assert_eq!(&pages[..7], &heap.pages(&db.pager).unwrap()[..]);
        let landed: Vec<u64> = (100..104)
            .map(|key| heap.insert(&db.pager, Xid(3), &wide(key)).unwrap().page())
            .collect();
        assert_eq!(vec![pages[0].0, pages[0].0, pages[5].0, pages[5].0], landed);
        assert_eq!(&pages[..7], &heap.pages(&db.pager).unwrap()[..]);
        assert_eq!(Vec::<String>::new(), heap.verify(&db.pager).unwrap());
    });
}

/// A row whose text goes out of line.
fn long_row(key: i64) -> Vec<Value> {
    vec![
        Value::I64(key),
        Value::Text("t".repeat(1500)),
        Value::Bytes(vec![key as u8; 700]),
    ]
}

/// Prune every page of `heap`, removing the versions `gone` deleted and marking the pages
/// all-visible.
fn settle_all_visible(db: &Db, heap: usize, gone: u64) {
    let judge = |w: &Writers| {
        Ok(if w.deleted_by(Xid(gone)) {
            Fate::Remove
        } else {
            Fate::AllVisible
        })
    };
    let heap = &db.heaps[heap];
    for pid in heap.pages(&db.pager).unwrap() {
        heap.prune(&db.pager, pid, judge, &[], true).unwrap();
    }
}

#[test]
fn writes_between_all_visible_pages_of_deep_directories_stay_within_the_latch_bound() {
    let mut sim = Sim::new(13, Config::default());
    finished(&mut sim, |rt| {
        let (db, _) = Db::create(&rt, 2);
        let mut rows = BTreeMap::new();
        for key in 0..80 {
            let h = if key % 2 == 0 { EMAIL } else { GOOGLE };
            let t = db.heaps[h]
                .insert(&db.pager, Xid(1), &long_row(key))
                .unwrap();
            rows.insert(key, (h, t));
        }
        for key in [1, 3, 5] {
            db.heaps[GOOGLE]
                .delete(&db.pager, rows[&key].1, Xid(2))
                .unwrap();
            rows.remove(&key);
        }
        settle_all_visible(&db, EMAIL, 2);
        settle_all_visible(&db, GOOGLE, 2);
        assert!(db.heaps[EMAIL].pages(&db.pager).unwrap().len() > 4);
        let (_, at) = rows[&0];
        let moved = db.heaps[EMAIL]
            .retag(&db.pager, at, Xid(3), &db.heaps[GOOGLE], &long_row(0))
            .unwrap();
        rows.insert(0, (GOOGLE, moved));
        let (_, at) = rows[&2];
        let mut changed = long_row(2);
        changed[0] = Value::I64(102);
        let u = db.heaps[EMAIL]
            .update(&db.pager, at, Xid(3), &changed, &[0])
            .unwrap();
        assert!(!u.heap_only);
        rows.insert(2, (EMAIL, u.tid));
        db.heaps[EMAIL]
            .delete(&db.pager, rows[&4].1, Xid(3))
            .unwrap();
        rows.remove(&4);
        for (key, (h, t)) in rows {
            let v = db.heaps[h].read(&db.pager, t).unwrap().unwrap();
            assert_eq!(long_row(key)[1..], v.row[1..], "key {key}");
        }
        let inventory = Inventory {
            sums: vec![Sum {
                heaps: (0..3).map(|h| (CtorId(h as u32), &db.heaps[h])).collect(),
                indexes: Vec::new(),
            }],
            ..Inventory::default()
        };
        let report = crate::verify::verify(&db.pager, &inventory);
        let wrong: Vec<_> = report
            .findings
            .iter()
            .filter(|f| !matches!(f.problem, crate::verify::Problem::Overcounted { .. }))
            .collect();
        assert!(wrong.is_empty(), "{wrong:#?}");
    });
}

#[test]
fn a_reopened_heap_finds_room_through_its_directory() {
    let mut sim = Sim::new(7, Config::default());
    finished(&mut sim, |rt| {
        let (db, _) = Db::create(&rt, 3);
        for key in 0..40 {
            let row = vec![
                Value::I64(key),
                Value::Text(String::new()),
                Value::Bytes(vec![1; 900]),
            ];
            db.heaps[EMAIL].insert(&db.pager, Xid(1), &row).unwrap();
        }
        let pages = db.heaps[EMAIL].pages(&db.pager).unwrap().len();
        let reopened = Heap::open(
            &db.pager,
            db.ids[EMAIL].0,
            db.heaps[EMAIL].root(),
            row_type(EMAIL),
            db.heaps[EMAIL].large(),
        )
        .unwrap();
        let small = vec![
            Value::I64(99),
            Value::Text(String::new()),
            Value::Bytes(vec![2; 10]),
        ];
        let t = reopened.insert(&db.pager, Xid(2), &small).unwrap();
        assert_eq!(pages, reopened.pages(&db.pager).unwrap().len());
        assert_eq!(small, reopened.read(&db.pager, t).unwrap().unwrap().row);
    });
}

#[test]
fn a_heap_scan_builds_a_batch_that_reads_back_its_rows() {
    let mut sim = Sim::new(8, Config::default());
    finished(&mut sim, |rt| {
        let (db, _) = Db::create(&rt, 4);
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
            heap.insert(&db.pager, Xid(1), r).unwrap();
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
        let (mut db, _) = Db::create(&rt, 4);
        let pager = &mut db.pager;
        let large = db.heaps[EMAIL].large();
        let mut heap = Heap::create(pager, 30, counter_type(false), large).unwrap();
        let old: Vec<Tid> = (0..20)
            .map(|k| {
                heap.insert(pager, Xid(1), &[Value::I64(k), Value::I32(-1)])
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
        heap.delete(pager, old[4], Xid(2)).unwrap();
        assert_eq!(1, storage::format(&pager.read(page).unwrap()));
        let fresh = vec![Value::I64(100), Value::I32(5), Value::Text("new".into())];
        let t = heap.insert(pager, Xid(3), &fresh).unwrap();
        assert_eq!(page.0, t.page());
        let p = &pager.read(page).unwrap().to_vec();
        assert_eq!(
            (2, PageKind::HeapSlotted as u16),
            (storage::format(p), storage::kind(p))
        );
        for (k, tid) in old.iter().enumerate() {
            let v = heap.read(pager, *tid).unwrap().unwrap();
            assert_eq!(upgraded(k as i64), v.row);
            assert_eq!((k == 4).then_some(Xid(2)), v.header.writers.deleter());
        }
        assert_eq!(fresh, heap.read(pager, t).unwrap().unwrap().row);
        assert!(heap.verify(pager).unwrap().is_empty());
    });
}

#[test]
fn a_page_whose_rows_outgrow_it_keeps_its_format_and_takes_no_insert() {
    let mut sim = Sim::new(9, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(&rt, 4);
        let pager = &mut db.pager;
        let large = db.heaps[EMAIL].large();
        let mut heap = Heap::create(pager, 31, counter_type(false), large).unwrap();
        let mut old: Vec<Tid> = Vec::new();
        for k in 0.. {
            let t = heap
                .insert(pager, Xid(1), &[Value::I64(k), Value::I32(0)])
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
                    Xid(2),
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
                Xid(3),
                &[Value::I64(7), Value::I32(9), Value::Text(String::new())],
                &[0],
            )
            .unwrap();
        assert!(!u.heap_only);
        assert_eq!(1, storage::format(&pager.read(first).unwrap()));
        assert!(heap.verify(pager).unwrap().is_empty());
    });
}

const NOTES: i64 = 40;
const NOTE_LEN: usize = 1100;

fn note_type() -> Vec<FieldType> {
    vec![scalar(Scalar::I64), scalar(Scalar::Bytes)]
}

fn long_note(row: &[Value]) -> Vec<u8> {
    vec![key_of(row) as u8; NOTE_LEN]
}

fn lengthen(mut row: Vec<Value>) -> Vec<Value> {
    row[1] = Value::Bytes(long_note(&row));
    row
}

fn tight_options() -> Options {
    Options {
        pool_frames: 8,
        ..options(FRAMES)
    }
}

/// Create the file of [`tight_options`] with a store and a heap of [`NOTES`] short notes on one
/// page, and return it with their TIDs.
fn write_notes(rt: &SimRuntime) -> (Storage, Vec<Tid>) {
    let mut s = Storage::create(rt, &tight_options());
    let large = s.large();
    let fanout = max_fanout(s.pager().page_size());
    let pager = s.pager().clone();
    let heap = s.heap(note_type(), large, fanout);
    let tids: Vec<Tid> = (0..NOTES)
        .map(|k| {
            let row = [Value::I64(k), Value::Bytes(vec![1; 8])];
            heap.insert(&pager, Xid(1), &row).unwrap()
        })
        .collect();
    assert!(tids.iter().all(|t| t.page() == tids[0].page()));
    (s, tids)
}

/// Returns the pager and the one heap of `s`, its notes able to lengthen.
fn lengthening(s: Storage) -> (Pager<SimRuntime>, Heap) {
    let (pager, _, heaps, _) = s.split();
    let [mut heap] = <[Heap; 1]>::try_from(heaps).expect("one heap");
    heap.add_format(note_type(), Box::new(lengthen)).unwrap();
    (pager, heap)
}

fn notes_inventory(heap: &Heap) -> Inventory<'_> {
    Inventory {
        sums: vec![Sum {
            heaps: vec![(CtorId(0), heap)],
            indexes: Vec::new(),
        }],
        ..Inventory::default()
    }
}

#[test]
fn a_conversion_moving_many_values_out_of_line_fits_a_pool_of_a_few_frames() {
    let mut sim = Sim::new(11, Config::default());
    finished(&mut sim, |rt| {
        let (s, tids) = write_notes(&rt);
        let (pager, heap) = lengthening(s);
        let fresh = [Value::I64(NOTES), Value::Bytes(vec![2; 8])];
        let t = heap.insert(&pager, Xid(2), &fresh).unwrap();
        assert_eq!(tids[0].page(), t.page());
        for (k, tid) in tids.iter().enumerate() {
            let v = heap.read(&pager, *tid).unwrap().unwrap();
            assert_eq!(Value::Bytes(long_note(&v.row)), v.row[1], "note {k}");
        }
        assert_eq!(
            NOTES as usize,
            heap.out_of_line(&pager).unwrap().unwrap().len()
        );
        let report = crate::verify::verify(&pager, &notes_inventory(&heap));
        assert!(report.findings.is_empty(), "{:#?}", report.findings);
    });
}

#[test]
fn a_crash_while_a_conversion_counts_its_values_leaves_no_count_too_low() {
    let (mut crashes, mut left) = (0, 0);
    for step in 0..24u64 {
        let mut sim = Sim::new(step, Config::default());
        let mut layout = None;
        finished(&mut sim, |rt| {
            let (s, _) = write_notes(&rt);
            s.pager().checkpoint();
            layout = Some(s.layout());
        });
        let layout = layout.unwrap();
        let open = |rt: SimRuntime| lengthening(Storage::reopen(&rt, &layout).unwrap());
        let out = crash_loop(
            &mut sim,
            step,
            Faults::none(512),
            Rounds::new(1, 9_000, 26_400),
            |r| {
                r.crash(|rt| {
                    let (pager, heap) = open(rt);
                    let fresh = [Value::I64(NOTES), Value::Bytes(vec![2; 8])];
                    heap.insert(&pager, Xid(2), &fresh).unwrap();
                    pager.flush_log();
                });
            },
            |rt, _| {
                let (pager, heap) = open(rt);
                let inventory = notes_inventory(&heap);
                left += usize::from(
                    !crate::verify::verify(&pager, &inventory)
                        .findings
                        .is_empty(),
                );
                let mut problems = verify_after_crash(&pager, &inventory);
                let mut rows = 0;
                heap.scan(&pager, |v| {
                    rows += 1;
                    if key_of(&v.row) < NOTES && Value::Bytes(long_note(&v.row)) != v.row[1] {
                        problems.push(format!("note {} lost its lengthened value", key_of(&v.row)));
                    }
                })
                .unwrap();
                if rows < NOTES as usize {
                    problems.push(format!("{rows} rows"));
                }
                Reopen::found(problems)
            },
        );
        crashes += out.crashed();
    }
    eprintln!("{crashes} of 24 runs crashed, {left} left findings for repair");
    assert!(crashes > 4, "{crashes} crashes");
    assert!(left > 0, "no crash left a conversion for repair to finish");
}

/// Returns whether `pager` holds a directory entry promising less than its child holds.
fn unsettled(pager: &Pager<SimRuntime>, inventory: &Inventory) -> bool {
    crate::verify::verify(pager, inventory)
        .findings
        .iter()
        .any(|f| matches!(f.problem, crate::verify::Problem::Unsettled(_)))
}

#[test]
fn a_crash_while_pruning_raises_entries_leaves_what_repair_mends() {
    let (mut crashes, mut caught) = (0, 0);
    for step in 0..40u64 {
        let mut sim = Sim::new(
            step,
            Config {
                switch_rate: 0.3,
                ..Config::default()
            },
        );
        let mut roots = None;
        finished(&mut sim, |rt| {
            let (db, r) = Db::create(&rt, 2);
            let wide = |key: i64| {
                vec![
                    Value::I64(key),
                    Value::Text(String::new()),
                    Value::Bytes(vec![key as u8; 900]),
                ]
            };
            for key in 0..400 {
                let t = db.heaps[EMAIL]
                    .insert(&db.pager, Xid(1), &wide(key))
                    .unwrap();
                if key % 4 != 0 {
                    db.heaps[EMAIL].delete(&db.pager, t, Xid(2)).unwrap();
                }
            }
            db.heaps[EMAIL].settle_directory(&db.pager).unwrap();
            db.pager.checkpoint();
            roots = Some(r);
        });
        let roots = roots.unwrap();
        let prune = |rt: SimRuntime| {
            let db = Arc::new(Db::of(Storage::reopen(&rt, &roots).unwrap()));
            let done = Arc::new(AtomicBool::new(false));
            let (d, stop, r) = (db.clone(), done.clone(), rt.clone());
            let flusher = rt.spawn("flusher", move || {
                while !stop.load(Ordering::Relaxed) {
                    d.pager.flush_log();
                    r.yield_now();
                }
            });
            let judge = |w: &Writers| {
                Ok(if w.deleted_by(Xid(2)) {
                    Fate::Remove
                } else {
                    Fate::Keep
                })
            };
            for pid in db.heaps[EMAIL].pages(&db.pager).unwrap() {
                db.heaps[EMAIL]
                    .prune(&db.pager, pid, judge, &[], false)
                    .unwrap();
            }
            done.store(true, Ordering::Relaxed);
            rt.join(flusher);
        };
        let out = crash_loop(
            &mut sim,
            step,
            Faults::none(512),
            Rounds::new(1, 85_000, 140_000),
            |r| {
                r.crash(prune);
            },
            |rt, _| {
                let db = Db::of(Storage::reopen(&rt, &roots).unwrap());
                let inventory = Inventory {
                    sums: vec![Sum {
                        heaps: (0..3).map(|h| (CtorId(h as u32), &db.heaps[h])).collect(),
                        indexes: Vec::new(),
                    }],
                    ..Inventory::default()
                };
                caught += usize::from(unsettled(&db.pager, &inventory));
                Reopen::found(verify_after_crash(&db.pager, &inventory))
            },
        );
        crashes += out.crashed();
    }
    eprintln!("{crashes} of 40 runs crashed, {caught} left entries to settle");
    assert!(crashes > 16, "{crashes} crashes");
    assert!(caught > 2, "{caught} runs left entries to settle");
}

/// What the client knows, kept outside the simulated process so it survives crashes.
struct World {
    roots: Layout,
    next_xid: u64,
    next_key: i64,
    ops: BTreeMap<u64, Op>,
}

impl World {
    /// Returns every transaction `ledger` holds as acknowledged, or as in doubt when `logged`
    /// holds it.
    fn committed(&self, ledger: &Ledger, logged: &BTreeSet<u64>) -> BTreeSet<u64> {
        self.ops
            .keys()
            .copied()
            .filter(|x| match ledger.fate(*x) {
                Some(LedgerFate::Acknowledged) => true,
                Some(LedgerFate::InDoubt) => logged.contains(x),
                _ => false,
            })
            .collect()
    }
}

fn workload(rt: SimRuntime, world: &Mutex<World>, ledger: &Ledger, steps: usize) {
    let roots = world.lock().unwrap().roots.clone();
    let Some(mut db) = Storage::reopen(&rt, &roots).ok().map(Db::of) else {
        return;
    };
    let committed = world.lock().unwrap().committed(ledger, &BTreeSet::new());
    let (mut live, _) = current(&mut db, |x| committed.contains(&x));
    let mut unacked = Vec::new();
    let ack = |db: &mut Db, unacked: &mut Vec<u64>| {
        db.pager.flush_log();
        for x in unacked.drain(..) {
            ledger.acknowledge(x);
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
                    (op, xid)
                };
                ledger.submit(xid);
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

/// Reopen the database and check it against every committed operation, those left in doubt
/// counted committed when the log holds them.
fn inspect(rt: SimRuntime, world: &Mutex<World>, ledger: &Ledger) -> Reopen {
    let roots = world.lock().unwrap().roots.clone();
    let Some(mut db) = Storage::reopen(&rt, &roots).ok().map(Db::of) else {
        return Reopen::found(vec!["open failed".into()]);
    };
    let v = db.pager.verify();
    let mut problems: Vec<String> = v.findings.iter().map(|f| format!("{f:?}")).collect();
    let logged: BTreeSet<u64> = v.commits.into_iter().collect();
    let w = world.lock().unwrap();
    let committed = w.committed(ledger, &logged);
    let mut expected = BTreeMap::new();
    for (xid, op) in &w.ops {
        if committed.contains(xid) {
            replay(&mut expected, op);
        }
    }
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
    problems.extend(db.verify_file());
    Reopen::new(logged, problems)
}

#[must_use]
fn crash_rounds(seed: u64, faults: Faults, rounds: usize) -> usize {
    let mut sim = Sim::new(seed, Config::default());
    let mut roots = None;
    finished(&mut sim, |rt| {
        let (db, r) = Db::create(&rt, 3);
        db.pager.checkpoint();
        roots = Some(r);
    });
    let world = Arc::new(Mutex::new(World {
        roots: roots.unwrap(),
        next_xid: 0,
        next_key: 0,
        ops: BTreeMap::new(),
    }));
    let rounds = Rounds::new(rounds, 2000, 80_000);
    let out = crash_loop(
        &mut sim,
        seed,
        faults,
        rounds,
        |r| {
            let (l, w) = (r.ledger().clone(), world.clone());
            r.crash(|rt| workload(rt, &w, &l, 300));
        },
        |rt, ledger| inspect(rt, &world, ledger),
    );
    let w = world.lock().unwrap();
    assert!(
        w.ops
            .keys()
            .any(|x| out.ledger().fate(*x) == Some(LedgerFate::Acknowledged)),
        "seed {seed} acknowledged nothing"
    );
    out.crashed()
}

#[test]
fn crashes_with_torn_writes_keep_every_committed_change() {
    const SEEDS: Range<u64> = 0..8;
    let crashes: usize = sweep(SEEDS)
        .map(|seed| {
            crash_rounds(
                seed,
                Faults {
                    torn_p: 0.5,
                    ..Faults::none(512)
                },
                4,
            )
        })
        .sum();
    assert!(crashes > share(SEEDS, 10), "{crashes} crashes");
}

#[test]
fn failed_writes_crash_and_recover_every_committed_change() {
    const SEEDS: Range<u64> = 100..106;
    let crashes: usize = sweep(SEEDS)
        .map(|seed| {
            crash_rounds(
                seed,
                Faults {
                    torn_p: 0.3,
                    fail_p: 0.003,
                    ..Faults::none(512)
                },
                4,
            )
        })
        .sum();
    assert!(crashes > share(SEEDS, 5), "{crashes} crashes");
}

#[test]
fn readers_with_a_stale_page_list_meet_a_freed_page_another_allocation_reserved_as_freed() {
    let mut sim = Sim::new(12, Config::default());
    finished(&mut sim, |rt| {
        let (db, _) = Db::create(&rt, 2);
        let heap = &db.heaps[EMAIL];
        let wide = |key: i64| {
            vec![
                Value::I64(key),
                Value::Text(String::new()),
                Value::Bytes(vec![key as u8; 900]),
            ]
        };
        let mut rows = BTreeMap::new();
        for key in 0.. {
            rows.insert(
                heap.insert(&db.pager, Xid(1), &wide(key)).unwrap(),
                wide(key),
            );
            if heap.pages(&db.pager).unwrap().len() == 2 * FRAMES {
                break;
            }
        }
        let pages = heap.pages(&db.pager).unwrap();
        let victim = pages[1];
        let gone: Vec<Tid> = rows
            .keys()
            .filter(|t| t.page() == victim.0)
            .copied()
            .collect();
        for &t in &gone {
            heap.delete(&db.pager, t, Xid(2)).unwrap();
            rows.remove(&t);
        }
        let judge = |w: &Writers| {
            Ok(if w.deleted_by(Xid(2)) {
                Fate::Remove
            } else {
                Fate::Keep
            })
        };
        let dead = heap.dead_slots(&db.pager, victim, judge).unwrap();
        heap.prune(&db.pager, victim, judge, &dead, false).unwrap();
        assert_eq!(
            Ok(true),
            db.pager.is_free(victim),
            "the prune freed the page"
        );
        heap.scan(&db.pager, drop).unwrap();
        let mut new = db.pager.reserve();
        assert_eq!(Ok(victim), new.page(1, PageKind::HeapSlotted, 99));
        let mut stale = BTreeMap::new();
        let scanned = heap.scan_pages(&db.pager, &pages, |v| {
            stale.insert(v.tid, v.row);
        });
        assert_eq!(
            Ok(()),
            scanned,
            "a scan of the page list taken before the free"
        );
        assert_eq!(rows, stale);
        let read = heap.read(&db.pager, gone[0]).map(|v| v.map(|v| v.row));
        assert_eq!(Ok(None), read, "a read of a row id on the freed page");
        let mut whole = BTreeMap::new();
        let queried = heap.scan(&db.pager, |v| {
            whole.insert(v.tid, v.row);
        });
        assert_eq!(Ok(()), queried, "a scan of the heap");
        assert_eq!(rows, whole);
        drop(new);
    });
}
