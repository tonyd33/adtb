use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::btree::{BTree, Index};
use crate::heap::{FieldType, Heap, LargeStore};
use crate::key::{CtorId, Dir, Entry, KeySchema, KeyType, Scalar, Tid, Value};
use crate::mvcc::{Transactions, Writers};
use crate::runtime::Threads;
use crate::storage::{PageKind, Pager, Pid};
use crate::verify::{self, Inventory, Problem, Report, Sum};
use crate::{CallOptions, Database, Log};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_testkit::process;
use adtb_testkit::process::finished;

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
        let begun = db.txns.begin(&db.pager).unwrap();
        let x = begun.xid();
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
        db.txns.commit(&db.pager, begun).unwrap().done();
        db.pager.flush_log();
        db
    }

    /// Returns what `f` makes of the inventory of the file, with `pages` the caller's own.
    fn with_inventory<T>(
        &self,
        pages: &[Pid],
        f: impl FnOnce(&Inventory<'_, SimRuntime>) -> T,
    ) -> T {
        let inventory = Inventory {
            transactions: Some(&self.txns),
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

    fn verify(&mut self) -> Report {
        self.verify_with(&[])
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
fn a_page_nothing_refers_to_is_leaked() {
    run(2, |db| {
        let lost = db.pager.allocate(1, PageKind::HeapSlotted, 10).unwrap();
        let report = db.verify();
        assert_eq!(vec![(lost, Problem::Leaked)], problems(&report));
        assert!(db.verify_with(&[lost]).is_clean());
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

/// Supersede key 2 by a running transaction, then free the successor as if it were dead.
/// Returns the transaction and the TID of the old version.
fn orphan_a_successor(db: &Db) -> (crate::mvcc::Begun, Tid) {
    let at = db
        .index
        .probe(&db.pager, &[Value::I64(2)])
        .unwrap()
        .remove(0)
        .tid;
    let begun = db.txns.begin(&db.pager).unwrap();
    let x = begun.xid();
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
    (begun, at)
}

#[test]
fn a_version_superseded_by_a_running_transaction_names_a_version_it_wrote() {
    run(12, |db| {
        let (_running, at) = orphan_a_successor(db);
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
        let (begun, _) = orphan_a_successor(db);
        db.txns.abort(&db.pager, begun).unwrap();
        assert!(db.verify().is_clean());
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
    for seed in adtb_testkit::seeds::sweep(0..1) {
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
    for seed in adtb_testkit::seeds::sweep(200..203) {
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
    for seed in adtb_testkit::seeds::sweep(210..213) {
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

fn a_large_value(db: &Db) -> crate::heap::Pointer {
    db.heaps[0].out_of_line(&db.pager).unwrap().unwrap()[0]
}

#[test]
fn a_head_counting_too_many_versions_is_overcounted() {
    run(10, |db| {
        let p = a_large_value(db);
        db.heaps[0]
            .large()
            .plant_count(&db.pager, p.first, 3)
            .unwrap();
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
    });
}
