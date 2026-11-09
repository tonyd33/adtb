use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::btree::{BTree, Index};
use crate::heap::{FieldType, HEADER_LEN, Heap, LargeStore};
use crate::key::{CtorId, Dir, Entry, KeySchema, KeyType, Scalar, Tid, Value};
use crate::mvcc::{Transactions, Writers};
use crate::runtime::Threads;
use crate::storage::{PageKind, Pager, Pid};
use crate::verify::{self, Inventory, Problem, Report, Sum};
use crate::{CallOptions, Database, Log};
use adtb_harness::process;
use adtb_harness::process::finished;
use adtb_sim::{Config, Sim, SimRuntime};

use crate::test_world as harness;
use crate::test_world::api;
use crate::test_world::load as open;

use harness::options;

const FRAMES: usize = 32;

struct Db {
    pager: Pager<SimRuntime>,
    txns: Transactions<SimRuntime>,
    heaps: Vec<Heap>,
    index: Index,
    tree: BTree,
}

impl Db {
    fn create(rt: SimRuntime) -> Db {
        let pager = Pager::create(rt.clone(), &options(FRAMES)).expect("create");
        let txns = Transactions::create(rt, &pager).unwrap();
        let large = LargeStore::create(&pager, 9).unwrap();
        let slotted = vec![
            FieldType::Scalar(Scalar::I64),
            FieldType::Scalar(Scalar::Bytes),
        ];
        let fixed = vec![
            FieldType::Scalar(Scalar::I64),
            FieldType::Scalar(Scalar::I32),
        ];
        let heaps = vec![
            Heap::create_with_fanout(&pager, 10, slotted, large, 3).unwrap(),
            Heap::create_with_fanout(&pager, 11, fixed, large, 3).unwrap(),
        ];
        let schema = KeySchema::new(vec![(KeyType::Scalar(Scalar::I64), Dir::Asc)]);
        let index =
            Index::create(&pager, 20, schema, BTreeSet::from([CtorId(0), CtorId(1)])).unwrap();
        let tree = BTree::create(&pager, 30).unwrap();
        let db = Db {
            pager,
            txns,
            heaps,
            index,
            tree,
        };
        let x = db.txns.begin(&db.pager).unwrap();
        for key in 0..300i64 {
            let h = (key % 2) as usize;
            let row = if h == 0 {
                let len = if key % 50 == 0 { 9000 } else { 40 };
                vec![Value::I64(key), Value::Bytes(vec![key as u8; len])]
            } else {
                vec![Value::I64(key), Value::I32(key as i32)]
            };
            let tid = db.heaps[h].insert(&db.pager, x, &row).unwrap();
            let entry = Entry {
                key: vec![Value::I64(key)],
                ctor: CtorId(h as u32),
                tid,
            };
            assert!(db.index.insert(&db.pager, &entry).unwrap());
            db.tree.insert(&db.pager, &key.to_be_bytes()).unwrap();
        }
        db.txns.commit(&db.pager, x).unwrap();
        db.pager.flush_log();
        db
    }

    /// Returns what `f` makes of the inventory of the file, with `pages` the caller's own.
    fn with_inventory<T>(&self, pages: &[Pid], f: impl FnOnce(&Inventory) -> T) -> T {
        let clog = self.txns.commit_log();
        let inventory = Inventory {
            commit_log: Some(&clog),
            sums: vec![Sum {
                heaps: vec![(CtorId(0), &self.heaps[0]), (CtorId(1), &self.heaps[1])],
                indexes: vec![&self.index],
            }],
            trees: vec![self.tree],
            pages: pages.to_vec(),
        };
        f(&inventory)
    }

    fn verify_with(&mut self, pages: &[Pid]) -> Report {
        self.with_inventory(pages, |i| verify::verify(&self.pager, i))
    }

    fn repair(&self, report: &Report) -> Result<usize, crate::storage::PageError> {
        self.with_inventory(&[], |i| verify::repair(&self.pager, i, report))
    }

    fn verify(&mut self) -> Report {
        self.verify_with(&[])
    }

    fn a_heap_page(&mut self, h: usize) -> Pid {
        self.heaps[h].pages(&self.pager).unwrap()[1]
    }
}

fn run(seed: u64, body: impl FnOnce(&mut Db)) {
    let mut sim = Sim::new(seed, Config::default());
    finished(&mut sim, |rt| body(&mut Db::create(rt)));
}

fn problems(report: &Report) -> Vec<(Pid, Problem)> {
    report
        .findings
        .iter()
        .map(|f| (f.pid, f.problem.clone()))
        .collect()
}

#[test]
fn a_sound_file_verifies_clean() {
    run(1, |db| {
        let report = db.verify();
        assert!(report.is_clean() && report.complete, "{report:#?}");
        assert!(!report.commits.is_empty());
    });
}

#[test]
fn a_page_nothing_refers_to_is_leaked_and_repair_frees_it() {
    run(2, |db| {
        let lost = db.pager.allocate(1, PageKind::HeapSlotted, 10).unwrap();
        let report = db.verify();
        assert_eq!(vec![(lost, Problem::Leaked)], problems(&report));
        assert_eq!(Ok(1), db.repair(&report));
        assert!(db.verify().is_clean());
        assert_eq!(Ok(lost), db.pager.allocate(1, PageKind::HeapSlotted, 10));
        assert!(db.verify_with(&[lost]).is_clean());
    });
}

#[test]
fn a_freed_page_a_heap_still_names_is_unallocated() {
    run(3, |db| {
        let pid = db.a_heap_page(1);
        db.pager.free(pid).unwrap();
        let report = db.verify();
        assert!(
            problems(&report).contains(&(pid, Problem::Unallocated)),
            "{report:#?}"
        );
    });
}

#[test]
fn a_page_two_structures_name_is_shared() {
    run(4, |db| {
        let pid = db.a_heap_page(0);
        let report = db.verify_with(&[pid]);
        assert_eq!(
            vec![(pid, Problem::Shared("heap 10".into(), "the caller".into()))],
            problems(&report)
        );
    });
}

#[test]
fn a_broken_slot_table_is_named_by_its_page() {
    run(5, |db| {
        let pid = db.a_heap_page(0);
        db.pager.update(&[pid], |p| p[0][40 + 4] = 9).unwrap();
        let report = db.verify();
        assert!(!report.findings.is_empty());
        assert!(
            report.findings.iter().all(|f| f.pid == pid
                && matches!(&f.problem, Problem::Structure(s) if s.starts_with("heap 10"))),
            "{report:#?}"
        );
    });
}

#[test]
fn an_entry_naming_an_empty_slot_is_named_by_the_page_it_names() {
    run(6, |db| {
        let pid = db.a_heap_page(1);
        let entry = Entry {
            key: vec![Value::I64(1000)],
            ctor: CtorId(1),
            tid: Tid::new(pid.0, 400).unwrap(),
        };
        db.index.insert(&db.pager, &entry).unwrap();
        let report = db.verify();
        let found = problems(&report);
        assert_eq!(1, found.len(), "{report:#?}");
        assert!(
            matches!(&found[0], (p, Problem::Structure(s)) if *p == pid && s.starts_with("index 20"))
        );
    });
}

#[test]
fn a_tree_page_of_another_owner_breaks_the_tree_and_hides_no_leak() {
    run(7, |db| {
        let pages = db.tree.pages(&db.pager).unwrap();
        let leaf = *pages.last().unwrap();
        db.pager
            .update(&[leaf], |p| {
                p[0][16..20].copy_from_slice(&31u32.to_le_bytes());
            })
            .unwrap();
        let report = db.verify();
        assert!(!report.complete);
        assert!(
            problems(&report).iter().any(|(p, f)| *p == leaf
                && matches!(f, Problem::Structure(s) if s.starts_with("tree 0"))),
            "{report:#?}"
        );
        assert!(report.leaked().is_empty());
    });
}

#[test]
#[should_panic(expected = "cannot name leaks")]
fn repair_refuses_an_incomplete_report() {
    run(8, |db| {
        let leaf = *db.tree.pages(&db.pager).unwrap().last().unwrap();
        db.pager
            .update(&[leaf], |p| {
                p[0][16..20].copy_from_slice(&31u32.to_le_bytes());
            })
            .unwrap();
        let report = db.verify();
        db.repair(&report).unwrap();
    });
}

#[test]
fn a_check_without_the_log_reads_only_the_structures() {
    run(9, |db| {
        let lost = db.pager.allocate(1, PageKind::HeapSlotted, 10).unwrap();
        let report = db.with_inventory(&[], |i| {
            verify::verify_with(&db.pager, i, verify::Depth::STRUCTURE)
        });
        assert_eq!(vec![(lost, Problem::Leaked)], problems(&report));
        assert!(report.commits.is_empty() && report.log.is_empty());
    });
}

fn a_large_value(db: &Db) -> crate::heap::Pointer {
    db.heaps[0].out_of_line(&db.pager).unwrap().unwrap()[0]
}

#[test]
fn a_head_counting_too_many_versions_is_repaired() {
    run(10, |db| {
        let p = a_large_value(db);
        db.heaps[0].large().recount(&db.pager, p.first, 3).unwrap();
        let report = db.verify();
        assert_eq!(
            vec![(
                Pid(p.first.page()),
                Problem::Overcounted {
                    slot: p.first.slot(),
                    count: 3,
                    versions: 1
                }
            )],
            problems(&report)
        );
        assert_eq!(Ok(1), db.repair(&report));
        assert!(db.verify().is_clean());
    });
}

#[test]
fn a_head_counting_too_few_versions_is_a_fault() {
    run(11, |db| {
        let p = a_large_value(db);
        db.heaps[0].large().recount(&db.pager, p.first, 0).unwrap();
        let report = db.verify();
        let found = problems(&report);
        assert_eq!(1, found.len(), "{report:#?}");
        assert!(matches!(&found[0], (pid, Problem::Structure(s))
            if *pid == Pid(p.first.page()) && s.contains("counts 0 versions, 1 point at it")));
    });
}

/// Supersede key 2 by a running transaction, then free the successor as if it were dead.
/// Returns the transaction and the TID of the old version.
fn orphan_a_successor(db: &Db) -> (crate::mvcc::Xid, Tid) {
    let at = db
        .index
        .probe(&db.pager, &[Value::I64(2)])
        .unwrap()
        .remove(0)
        .tid;
    let x = db.txns.begin(&db.pager).unwrap();
    let row = vec![Value::I64(1002), Value::Bytes(vec![2; 40])];
    let u = db.heaps[0].update(&db.pager, at, x, &row, &[0]).unwrap();
    assert!(!u.heap_only);
    let gone = |w: &Writers| {
        Ok(if w.inserted_by(x) {
            crate::mvcc::Fate::Remove
        } else {
            crate::mvcc::Fate::Keep
        })
    };
    let done = db.heaps[0]
        .prune(&db.pager, Pid(u.tid.page()), gone, &[u.tid.slot()], false)
        .unwrap();
    assert!(done.freed >= 1);
    (x, at)
}

#[test]
fn a_version_superseded_by_a_running_transaction_names_a_version_it_wrote() {
    run(12, |db| {
        let (_, at) = orphan_a_successor(db);
        let report = db.verify();
        let found = problems(&report);
        assert_eq!(1, found.len(), "{report:#?}");
        assert!(matches!(&found[0], (pid, Problem::Structure(s))
            if *pid == Pid(at.page()) && s.contains("names no version of it")));
    });
}

#[test]
fn a_successor_freed_after_its_writer_resolved_may_dangle() {
    run(13, |db| {
        let (x, _) = orphan_a_successor(db);
        db.txns.abort(&db.pager, x).unwrap();
        assert!(db.verify().is_clean());
    });
}

#[test]
fn a_version_naming_an_unreserved_deleter_is_a_structure_fault() {
    run(14, |db| {
        let (_, at) = orphan_a_successor(db);
        let was = db.heaps[0].read(&db.pager, at).unwrap().unwrap().header;
        let reserved = db.txns.commit_log().reserved();
        let mut forged = was;
        forged.writers = Writers::inserted(was.writers.inserter()).superseded_by(reserved);
        let (mut old, mut new) = ([0; HEADER_LEN], [0; HEADER_LEN]);
        was.write(&mut old);
        forged.write(&mut new);
        db.pager
            .update(&[Pid(at.page())], |p| {
                let page = &mut p[0];
                let starts: Vec<usize> = (0..=page.len() - HEADER_LEN)
                    .filter(|&i| page[i..i + HEADER_LEN] == old)
                    .collect();
                assert_eq!(1, starts.len(), "the header of {at:?} is found once");
                page[starts[0]..starts[0] + HEADER_LEN].copy_from_slice(&new);
            })
            .unwrap();
        let what = format!(
            "heap 10: slot {} names unreserved deleter {}",
            at.slot(),
            reserved.0
        );
        assert_eq!(
            vec![(Pid(at.page()), Problem::Structure(what))],
            problems(&db.verify())
        );
    });
}

const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
  index (name)
add type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text }
  | Google    { gid: Text }
add relation identities : Identity key id generate sequence
  index (user) in Google
  index (user)
add constraint named =
  from u in users
  where scalar_len(u.name) == 0
  select u
";

const NAME: &str = "verify.adt";
const PAGE_SIZE: usize = 8192;

fn seed_app(db: &Database<SimRuntime>) {
    for i in 1..=20 {
        api::results(
            db,
            &format!(
                r#"insert users {{ id: users.key({i}), name: "n{i}" }}
                   insert identities.Google {{ id: identities.key({i}), user: users.key({i}), gid: "g{i}" }}"#
            ),
        );
    }
}

fn verified(db: &Database<SimRuntime>, relation: Option<&str>) -> Vec<String> {
    api::printed(db.verify(relation).expect("verify runs"))
}

#[test]
fn a_run_finds_nothing_under_writers_of_hot_chains_some_rolled_back() {
    for seed in adtb_harness::seeds::sweep(0..1) {
        stressed(seed, |rt| {
            let db = open_stressed(rt.clone());
            seed_app(&db);
            let stop = Arc::new(AtomicBool::new(false));
            let workers: Vec<_> = (1..=3i64)
                .map(|row| {
                    let (rt2, db, stop) = (rt.clone(), db.clone(), stop.clone());
                    rt.spawn(&format!("writer {row}"), move || {
                        let text = format!(
                            r#"update i in identities where key i == identities.key({row})
                               set Google {{ i with gid: "hot" }}"#
                        );
                        let mut n = 0;
                        while !stop.load(Ordering::Relaxed) {
                            let mut t = db.begin(&crate::TxnOptions::default()).unwrap();
                            let run = |t: &mut crate::AdHocTransaction<'_, SimRuntime>| {
                                t.query_text(&text, &[], &crate::TxnCallOptions::default())
                                    .is_ok()
                            };
                            if run(&mut t) && run(&mut t) && n % 2 == 0 {
                                t.commit(None).ok();
                            } else {
                                t.rollback().ok();
                            }
                            db.query_text("vacuum identities", &[], &CallOptions::default())
                                .expect("a writer vacuums");
                            n += 1;
                            rt2.yield_now();
                        }
                    })
                })
                .collect();
            let got = verified(&db, None);
            assert_eq!(Vec::<String>::new(), got, "seed {seed} under writers");
            stop.store(true, Ordering::Relaxed);
            for t in workers {
                rt.join(t);
            }
            let mut t = db.begin(&crate::TxnOptions::default()).unwrap();
            api::in_txn_results(
                &mut t,
                r#"update i in identities where key i == identities.key(1)
                   set Google { i with gid: "rolled back" }"#,
            );
            t.rollback().unwrap();
            run_text(&db, "vacuum identities");
            assert_eq!(Vec::<String>::new(), verified(&db, None), "seed {seed}");
            let got = api::printed(db.testing().verify_holding().unwrap());
            assert_eq!(Vec::<String>::new(), got, "seed {seed}");
        });
    }
}

/// Run `body` as the one process of a simulation that switches threads often inside page memory.
fn stressed(seed: u64, body: impl FnOnce(SimRuntime)) {
    let config = Config {
        switch_rate: 0.3,
        ..Config::default()
    };
    let mut sim = Sim::new(seed, config);
    process::run(&mut sim, body);
}

/// Open the database over a pool small enough that the checks of a run read from the file.
fn open_stressed(rt: SimRuntime) -> Database<SimRuntime> {
    let options = crate::OpenOptions {
        pool_bytes: Some(64 * PAGE_SIZE as u64),
        ..open::options(2)
    };
    Database::open_in(rt, NAME, &Log::new([APP]), &options).expect("the database opens")
}

#[test]
fn a_live_tree_check_finds_nothing_while_writers_split_and_unlink_pages() {
    for seed in adtb_harness::seeds::sweep(200..203) {
        stressed(seed, |rt| {
            let db = Db::create(rt.clone());
            let tree = db.tree;
            let stop = Arc::new(AtomicBool::new(false));
            let workers: Vec<_> = (0..3u64)
                .map(|w| {
                    let (rt2, pager, stop) = (rt.clone(), db.pager.clone(), stop.clone());
                    rt.spawn(&format!("tree writer {w}"), move || {
                        let mut n = 0u64;
                        while !stop.load(Ordering::Relaxed) {
                            let keys = (0..80).map(|i| (w, n, i));
                            for (a, b, c) in keys.clone() {
                                let key = format!("{a:03}-{b:06}-{c:03}-{:0200}", 0);
                                tree.insert(&pager, key.as_bytes()).unwrap();
                            }
                            for (a, b, c) in keys {
                                let key = format!("{a:03}-{b:06}-{c:03}-{:0200}", 0);
                                tree.delete(&pager, key.as_bytes()).unwrap();
                            }
                            n += 1;
                            rt2.yield_now();
                        }
                    })
                })
                .collect();
            for round in 0..100 {
                let found = tree.check_live(&db.pager).unwrap();
                assert!(found.is_empty(), "seed {seed} round {round}: {found:?}");
            }
            stop.store(true, Ordering::Relaxed);
            for t in workers {
                rt.join(t);
            }
        });
    }
}

fn run_text(db: &Database<SimRuntime>, text: &str) {
    db.query_text(text, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}`: {e}"));
}

#[test]
fn a_tree_check_out_of_retries_goes_on_past_the_changing_pair() {
    for seed in adtb_harness::seeds::sweep(210..213) {
        stressed(seed, |rt| {
            let db = Db::create(rt.clone());
            let tree = db.tree;
            let key = |a: u64, b: u64| format!("{a:03}-{b:06}-{:0200}", 0);
            for b in 0..200 {
                tree.insert(&db.pager, key(5, b).as_bytes()).unwrap();
            }
            let leaf = *tree.pages(&db.pager).unwrap().last().unwrap();
            tree.misorder_leaf(&db.pager, leaf).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let (rt2, pager, stopped) = (rt.clone(), db.pager.clone(), stop.clone());
            let writer = rt.spawn("hot", move || {
                let mut n = 0;
                while !stopped.load(Ordering::Relaxed) {
                    let k = format!("001-{n:06}-{:0200}", 0);
                    tree.insert(&pager, k.as_bytes()).unwrap();
                    tree.delete(&pager, k.as_bytes()).unwrap();
                    n = (n + 1) % 40;
                    rt2.yield_now();
                }
            });
            for round in 0..20 {
                let found = tree.check_live_trying(&db.pager, 0).unwrap();
                assert!(
                    found.iter().any(|(p, _)| *p == leaf),
                    "seed {seed} round {round}: {found:?}"
                );
            }
            stop.store(true, Ordering::Relaxed);
            rt.join(writer);
        });
    }
}
