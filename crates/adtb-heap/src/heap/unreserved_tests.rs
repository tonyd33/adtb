//! A version or chunk whose header names a writer past the commit log's reservation: each reader
//! or writer that looks the writer up refuses the page holding it as damage.

use std::collections::BTreeMap;

use adtb_testkit::process;

use super::{Error as HeapError, FieldType, Heap, LargeStore};
use crate::key::{Scalar, Value};
use crate::mvcc::{StatusError, Transactions, Xid};
use crate::storage::{Damage, Options, PageError, Pager, Pid, Shape, Walk};

const PAST: Xid = Xid(1 << 40);

const FRAME: Damage = Damage::Shape(Shape::Frame);

fn options() -> Options {
    Options {
        page_size: 4096,
        pool_frames: 32,
        ..Options::new("unreserved.adt")
    }
}

fn refused_page(pid: Pid) -> PageError {
    PageError { pid, damage: FRAME }
}

#[test]
fn an_update_pruning_a_full_page_refuses_it_at_an_unreserved_deleter() {
    process::once(5, |rt| {
        let pager = Pager::create(rt.clone(), &options()).unwrap();
        let txns = Transactions::create(rt, &pager).unwrap();
        let large = LargeStore::create(&pager, 40).unwrap();
        let heap = Heap::create(&pager, 41, vec![FieldType::Scalar(Scalar::I64)], large).unwrap();
        let begun = txns.begin(&pager).unwrap();
        let x = begun.xid();
        let dead = heap.insert(&pager, x, &[Value::I64(0)]).unwrap();
        let target = heap.insert(&pager, x, &[Value::I64(1)]).unwrap();
        while heap.pages(&pager).unwrap().len() == 1 {
            heap.insert(&pager, x, &[Value::I64(2)]).unwrap();
        }
        txns.commit(&pager, begun).unwrap().done();
        let page = Pid(dead.page());
        assert_eq!(page, Pid(target.page()), "both versions on the full page");
        heap.delete(&pager, dead, PAST)
            .kept(&crate::heap::Tested)
            .unwrap()
            .unchecked(&crate::heap::Tested);
        let begun = txns.begin(&pager).unwrap();
        let y = begun.xid();
        let mut judge = txns.judge_now(&pager, heap.id().0);
        let got = heap.update_pruning(&pager, target, y, &[Value::I64(3)], &[], &mut judge);
        let got = got.kept(&crate::heap::Tested);
        assert_eq!(Err(HeapError::Page(refused_page(page))), got);
        assert_eq!(BTreeMap::from([(page, FRAME)]), pager.damaged());
        assert_eq!(Err(refused_page(page)), pager.read(page).map(drop));
        pager.close();
    });
}

#[test]
fn a_status_lookup_of_a_version_whose_header_changed_before_the_latch_reads_again() {
    process::once(7, |rt| {
        let pager = Pager::create(rt, &options()).unwrap();
        let large = LargeStore::create(&pager, 40).unwrap();
        let heap = Heap::create(&pager, 41, vec![FieldType::Scalar(Scalar::I64)], large).unwrap();
        let tid = heap.insert(&pager, Xid(1), &[Value::I64(1)]).unwrap();
        heap.insert(&pager, Xid(1), &[Value::I64(2)]).unwrap();
        let mut versions = Vec::new();
        heap.scan(&pager, |v| versions.push(v)).unwrap();
        let [changed, kept] = versions.as_slice() else {
            panic!("two versions: {versions:?}");
        };
        heap.delete(&pager, tid, PAST)
            .kept(&crate::heap::Tested)
            .unwrap()
            .unchecked(&crate::heap::Tested);
        let unreserved = || Err::<(), _>(StatusError::Unreserved(PAST));
        assert_eq!(Ok(None), changed.looked_up(&pager, unreserved()));
        assert_eq!(BTreeMap::new(), pager.damaged());
        let page = Pid(tid.page());
        assert_eq!(
            Err(refused_page(page)),
            kept.looked_up(&pager, unreserved())
        );
        assert_eq!(BTreeMap::from([(page, FRAME)]), pager.damaged());
        pager.close();
    });
}

#[test]
fn a_sweep_meeting_a_chunk_of_an_unreserved_writer_refuses_its_page() {
    process::once(6, |rt| {
        let pager = Pager::create(rt.clone(), &options()).unwrap();
        let txns = Transactions::create(rt, &pager).unwrap();
        let large = LargeStore::create(&pager, 40).unwrap();
        let fields = vec![
            FieldType::Scalar(Scalar::I64),
            FieldType::Scalar(Scalar::Bytes),
        ];
        let heap = Heap::create(&pager, 41, fields, large).unwrap();
        let row = [Value::I64(1), Value::Bytes(vec![7; 20_000])];
        heap.insert(&pager, PAST, &row).unwrap();
        let head = heap.out_of_line(&pager).unwrap().unwrap()[0].first;
        large.recount(&pager, head, 0).unwrap();
        let page = Pid(head.page());
        let resolved = |x| txns.resolved(&pager, x);
        let got = large.sweep(
            &crate::vacuum::Vacuuming::unheld(&pager, &txns),
            resolved,
            &mut || true,
        );
        assert_eq!(Err::<Walk, _>(refused_page(page)), got);
        assert_eq!(BTreeMap::from([(page, FRAME)]), pager.damaged());
        pager.close();
    });
}
