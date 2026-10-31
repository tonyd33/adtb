use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use adtb::btree::{BTree, Index};
use adtb::heap::{FieldType, Flags, Heap, LargeStore};
use adtb::key::{CtorId, Dir, Entry, KeySchema, KeyType, Scalar, Tid, Value};
use adtb::mvcc::Transactions;
use adtb::runtime::Threads;
use adtb::storage::{PageKind, Pager, Pid, Quarantine};
use adtb::verify::{self, Inventory, Problem, Report, Sum};
use adtb::write::{Txn, VerifyScope};
use adtb::{CallOptions, Capability, Database, Log};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};

use adtb_sim::Faults;
use adtb_simkit::{api, crash, harness};

use harness::{finished, options};

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
            let tid = db.heaps[h].insert(&db.pager, x.0, &row).unwrap();
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

    fn repair(&self, report: &Report) -> Result<usize, adtb::storage::PageError> {
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

fn a_large_value(db: &Db) -> adtb::heap::Pointer {
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
fn orphan_a_successor(db: &Db) -> (adtb::mvcc::Xid, Tid) {
    let at = db
        .index
        .probe(&db.pager, &[Value::I64(2)])
        .unwrap()
        .remove(0)
        .tid;
    let x = db.txns.begin(&db.pager).unwrap();
    let row = vec![Value::I64(1002), Value::Bytes(vec![2; 40])];
    let u = db.heaps[0].update(&db.pager, at, x.0, &row, &[0]).unwrap();
    assert!(!u.heap_only);
    let gone = |_: &Pager<SimRuntime>, h: &adtb::heap::Header| {
        Ok(if h.xmin == x.0 {
            adtb::heap::Fate::Remove
        } else {
            adtb::heap::Fate::Keep
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

fn open_app(rt: SimRuntime) -> Database<SimRuntime> {
    Database::open_in(rt, NAME, &Log::new([APP]), &api::options(2)).expect("the database opens")
}

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

fn findings(db: &Database<SimRuntime>) -> Vec<String> {
    api::sorted(api::result(db, "from f in sys::verify_findings select f"))
}

/// Returns the index of `relation` of `clause`.
fn index(db: &Database<SimRuntime>, relation: &str, clause: &str) -> Index {
    db.write_path()
        .index_named(relation, clause)
        .expect("the index exists")
}

/// Returns the entry of `index` of key `key`.
fn entry(db: &Database<SimRuntime>, index: &Index, key: Value) -> Entry {
    index
        .probe(db.write_path().pager(), &[key])
        .unwrap()
        .remove(0)
}

/// Add an entry of key 1000 to `relation`'s key index, naming a slot that holds nothing.
fn plant_orphan(db: &Database<SimRuntime>, relation: &str, ctor: &str) -> Entry {
    let pager = db.write_path().pager();
    let heap = db.write_path().heap_named(relation, ctor).unwrap();
    let page = heap.pages(pager).unwrap()[0];
    let key_index = index(db, relation, "key id");
    let ctor = *key_index.ctors().iter().next().unwrap();
    let orphan = Entry {
        key: vec![Value::I64(1000)],
        ctor,
        tid: Tid::new(page.0, 400).unwrap(),
    };
    assert!(key_index.insert(pager, &orphan).unwrap());
    orphan
}

/// Commit what `f` writes beneath the checks of the write path, as one transaction.
fn beneath(db: &Database<SimRuntime>, f: impl FnOnce(&Txn)) {
    let w = db.write_path();
    let tx = w.begin().unwrap();
    f(&tx);
    w.commit(tx).unwrap();
}

/// Returns the first page of the heap of `users`.
fn users_page(db: &Database<SimRuntime>) -> Pid {
    let w = db.write_path();
    let heap = w.heap_named("app::users", "User").unwrap();
    heap.pages(w.pager()).unwrap()[0]
}

#[test]
fn a_clean_database_has_no_findings() {
    api::once(40, |rt| {
        let db = open_app(rt);
        seed_app(&db);
        api::results(
            &db,
            r#"update u in users where key u == users.key(1) set User { u with name: "renamed" }"#,
        );
        api::results(
            &db,
            "delete i in identities where key i == identities.key(3)",
        );
        api::results(
            &db,
            r#"update i in identities.Google where key i == identities.key(4)
               set Google { i with gid: "hot" }"#,
        );
        assert_eq!(Vec::<String>::new(), verified(&db, None));
        assert_eq!(Vec::<String>::new(), verified(&db, Some("users")));
        assert_eq!(Vec::<String>::new(), findings(&db));
    });
}

#[test]
fn a_retag_and_a_removed_constructor_leave_no_finding() {
    let add = "module app\nadd constructor Identity.Apple { sub: Text }\nenable identities.Apple\n";
    let deprecate = "module app\ndeprecate identities.Apple\n";
    let remove = "module app\nremove constructor Identity.Apple\n";
    api::once(41, |rt| {
        let db = Database::open_in(
            rt,
            NAME,
            &Log::new([APP, add, deprecate, remove]),
            &api::options(2),
        )
        .expect("the database opens");
        seed_app(&db);
        api::results(
            &db,
            r"update i in identities.Google where key i == identities.key(2)
               set EmailPass { i with email: i.gid }",
        );
        assert_eq!(Vec::<String>::new(), verified(&db, None));
    });
}

#[test]
fn a_flipped_page_is_a_checksum_finding() {
    let mut sim = Sim::new(42, Config::default());
    let page = api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed_app(&db);
        let page = users_page(&db);
        db.close().unwrap();
        page
    });
    let mut files = sim.files();
    let size = PAGE_SIZE;
    files.get_mut(NAME).unwrap()[page.0 as usize * size + 300] ^= 0x10;
    let mut sim = Sim::with_files(42, Config::default(), files);
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        let got = verified(&db, None);
        assert_eq!(1, got.len(), "{got:?}");
        assert!(got[0].starts_with("Checksum"), "{got:?}");
    });
}

#[test]
fn a_lost_write_is_a_stale_page_finding() {
    let mut sim = Sim::new(43, Config::default());
    let page = api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed_app(&db);
        let page = users_page(&db);
        db.close().unwrap();
        page
    });
    let before = sim.files();
    let mut sim = Sim::with_files(43, Config::default(), before.clone());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        api::results(
            &db,
            r#"update u in users where key u == users.key(1) set User { u with name: "later" }"#,
        );
        db.close().unwrap();
    });
    let mut files = sim.files();
    let size = PAGE_SIZE;
    let at = page.0 as usize * size;
    let old = before[NAME][at..at + size].to_vec();
    files.get_mut(NAME).unwrap()[at..at + size].copy_from_slice(&old);
    let mut sim = Sim::with_files(43, Config::default(), files);
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        let got = verified(&db, None);
        assert_eq!(1, got.len(), "{got:?}");
        assert!(got[0].starts_with("StalePage"), "{got:?}");
    });
}

#[test]
fn a_misordered_leaf_is_an_order_finding() {
    api::once(44, |rt| {
        let db = open_app(rt);
        seed_app(&db);
        let tree = index(&db, "app::users", "index (name)").tree();
        let w = db.write_path();
        let leaf = *tree.pages(w.pager()).unwrap().last().unwrap();
        tree.misorder_leaf(w.pager(), leaf).unwrap();
        let got = verified(&db, None);
        assert_eq!(1, got.len(), "{got:?}");
        assert!(got[0].starts_with("Order"), "{got:?}");
    });
}

#[test]
fn a_dropped_entry_is_a_missing_finding() {
    api::once(45, |rt| {
        let db = open_app(rt);
        seed_app(&db);
        let ix = index(&db, "app::identities", "index (user)");
        let gone = entry(&db, &ix, Value::I64(5));
        assert!(ix.delete(db.write_path().pager(), &gone).unwrap());
        let got = verified(&db, None);
        assert_eq!(1, got.len(), "{got:?}");
        assert!(got[0].starts_with("Missing"), "{got:?}");
    });
}

#[test]
fn an_entry_of_a_constructor_outside_its_set_is_an_orphan_finding() {
    api::once(46, |rt| {
        let db = open_app(rt);
        seed_app(&db);
        let ix = index(&db, "app::identities", "index (user) in Google");
        let mut stray = entry(&db, &ix, Value::I64(5));
        stray.ctor = CtorId(u32::MAX >> 4);
        let bytes = ix.schema().encode_entry(&stray);
        ix.tree().insert(db.write_path().pager(), &bytes).unwrap();
        let got = verified(&db, None);
        assert_eq!(1, got.len(), "{got:?}");
        assert!(got[0].starts_with("Orphan"), "{got:?}");
    });
}

/// Returns the TIDs of the versions of keys `a` and `b` of `users`, on one page, the first in the
/// later slot.
fn stray_ends(db: &Database<SimRuntime>, a: i64, b: i64) -> (Entry, Tid) {
    let ix = index(db, "app::users", "key id");
    let from = entry(db, &ix, Value::I64(a));
    let to = entry(db, &ix, Value::I64(b));
    assert_eq!(from.tid.page(), to.tid.page());
    assert!(from.tid.slot() > to.tid.slot());
    (from, to.tid)
}

#[test]
fn a_stray_next_to_a_version_that_is_not_heap_only_is_reported_with_no_false_missing() {
    api::once(146, |rt| {
        let db = open_app(rt);
        seed_app(&db);
        let (from, to) = stray_ends(&db, 9, 2);
        let w = db.write_path();
        let heap = w.heap_named("app::users", "User").unwrap();
        heap.plant_next(w.pager(), from.tid, to).unwrap();
        let got = verified(&db, None);
        assert_eq!(1, got.len(), "{got:?}");
        assert!(got[0].starts_with("Structure"), "{got:?}");
        assert!(got[0].contains("hot-updated"), "{got:?}");
    });
}

#[test]
fn a_stray_next_lets_no_orphan_pass_on_the_row_it_names() {
    api::once(147, |rt| {
        let db = open_app(rt);
        seed_app(&db);
        let (from, to) = stray_ends(&db, 9, 2);
        let w = db.write_path();
        let heap = w.heap_named("app::users", "User").unwrap();
        heap.plant_next(w.pager(), from.tid, to).unwrap();
        let orphan = Entry {
            key: vec![Value::I64(2)],
            ..from
        };
        assert!(
            index(&db, "app::users", "key id")
                .insert(w.pager(), &orphan)
                .unwrap()
        );
        let got = verified(&db, None);
        assert_eq!(2, got.len(), "{got:?}");
        assert!(got.iter().any(|r| r.starts_with("Structure")), "{got:?}");
        assert!(got.iter().any(|r| r.starts_with("Orphan")), "{got:?}");
    });
}

/// Plant in the version of identity `from` a stray `next` to the heap-only successor of identity
/// 5, and expect verify to report it and nothing else.
fn stray_into_a_chain(seed: u64, from: i64) {
    api::once(seed, |rt| {
        let db = open_app(rt);
        seed_app(&db);
        api::results(
            &db,
            r#"update i in identities where key i == identities.key(5)
               set Google { i with gid: "hot" }"#,
        );
        let w = db.write_path();
        let heap = w.heap_named("app::identities", "Google").unwrap();
        let mut heap_only = None;
        heap.scan_pages(w.pager(), &heap.pages(w.pager()).unwrap(), |v| {
            if v.flags.contains(Flags::HEAP_ONLY) {
                heap_only = Some(v.tid);
            }
        })
        .unwrap();
        let heap_only = heap_only.expect("the update is heap-only");
        let from = entry(
            &db,
            &index(&db, "app::identities", "key id"),
            Value::I64(from),
        );
        assert_eq!(from.tid.page(), heap_only.page());
        heap.plant_next(w.pager(), from.tid, heap_only).unwrap();
        let got = verified(&db, None);
        assert_eq!(1, got.len(), "{got:?}");
        assert!(got[0].starts_with("Structure"), "{got:?}");
    });
}

#[test]
fn a_stray_next_into_a_later_chain_is_reported_with_no_false_missing() {
    stray_into_a_chain(148, 1);
}

#[test]
fn a_stray_next_into_an_earlier_chain_is_reported_with_no_false_missing() {
    stray_into_a_chain(149, 9);
}

/// Run the hot update of identity 5 and `other`, on identity 6, in one transaction. Then plant in
/// the version of identity 6 a stray `next` to the heap-only successor of identity 5.
fn stray_under_one_writer(db: &Database<SimRuntime>, other: &str) {
    let mut t = db.begin(&adtb::TxnOptions::default()).unwrap();
    api::in_txn_results(
        &mut t,
        r#"update i in identities where key i == identities.key(5)
           set Google { i with gid: "hot" }"#,
    );
    api::in_txn_results(&mut t, other);
    t.commit(None).unwrap();
    let w = db.write_path();
    let heap = w.heap_named("app::identities", "Google").unwrap();
    let ix = index(db, "app::identities", "key id");
    let root = entry(db, &ix, Value::I64(5)).tid;
    let mut successor = None;
    heap.scan_pages(w.pager(), &heap.pages(w.pager()).unwrap(), |v| {
        if v.tid == root {
            successor = v.header.next;
        }
    })
    .unwrap();
    let successor = successor.expect("the update is heap-only");
    let from = entry(db, &ix, Value::I64(6)).tid;
    assert_eq!(from.page(), successor.page());
    heap.plant_next(w.pager(), from, successor).unwrap();
}

#[test]
fn a_stray_next_from_a_version_its_writer_deleted_is_reported_once() {
    api::once(150, |rt| {
        let db = open_app(rt);
        seed_app(&db);
        stray_under_one_writer(
            &db,
            "delete i in identities where key i == identities.key(6)",
        );
        for got in [
            verified(&db, None),
            api::printed(db.verify_holding().unwrap()),
        ] {
            assert_eq!(1, got.len(), "{got:?}");
            assert!(got[0].starts_with("Structure"), "{got:?}");
            assert!(got[0].contains("more than one slot"), "{got:?}");
        }
    });
}

#[test]
fn a_stray_next_from_a_version_its_writer_updated_is_reported_with_the_row_it_cuts_off() {
    api::once(151, |rt| {
        let db = open_app(rt);
        seed_app(&db);
        stray_under_one_writer(
            &db,
            r#"update i in identities where key i == identities.key(6)
               set Google { i with gid: "hot" }"#,
        );
        for got in [
            verified(&db, None),
            api::printed(db.verify_holding().unwrap()),
        ] {
            let (structure, missing): (Vec<_>, Vec<_>) =
                got.iter().partition(|r| r.starts_with("Structure"));
            assert_eq!(1, structure.len(), "{got:?}");
            assert!(structure[0].contains("more than one slot"), "{got:?}");
            assert!(!missing.is_empty(), "{got:?}");
            assert!(missing.iter().all(|r| r.starts_with("Missing")), "{got:?}");
        }
    });
}

#[test]
fn a_holding_run_reports_a_stray_across_writers_once() {
    api::once(152, |rt| {
        let db = open_app(rt);
        seed_app(&db);
        let (from, to) = stray_ends(&db, 9, 2);
        let w = db.write_path();
        let heap = w.heap_named("app::users", "User").unwrap();
        heap.plant_next(w.pager(), from.tid, to).unwrap();
        let got = api::printed(db.verify_holding().unwrap());
        assert_eq!(1, got.len(), "{got:?}");
        assert!(got[0].starts_with("Structure"), "{got:?}");
        assert!(got[0].contains("hot-updated"), "{got:?}");
    });
}

#[test]
fn a_run_finds_nothing_under_writers_of_hot_chains_some_rolled_back() {
    for seed in adtb_simkit::seeds::sweep(0..1) {
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
                            let mut t = db.begin(&adtb::TxnOptions::default()).unwrap();
                            let run = |t: &mut adtb::AdHocTransaction<'_, SimRuntime>| {
                                t.query_text(&text, &[], &adtb::TxnCallOptions::default())
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
            let mut t = db.begin(&adtb::TxnOptions::default()).unwrap();
            api::in_txn_results(
                &mut t,
                r#"update i in identities where key i == identities.key(1)
                   set Google { i with gid: "rolled back" }"#,
            );
            t.rollback().unwrap();
            run_text(&db, "vacuum identities");
            assert_eq!(Vec::<String>::new(), verified(&db, None), "seed {seed}");
            let got = api::printed(db.verify_holding().unwrap());
            assert_eq!(Vec::<String>::new(), got, "seed {seed}");
        });
    }
}

#[test]
fn an_entry_naming_an_empty_slot_is_an_orphan_finding() {
    api::once(47, |rt| {
        let db = open_app(rt);
        seed_app(&db);
        plant_orphan(&db, "app::users", "User");
        let got = verified(&db, None);
        assert_eq!(1, got.len(), "{got:?}");
        assert!(got[0].starts_with("Orphan"), "{got:?}");
    });
}

#[test]
fn verify_is_refused_and_audited_for_a_role_when_a_violation_holds_a_witness() {
    api::once(49, |rt| {
        let db = open_app(rt.clone());
        seed_app(&db);
        let authority = db.role_authority();
        authority.create_role("dba").unwrap();
        for capability in [Capability::Adhoc, Capability::Maintain] {
            authority
                .grant_capability("dba", capability, false)
                .unwrap();
        }
        let dba = authority.session("dba").unwrap();
        api::task(&rt, &dba, "verify").expect("a sound database verifies");
        let w = db.write_path();
        let heap = w.heap_named("app::users", "User").unwrap();
        let ix = index(&db, "app::users", "key id");
        let id = entry(&db, &ix, Value::I64(1)).ctor;
        beneath(&db, |tx| {
            let row = vec![Value::I64(99), Value::Text(String::new())];
            let tid = heap.insert(w.pager(), tx.xid().unwrap().0, &row).unwrap();
            let by_id = Entry {
                key: vec![Value::I64(99)],
                ctor: id,
                tid,
            };
            assert!(ix.insert(w.pager(), &by_id).unwrap());
        });
        let got = api::task(&rt, &dba, "verify");
        assert!(format!("{got:?}").contains("Denied"), "{got:?}");
        assert!(!format!("{got:?}").contains("key(99)"), "{got:?}");
        assert_eq!(
            1,
            api::query(&db, "from a in catalog::audit.Denied select a.seq").len()
        );
        assert_eq!(
            1,
            api::query(&db, "from a in catalog::audit.Call select a.seq").len(),
            "the first verify succeeded; the refused one has only its Denied row"
        );
    });
}

#[test]
fn a_witness_refused_on_a_marked_database_is_denied_with_no_audit_row() {
    api::once(64, |rt| {
        let db = open_app(rt.clone());
        seed_app(&db);
        let authority = db.role_authority();
        authority.create_role("dba").unwrap();
        for capability in [Capability::Adhoc, Capability::Maintain] {
            authority
                .grant_capability("dba", capability, false)
                .unwrap();
        }
        let dba = authority.session("dba").unwrap();
        let w = db.write_path();
        let heap = w.heap_named("app::users", "User").unwrap();
        let ix = index(&db, "app::users", "key id");
        let id = entry(&db, &ix, Value::I64(1)).ctor;
        beneath(&db, |tx| {
            let row = vec![Value::I64(99), Value::Text(String::new())];
            let tid = heap.insert(w.pager(), tx.xid().unwrap().0, &row).unwrap();
            let by_id = Entry {
                key: vec![Value::I64(99)],
                ctor: id,
                tid,
            };
            assert!(ix.insert(w.pager(), &by_id).unwrap());
        });
        w.pager().quarantine(Quarantine::Database).unwrap();
        let got = api::task(&rt, &dba, "verify");
        assert!(format!("{got:?}").contains("Denied"), "{got:?}");
        assert!(!format!("{got:?}").contains("recording"), "{got:?}");
    });
}

#[test]
fn a_row_beneath_the_constraint_check_is_a_violation_with_it_as_witness() {
    api::once(48, |rt| {
        let db = open_app(rt);
        seed_app(&db);
        let w = db.write_path();
        let heap = w.heap_named("app::users", "User").unwrap();
        let ix = index(&db, "app::users", "key id");
        let names = index(&db, "app::users", "index (name)");
        let id = entry(&db, &ix, Value::I64(1)).ctor;
        beneath(&db, |tx| {
            let row = vec![Value::I64(99), Value::Text(String::new())];
            let tid = heap.insert(w.pager(), tx.xid().unwrap().0, &row).unwrap();
            let by_id = Entry {
                key: vec![Value::I64(99)],
                ctor: id,
                tid,
            };
            assert!(ix.insert(w.pager(), &by_id).unwrap());
            let by_name = Entry {
                key: vec![Value::Text(String::new())],
                ctor: names.ctors().iter().next().copied().unwrap(),
                tid,
            };
            assert!(names.insert(w.pager(), &by_name).unwrap());
        });
        let got = verified(&db, None);
        assert_eq!(1, got.len(), "{got:?}");
        assert!(got[0].starts_with("Violation"), "{got:?}");
        assert!(got[0].contains("users.key(99)"), "{got:?}");
    });
}

#[test]
fn a_leaked_page_is_a_structure_finding_only_where_no_writer_can_exist() {
    api::once(49, |rt| {
        let db = open_app(rt);
        seed_app(&db);
        let pager = db.write_path().pager();
        pager.allocate(1, PageKind::HeapSlotted, 3).unwrap();
        assert_eq!(Vec::<String>::new(), verified(&db, None));
        let got = api::printed(db.verify_holding().unwrap());
        assert_eq!(1, got.len(), "{got:?}");
        assert!(got[0].starts_with("Structure"), "{got:?}");
        assert!(got[0].contains("Leaked"), "{got:?}");
    });
}

/// Run `body` as the one process of a simulation that switches threads often inside page memory.
fn stressed(seed: u64, body: impl FnOnce(SimRuntime)) {
    let config = Config {
        switch_rate: 0.3,
        ..Config::default()
    };
    let mut sim = Sim::new(seed, config);
    api::run(&mut sim, body);
}

/// Open the database over a pool small enough that the checks of a run read from the file.
fn open_stressed(rt: SimRuntime) -> Database<SimRuntime> {
    let options = adtb::OpenOptions {
        pool_bytes: Some(64 * PAGE_SIZE as u64),
        ..api::options(2)
    };
    Database::open_in(rt, NAME, &Log::new([APP]), &options).expect("the database opens")
}

/// Returns whether `text` ran, or `false` when a finding of a run quarantined what it writes.
fn writes(db: &Database<SimRuntime>, text: &str) -> bool {
    match db.query_text(text, &[], &CallOptions::default()) {
        Ok(_) => true,
        Err(adtb::Error::Fatal {
            reason: adtb::FatalReason::Quarantined { .. },
        }) => false,
        Err(e) => panic!("a writer runs `{text}`: {e:?}"),
    }
}

/// Run writers on `db` until `body` ends or a finding stops them. Each updates, inserts, retags
/// and deletes its own rows.
fn with_writers(rt: &SimRuntime, db: &Database<SimRuntime>, body: impl FnOnce()) {
    let stop = Arc::new(AtomicBool::new(false));
    let workers: Vec<_> = (0..3i64)
        .map(|w| {
            let (rt2, db, stop) = (rt.clone(), db.clone(), stop.clone());
            rt.spawn(&format!("writer {w}"), move || {
                let mut n = 0i64;
                while !stop.load(Ordering::Relaxed) {
                    let row = (w + 1) * 1000 + n;
                    let text = format!(
                        r#"insert users {{ id: users.key({row}), name: "w{row}" }}
                           insert identities.Google {{ id: identities.key({row}), user: users.key({row}), gid: "g" }}"#
                    );
                    if !writes(&db, &text) {
                        return;
                    }
                    let renamed = format!(
                        r#"update u in users where key u == users.key({row}) set User {{ u with name: "r{row}" }}"#
                    );
                    if !writes(&db, &renamed) {
                        return;
                    }
                    if n % 3 == 0 {
                        let gone = format!("delete i in identities where key i == identities.key({row})");
                        if !writes(&db, &gone) || !writes(&db, "vacuum identities") {
                            return;
                        }
                    }
                    n += 1;
                    rt2.yield_now();
                }
            })
        })
        .collect();
    body();
    stop.store(true, Ordering::Relaxed);
    for t in workers {
        rt.join(t);
    }
}

#[test]
fn a_run_under_concurrent_writers_finds_nothing_when_nothing_is_wrong() {
    for seed in adtb_simkit::seeds::sweep(0..3) {
        stressed(seed, |rt| {
            let db = open_stressed(rt.clone());
            seed_app(&db);
            with_writers(&rt, &db, || {
                let got = verified(&db, None);
                assert_eq!(Vec::<String>::new(), got, "seed {seed}");
            });
            let marks = db.write_path().pager().quarantined();
            assert!(marks.is_empty(), "seed {seed}: {marks:?}");
        });
    }
}

#[test]
fn a_run_under_concurrent_writers_finds_exactly_the_one_fault_planted() {
    for seed in adtb_simkit::seeds::sweep(100..103) {
        stressed(seed, |rt| {
            let db = open_stressed(rt.clone());
            seed_app(&db);
            with_writers(&rt, &db, || {
                plant_orphan(&db, "app::users", "User");
                let got = verified(&db, None);
                assert_eq!(1, got.len(), "seed {seed}: {got:?}");
                assert!(got[0].starts_with("Orphan"), "seed {seed}: {got:?}");
            });
        });
    }
}

#[test]
fn a_run_replaces_only_the_findings_of_its_scope() {
    api::once(50, |rt| {
        let db = open_app(rt);
        seed_app(&db);
        let orphan = plant_orphan(&db, "app::users", "User");
        assert_eq!(1, verified(&db, Some("users")).len());
        assert_eq!(Vec::<String>::new(), verified(&db, Some("identities")));
        assert_eq!(1, findings(&db).len());
        assert_eq!(1, verified(&db, None).len());
        assert_eq!(2, findings(&db).len());
        let ix = index(&db, "app::users", "key id");
        assert!(ix.delete(db.write_path().pager(), &orphan).unwrap());
        assert_eq!(Vec::<String>::new(), verified(&db, Some("users")));
        assert_eq!(1, findings(&db).len(), "the run over the database stays");
        assert_eq!(Vec::<String>::new(), verified(&db, None));
        assert_eq!(Vec::<String>::new(), findings(&db));
    });
}

#[test]
fn findings_survive_a_reopen_and_the_next_run_replaces_them() {
    let mut sim = Sim::new(51, Config::default());
    let (kept, orphan) = api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed_app(&db);
        let orphan = plant_orphan(&db, "app::users", "User");
        assert_eq!(1, verified(&db, None).len());
        let kept = findings(&db);
        db.close().unwrap();
        (kept, orphan)
    });
    let mut sim = Sim::with_files(51, Config::default(), sim.files());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert_eq!(kept, findings(&db));
        let ix = index(&db, "app::users", "key id");
        assert!(ix.delete(db.write_path().pager(), &orphan).unwrap());
        assert_eq!(Vec::<String>::new(), verified(&db, None));
        db.close().unwrap();
    });
    let mut sim = Sim::with_files(51, Config::default(), sim.files());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert_eq!(Vec::<String>::new(), findings(&db));
    });
}

#[test]
fn a_running_verify_has_a_job_row_and_a_finished_one_none() {
    api::once(52, |rt| {
        let db = open_app(rt.clone());
        seed_app(&db);
        let jobs = |db: &Database<SimRuntime>| {
            api::printed(api::result(db, "from j in sys::jobs select j.kind"))
        };
        let mut writer = db.begin(&adtb::TxnOptions::default()).unwrap();
        api::in_txn_results(
            &mut writer,
            r#"insert users { id: users.key(500), name: "w" }"#,
        );
        let runner = db.clone();
        let run = rt.spawn("verify", move || {
            runner.verify(None).expect("verify runs");
        });
        while jobs(&db).is_empty() {
            rt.yield_now();
        }
        assert_eq!(vec!["Verify".to_owned()], jobs(&db));
        writer.commit(None).unwrap();
        rt.join(run);
        assert_eq!(Vec::<String>::new(), jobs(&db));
    });
}

#[test]
fn a_live_tree_check_finds_nothing_while_writers_split_and_unlink_pages() {
    for seed in adtb_simkit::seeds::sweep(200..203) {
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

#[test]
fn repeated_runs_find_nothing_while_writers_delete_and_vacuum() {
    for seed in adtb_simkit::seeds::sweep(300..302) {
        stressed(seed, |rt| {
            let db = open_stressed(rt.clone());
            seed_app(&db);
            let stop = Arc::new(AtomicBool::new(false));
            let (rt2, writer, stopped) = (rt.clone(), db.clone(), stop.clone());
            let worker = rt.spawn("churn", move || {
                let opts = CallOptions::default();
                let mut n = 0i64;
                while !stopped.load(Ordering::Relaxed) {
                    let inserts: String = (0..30)
                        .map(|i| {
                            let id = 10_000 + n * 100 + i;
                            format!("insert users {{ id: users.key({id}), name: \"c{id}\" }}\n")
                        })
                        .collect();
                    writer.query_text(&inserts, &[], &opts).expect("inserts");
                    let low = 10_000 + n * 100;
                    let gone = format!(
                        "delete u in users where key u >= users.key({low}) and key u < users.key({})",
                        low + 25
                    );
                    writer.query_text(&gone, &[], &opts).expect("deletes");
                    writer.query_text("vacuum users", &[], &opts).expect("vacuums");
                    n += 1;
                    rt2.yield_now();
                }
            });
            for round in 0..4 {
                let got = verified(&db, Some("users"));
                assert_eq!(Vec::<String>::new(), got, "seed {seed} round {round}");
            }
            stop.store(true, Ordering::Relaxed);
            rt.join(worker);
        });
    }
}

const BILLING: &str = r"module billing
add module billing
add type Status = Draft | Paid { ref: Text }
add type Invoice = { id: Key invoices, status: Status, email: Text, n: I64, score: F64, note: Text }
add relation invoices : Invoice key id generate sequence
  unique (status.Paid.ref)
  index (lower(email))
  index (score, email)
  index (n)
";

fn run_text(db: &Database<SimRuntime>, text: &str) {
    db.query_text(text, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}`: {e}"));
}

#[test]
fn verify_is_refused_in_any_open_transaction_and_in_text_that_writes() {
    api::once(60, |rt| {
        let db = open_app(rt);
        seed_app(&db);
        for options in [
            adtb::TxnOptions::default(),
            adtb::TxnOptions {
                read_only: true,
                ..adtb::TxnOptions::default()
            },
        ] {
            let mut t = db.begin(&options).unwrap();
            let e = t
                .query_text("verify", &[], &adtb::TxnCallOptions::default())
                .expect_err("verify in a transaction");
            assert!(api::is_invalid(&e), "{e:?}");
            t.rollback().unwrap();
        }
        let e = api::failure(
            &db,
            "insert users { id: users.key(700), name: \"w\" }\nverify",
        );
        assert!(api::is_invalid(&e), "{e:?}");
        assert_eq!(
            Vec::<String>::new(),
            api::query(
                &db,
                "from u in users where key u == users.key(700) select u.name"
            )
        );
    });
}

#[test]
fn two_runs_waiting_on_one_writer_have_a_job_row_each_and_both_finish() {
    api::once(61, |rt| {
        let db = open_app(rt.clone());
        seed_app(&db);
        let jobs = |db: &Database<SimRuntime>| {
            api::printed(api::result(db, "from j in sys::jobs select j.id")).len()
        };
        let mut writer = db.begin(&adtb::TxnOptions::default()).unwrap();
        api::in_txn_results(
            &mut writer,
            r#"insert users { id: users.key(500), name: "w" }"#,
        );
        let runs: Vec<_> = (0..2)
            .map(|n| {
                let runner = db.clone();
                rt.spawn(&format!("verify {n}"), move || {
                    runner.verify(None).expect("verify runs");
                })
            })
            .collect();
        while jobs(&db) < 2 {
            rt.yield_now();
        }
        writer.commit(None).unwrap();
        for t in runs {
            rt.join(t);
        }
        assert_eq!(0, jobs(&db));
    });
}

#[test]
fn a_panic_in_a_run_clears_its_job_row_and_releases_its_snapshot() {
    const LOG: &str = r#"module app
add module app
add type User = { id: Key users, name: Text }
define fn boom (t: Text) -> Text = host "boom" version "1.0" deterministic
add relation users : User key id generate sequence
  index (boom(name))
"#;
    api::once(62, |rt| {
        let panicking = Arc::new(AtomicBool::new(false));
        let armed = panicking.clone();
        let mut fns = adtb::HostFns::new();
        fns.register(
            "boom",
            "1.0",
            adtb::Determinism::Deterministic,
            move |b: &adtb::batch::Batch| {
                assert!(!armed.load(Ordering::Relaxed), "boom");
                Ok(b.clone())
            },
        );
        let options = adtb::OpenOptions {
            host_fns: fns,
            ..api::options(2)
        };
        let db = Database::open_in(rt, NAME, &Log::new([LOG]), &options).unwrap();
        run_text(&db, r#"insert users { id: users.key(1), name: "a" }"#);
        run_text(&db, r#"insert users { id: users.key(2), name: "b" }"#);
        panicking.store(true, Ordering::Relaxed);
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| db.verify(None)));
        panicking.store(false, Ordering::Relaxed);
        assert!(
            !matches!(ran, Ok(Ok(_))),
            "the host function failed the run"
        );
        assert_eq!(
            Vec::<String>::new(),
            api::printed(api::result(&db, "from j in sys::jobs select j.kind"))
        );
        run_text(&db, "delete u in users where key u == users.key(2)");
        let vacuumed = api::printed(api::results(&db, "vacuum users").remove(0));
        assert!(
            vacuumed.iter().any(|r| r.contains("removed: 1")),
            "{vacuumed:?}"
        );
    });
}

#[test]
fn a_vacuum_during_the_index_phase_leaves_no_false_orphan() {
    for seed in adtb_simkit::seeds::sweep(0..4) {
        stressed(seed, |rt| {
            let db = open_stressed(rt.clone());
            let rows: String = (0..120)
                .map(|i| {
                    format!(
                        "insert users {{ id: users.key({}), name: \"c{i}\" }}\n",
                        i + 10
                    )
                })
                .collect();
            run_text(&db, &rows);
            run_text(&db, "delete u in users where key u < users.key(100)");
            let (vacuumer, rt2) = (db.clone(), rt.clone());
            let over = Arc::new(AtomicBool::new(false));
            let stop = over.clone();
            let vacuum = rt.spawn("vacuum", move || {
                loop {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let phases =
                        api::printed(api::result(&vacuumer, "from j in sys::jobs select j.phase"));
                    if phases.iter().any(|p| p.contains("indexes")) {
                        break;
                    }
                    for _ in 0..40 {
                        rt2.yield_now();
                    }
                }
                run_text(&vacuumer, "vacuum users");
            });
            let got = verified(&db, Some("users"));
            over.store(true, Ordering::Relaxed);
            rt.join(vacuum);
            assert_eq!(Vec::<String>::new(), got, "seed {seed}");
        });
    }
}

#[test]
fn a_failed_store_write_returns_the_findings_and_says_they_were_not_kept() {
    api::once(63, |rt| {
        let db = open_app(rt);
        seed_app(&db);
        plant_orphan(&db, "app::users", "User");
        db.damage_kept_store();
        let got = verified(&db, None);
        assert!(got.iter().any(|r| r.starts_with("Orphan")), "{got:?}");
        assert!(got.iter().any(|r| r.contains("were not kept")), "{got:?}");
    });
}

#[test]
fn a_tree_check_out_of_retries_goes_on_past_the_changing_pair() {
    for seed in adtb_simkit::seeds::sweep(210..213) {
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

/// Damage page `page` of `files`.
fn flip(files: &mut std::collections::BTreeMap<String, Vec<u8>>, page: Pid) {
    files.get_mut(NAME).unwrap()[page.0 as usize * PAGE_SIZE + 300] ^= 0x10;
}

#[test]
fn verify_r_reports_only_the_pages_and_constraints_of_r() {
    let mut sim = Sim::new(64, Config::default());
    let page = api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed_app(&db);
        let w = db.write_path();
        let heap = w.heap_named("app::identities", "Google").unwrap();
        let page = heap.pages(w.pager()).unwrap()[0];
        db.close().unwrap();
        page
    });
    let mut files = sim.files();
    flip(&mut files, page);
    let mut sim = Sim::with_files(64, Config::default(), files);
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert_eq!(Vec::<String>::new(), verified(&db, Some("users")));
        let got = verified(&db, Some("identities"));
        assert!(got.iter().any(|r| r.starts_with("Checksum")), "{got:?}");
    });
}

#[test]
fn verify_r_runs_only_the_constraints_that_read_r() {
    api::once(65, |rt| {
        let db = open_app(rt);
        seed_app(&db);
        let w = db.write_path();
        let heap = w.heap_named("app::users", "User").unwrap();
        let ix = index(&db, "app::users", "key id");
        let names = index(&db, "app::users", "index (name)");
        let ctor = *ix.ctors().iter().next().unwrap();
        beneath(&db, |tx| {
            let row = vec![Value::I64(99), Value::Text(String::new())];
            let tid = heap.insert(w.pager(), tx.xid().unwrap().0, &row).unwrap();
            for (index, key) in [(&ix, Value::I64(99)), (&names, Value::Text(String::new()))] {
                let e = Entry {
                    key: vec![key],
                    ctor,
                    tid,
                };
                assert!(index.insert(w.pager(), &e).unwrap());
            }
        });
        let got = verified(&db, Some("users"));
        assert!(got.iter().any(|r| r.starts_with("Violation")), "{got:?}");
        assert_eq!(Vec::<String>::new(), verified(&db, Some("identities")));
    });
}

const JOINED: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
add type Identity = { id: Key identities, user: Key users, gid: Text }
add relation identities : Identity key id generate sequence
  index (user)
add constraint joined =
  from i in identities
  where scalar_len(i.user.name) == 0
  select i
";

#[test]
fn verify_r_keeps_a_damaged_page_a_constraint_of_r_read() {
    let mut sim = Sim::new(66, Config::default());
    let open = |rt| Database::open_in(rt, NAME, &Log::new([JOINED]), &api::options(2)).unwrap();
    let page = api::run(&mut sim, |rt| {
        let db = open(rt);
        run_text(&db, r#"insert users { id: users.key(1), name: "a" }"#);
        run_text(
            &db,
            r#"insert identities { id: identities.key(1), user: users.key(1), gid: "g" }"#,
        );
        let w = db.write_path();
        let heap = w.heap_named("app::identities", "Identity").unwrap();
        let page = heap.pages(w.pager()).unwrap()[0];
        db.close().unwrap();
        page
    });
    let mut files = sim.files();
    flip(&mut files, page);
    let mut sim = Sim::with_files(66, Config::default(), files);
    api::run(&mut sim, |rt| {
        let got = verified(&open(rt), Some("users"));
        assert!(got.iter().any(|r| r.starts_with("Checksum")), "{got:?}");
    });
}

#[test]
fn a_constraint_over_the_query_memory_is_a_structure_finding() {
    api::once(67, |rt| {
        let options = adtb::OpenOptions {
            query_memory_bytes: Some(4096),
            ..api::options(2)
        };
        let log = "module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
add constraint named =
  from u in users
  where scalar_len(u.name) == 0
  select u
";
        let db = Database::open_in(rt, NAME, &Log::new([log]), &options).unwrap();
        let name = "n".repeat(2000);
        for i in 0..20 {
            run_text(
                &db,
                &format!(r#"insert users {{ id: users.key({i}), name: "{name}" }}"#),
            );
        }
        let w = db.write_path();
        let heap = w.heap_named("app::users", "User").unwrap();
        let ix = index(&db, "app::users", "key id");
        let ctor = *ix.ctors().iter().next().unwrap();
        beneath(&db, |tx| {
            for i in 100..120 {
                let row = vec![Value::I64(i), Value::Text(String::new())];
                let tid = heap.insert(w.pager(), tx.xid().unwrap().0, &row).unwrap();
                let e = Entry {
                    key: vec![Value::I64(i)],
                    ctor,
                    tid,
                };
                assert!(ix.insert(w.pager(), &e).unwrap());
            }
        });
        let got = verified(&db, None);
        assert!(
            got.iter()
                .any(|r| r.starts_with("Structure") && r.contains("query memory")),
            "{got:?}"
        );
    });
}

#[test]
fn findings_of_a_removed_relation_go_at_the_next_run_and_ids_stay_monotonic() {
    let gone = "module app\ndrop relation identities\ndrop type Identity\n";
    let plain = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
add type Identity = { id: Key identities, gid: Text }
add relation identities : Identity key id generate sequence
";
    let mut sim = Sim::new(68, Config::default());
    let first = api::run(&mut sim, |rt| {
        let db = Database::open_in(rt, NAME, &Log::new([plain]), &api::options(2)).unwrap();
        run_text(
            &db,
            r#"insert identities { id: identities.key(1), gid: "g" }"#,
        );
        plant_orphan(&db, "app::identities", "Identity");
        run_text(
            &db,
            "delete i in identities where key i == identities.key(1)",
        );
        let got = verified(&db, Some("identities"));
        assert_eq!(1, got.len(), "{got:?}");
        db.close().unwrap();
        got
    });
    let mut sim = Sim::with_files(68, Config::default(), sim.files());
    api::run(&mut sim, |rt| {
        let db = Database::open_in(rt, NAME, &Log::new([plain, gone]), &api::options(2)).unwrap();
        assert_eq!(1, findings(&db).len(), "{first:?}");
        assert_eq!(Vec::<String>::new(), verified(&db, None));
        assert_eq!(Vec::<String>::new(), findings(&db));
        db.close().unwrap();
    });
    let mut sim = Sim::with_files(68, Config::default(), sim.files());
    api::run(&mut sim, |rt| {
        let db = Database::open_in(rt, NAME, &Log::new([plain, gone]), &api::options(2)).unwrap();
        run_text(&db, r#"insert users { id: users.key(1), name: "a" }"#);
        plant_orphan(&db, "app::users", "User");
        let got = verified(&db, None);
        assert_eq!(1, got.len(), "{got:?}");
        assert!(got[0].contains("id: verify_findings.key(2)"), "{got:?}");
        assert!(
            got[0].contains("run: RunId(3)") || got[0].contains("run: 3"),
            "{got:?}"
        );
    });
}

#[test]
fn a_fault_planted_in_each_kind_of_index_is_a_missing_finding() {
    for clause in [
        "index (lower(email))",
        "index (score, email)",
        "unique (status.Paid.ref)",
    ] {
        api::once(69, |rt| {
            let db = Database::open_in(rt, NAME, &Log::new([BILLING]), &api::options(2)).unwrap();
            run_text(
                &db,
                r#"insert invoices { id: invoices.key(1), status: Paid { ref: "p" }, email: "A", n: 0, score: 0.5, note: "" }"#,
            );
            assert_eq!(Vec::<String>::new(), verified(&db, None), "{clause}");
            let ix = index(&db, "billing::invoices", clause);
            let w = db.write_path();
            let all = ix
                .range(
                    w.pager(),
                    std::ops::Bound::Unbounded,
                    std::ops::Bound::Unbounded,
                )
                .unwrap();
            assert!(ix.delete(w.pager(), &all[0]).unwrap(), "{clause}");
            let got = verified(&db, None);
            assert_eq!(1, got.len(), "{clause}: {got:?}");
            assert!(got[0].starts_with("Missing"), "{clause}: {got:?}");
        });
    }
}

#[test]
fn a_run_under_writers_of_expression_and_path_indexes_finds_nothing() {
    for seed in adtb_simkit::seeds::sweep(400..402) {
        stressed(seed, |rt| {
            let options = adtb::OpenOptions {
                pool_bytes: Some(64 * PAGE_SIZE as u64),
                ..api::options(2)
            };
            let db = Database::open_in(rt.clone(), NAME, &Log::new([BILLING]), &options).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let workers: Vec<_> = (0..3i64)
                .map(|w| {
                    let (rt2, d, s) = (rt.clone(), db.clone(), stop.clone());
                    rt.spawn(&format!("writer {w}"), move || {
                        let mut n = 0i64;
                        while !s.load(Ordering::Relaxed) {
                            let r = (w + 1) * 100_000 + n;
                            let big = if n % 5 == 0 { "y".repeat(20_000) } else { String::new() };
                            let target = format!("i in invoices where key i == invoices.key({r})");
                            run_text(
                                &d,
                                &format!(
                                    r#"insert invoices {{ id: invoices.key({r}), status: Draft, email: "A{r}", n: 0, score: 0.5, note: "{big}" }}"#
                                ),
                            );
                            run_text(&d, &format!("update {target} set {{ i with n: 1 }}"));
                            run_text(&d, &format!(r#"update {target} set {{ i with note: "m" }}"#));
                            run_text(
                                &d,
                                &format!(
                                    r#"update {target} set {{ i with status: Paid {{ ref: "p{r}" }}, email: "B{r}" }}"#
                                ),
                            );
                            if n % 2 == 0 {
                                run_text(&d, &format!("delete {target}"));
                                run_text(&d, "vacuum invoices");
                            }
                            n += 1;
                            rt2.yield_now();
                        }
                    })
                })
                .collect();
            let got = verified(&db, None);
            stop.store(true, Ordering::Relaxed);
            for t in workers {
                rt.join(t);
            }
            assert_eq!(Vec::<String>::new(), got, "seed {seed}");
        });
    }
}

#[test]
fn a_file_after_evolution_and_history_holds_clean() {
    let add = "module app\nadd constructor Identity.Apple { sub: Text }\nenable identities.Apple\n";
    let deprecate = "module app\ndeprecate identities.Apple\n";
    let remove = "module app\nremove constructor Identity.Apple\n";
    let mut sim = Sim::new(70, Config::default());
    api::run(&mut sim, |rt| {
        let db = Database::open_in(rt, NAME, &Log::new([APP, add]), &api::options(2)).unwrap();
        run_text(&db, r#"insert users { id: users.key(1), name: "a" }"#);
        run_text(
            &db,
            r#"insert identities.Apple { id: identities.key(9), user: users.key(1), sub: "s" }"#,
        );
        run_text(
            &db,
            "delete i in identities where key i == identities.key(9)",
        );
        run_text(&db, "vacuum identities");
        db.close().unwrap();
    });
    let mut sim = Sim::with_files(70, Config::default(), sim.files());
    api::run(&mut sim, |rt| {
        let log = Log::new([APP, add, deprecate, remove]);
        let db = Database::open_in(rt, NAME, &log, &api::options(2)).unwrap();
        run_text(&db, "set setting RetentionWindow = { length: 1h }");
        run_text(
            &db,
            r#"update u in users where key u == users.key(1) set User { u with name: "z" }"#,
        );
        run_text(&db, "vacuum users");
        assert_eq!(
            Vec::<String>::new(),
            api::printed(db.verify_holding().unwrap())
        );
        assert_eq!(Vec::<String>::new(), verified(&db, None));
        db.close().unwrap();
    });
}

#[test]
fn findings_survive_a_crash_and_a_later_migration() {
    let mut sim = Sim::new(71, Config::default());
    let kept = api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed_app(&db);
        plant_orphan(&db, "app::users", "User");
        assert_eq!(1, verified(&db, None).len());
        let kept = findings(&db);
        std::mem::forget(db);
        kept
    });
    let more = "module app\nadd field User.age: I64 fill 0\n";
    api::run(&mut sim, move |rt| {
        let db = Database::open_in(rt, NAME, &Log::new([APP, more]), &api::options(2)).unwrap();
        assert_eq!(kept, findings(&db));
    });
}

#[test]
fn a_file_after_a_crash_of_writers_and_vacuums_holds_clean() {
    crash::silence_fatal_panics();
    for seed in adtb_simkit::seeds::sweep(0..5) {
        let config = Config {
            faults: Faults {
                torn_p: 0.5,
                ..Faults::default()
            },
            ..Config::default()
        };
        let mut sim = Sim::new(seed, config);
        let open =
            |rt| Database::open_in(rt, NAME, &Log::new([BILLING]), &api::options(2)).unwrap();
        api::run(&mut sim, |rt| open(rt).close().unwrap());
        sim.crash_at(sim.now() + crash::crash_after(seed, 1, 2_000, 300_000));
        let outcome = crash::run_process(&mut sim, |rt| {
            let db = open(rt);
            let big = "x".repeat(30_000);
            let o = CallOptions::default();
            let mut n = 0i64;
            loop {
                let t = format!(
                    r#"insert invoices {{ id: invoices.key({n}), status: Draft, email: "A{n}", n: {n}, score: 1.5, note: "{big}" }}"#
                );
                db.query_text(&t, &[], &o).ok();
                let gone = format!(
                    "delete i in invoices where key i == invoices.key({})",
                    n / 2
                );
                db.query_text(&gone, &[], &o).ok();
                db.query_text("vacuum invoices", &[], &o).ok();
                n += 1;
            }
        });
        assert_eq!(Outcome::Crashed, outcome, "seed {seed}");
        api::run(&mut sim, |rt| {
            let db = open(rt);
            assert_eq!(Vec::<String>::new(), verified(&db, None), "seed {seed}");
            let file = api::printed(db.verify_holding().unwrap());
            assert_eq!(Vec::<String>::new(), file, "seed {seed}");
            db.close().unwrap();
        });
    }
}

const ROWS_LOG: &str = "module app
add module app
add type Row = { id: Key rows, n: I64 }
add relation rows : Row key id generate sequence
";

const LIVE_ROWS: i64 = 3_000;

/// Returns each `Orphan` finding of a check of the indexes with vacuum running beside it, over
/// many live rows and a dead one with the last key, whose slot vacuum frees while the check is
/// on the rows before it.
fn orphans_beside_vacuum(seed: u64) -> Vec<String> {
    let config = Config {
        switch_rate: 0.3,
        ..Config::default()
    };
    let mut sim = Sim::new(seed, config);
    api::run(&mut sim, |rt| {
        let db = Database::open_in(
            rt.clone(),
            "hold.adt",
            &Log::new([ROWS_LOG]),
            &api::options(2),
        )
        .unwrap();
        let digit = "[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]";
        api::results(
            &db,
            &format!(
                "insert rows from (from a in {digit}, b in {digit}, c in {digit}, d in {digit} \
                 where a * 1000 + b * 100 + c * 10 + d <= {LIVE_ROWS} \
                 select {{ n: a * 1000 + b * 100 + c * 10 + d }})"
            ),
        );
        api::results(&db, &format!("delete r in rows where r.n == {LIVE_ROWS}"));
        let started = Arc::new(AtomicBool::new(false));
        let cleaner = {
            let (db, rt, started) = (db.clone(), rt.clone(), started.clone());
            rt.clone().spawn("vacuum", move || {
                while !started.load(Ordering::Acquire) {
                    rt.yield_now();
                }
                let report = api::query(&db, "vacuum rows");
                assert!(report[0].contains("removed: 1"), "{report:?}");
            })
        };
        let w = db.write_path();
        let snapshot = w.begin_verify().unwrap();
        started.store(true, Ordering::Release);
        let scope = VerifyScope::Relation(w.relation_named("app::rows").unwrap());
        let found = w.check_indexes(snapshot.txn(), scope).unwrap();
        rt.join(cleaner);
        found
            .iter()
            .map(|f| format!("{f:?}"))
            .filter(|f| f.starts_with("Orphan"))
            .collect()
    })
}

#[test]
fn a_check_of_the_indexes_beside_vacuum_finds_no_orphan() {
    for seed in adtb_simkit::seeds::sweep(0..1) {
        let orphans = orphans_beside_vacuum(seed);
        assert!(orphans.is_empty(), "seed {seed}: {orphans:?}");
    }
}
