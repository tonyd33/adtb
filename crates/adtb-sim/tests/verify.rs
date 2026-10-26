use std::collections::BTreeSet;

use adtb::btree::{BTree, Index};
use adtb::heap::{FieldType, Heap, LargeStore};
use adtb::key::{CtorId, Dir, Entry, KeySchema, KeyType, Scalar, Tid, Value};
use adtb::mvcc::Transactions;
use adtb::storage::{Options, PageKind, Pager, Pid};
use adtb::verify::{self, Inventory, Problem, Report, Sum};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};

const PAGE: usize = 4096;

fn options() -> Options {
    Options {
        page_size: PAGE as u32,
        log_block: 512,
        segment_size: 256 * 1024,
        pool_frames: 32,
        atomic_page_writes: false,
        ..Options::new("app.adt")
    }
}

struct Db {
    pager: Pager<SimRuntime>,
    txns: Transactions<SimRuntime>,
    heaps: Vec<Heap>,
    index: Index,
    tree: BTree,
}

impl Db {
    fn create(rt: SimRuntime) -> Db {
        let pager = Pager::create(rt.clone(), &options()).expect("create");
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
        let mut db = Db {
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

    fn verify_with(&mut self, pages: &[Pid]) -> Report {
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
        verify::verify(&self.pager, &inventory)
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
    let outcome = sim.run(|rt| body(&mut Db::create(rt)));
    assert_eq!(Outcome::Finished, outcome);
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
        assert_eq!(Ok(1), verify::repair(&db.pager, &report));
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
                p[0][16..20].copy_from_slice(&31u32.to_le_bytes())
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
                p[0][16..20].copy_from_slice(&31u32.to_le_bytes())
            })
            .unwrap();
        let report = db.verify();
        verify::repair(&db.pager, &report).unwrap();
    });
}
