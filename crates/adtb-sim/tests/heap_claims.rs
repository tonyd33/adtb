use adtb::heap::{Error, FieldType, Header, Heap, LargeStore, WriteTarget};
use adtb::key::{Scalar, Value};
use adtb::runtime::Threads;
use adtb::storage::{Options, Pager};
use adtb_sim::{Config, Outcome, Sim};

fn options() -> Options {
    Options {
        page_size: 4096,
        pool_frames: 32,
        ..Options::new("claims.adt")
    }
}

fn fields() -> Vec<FieldType> {
    vec![FieldType::Scalar(Scalar::I64)]
}

fn rejected_changes(cross_page: bool) {
    let mut sim = Sim::new(0, Config::default());
    let mut saved = None;
    assert_eq!(
        sim.run(|rt| {
            let pager = Pager::create(rt, &options()).unwrap();
            let large = LargeStore::create(&pager, 40).unwrap();
            let heap = Heap::create(&pager, 41, fields(), large).unwrap();
            let other = Heap::create(&pager, 42, fields(), large).unwrap();
            let old = heap.insert(&pager, 1, &[Value::I64(1)]).unwrap();
            if cross_page {
                while heap.pages(&pager).unwrap().len() == 1 {
                    heap.insert(&pager, 1, &[Value::I64(9)]).unwrap();
                }
            }
            let stale = WriteTarget::checked(old, heap.read(&pager, old).unwrap().unwrap().header);
            let next = heap
                .update(&pager, stale, 2, &[Value::I64(2)], &[0])
                .unwrap();
            assert_eq!(next.tid.page() != old.page(), cross_page);
            let header = heap.read(&pager, old).unwrap().unwrap().header;
            assert_eq!(
                header,
                Header {
                    xmin: 1,
                    xmax: 2,
                    next: Some(next.tid)
                }
            );
            let unchanged = heap.headers(&pager).unwrap();
            let expected = Error::Changed {
                current: Some(header),
            };
            assert_eq!(
                heap.update(&pager, stale, 3, &[Value::I64(3)], &[]),
                Err(expected)
            );
            assert_eq!(heap.delete(&pager, stale, 3), Err(expected));
            assert_eq!(
                heap.retag(&pager, stale, 3, &other, &[Value::I64(3)]),
                Err(expected)
            );
            assert_eq!(heap.restore(&pager, stale), Err(expected));
            assert_eq!(heap.headers(&pager).unwrap(), unchanged);
            assert!(other.headers(&pager).unwrap().is_empty());
            assert_eq!(
                heap.read(&pager, next.tid).unwrap().unwrap().row,
                [Value::I64(2)]
            );
            pager.flush_log();
            saved = Some((
                heap.root(),
                other.root(),
                large.root(),
                old,
                next.tid,
                unchanged,
            ));
        }),
        Outcome::Finished
    );
    sim.crash_at(sim.now());
    assert_eq!(sim.run(|rt| rt.yield_now()), Outcome::Crashed);
    let (root, other_root, large_root, old, next, unchanged) = saved.unwrap();
    assert_eq!(
        sim.run(|rt| {
            let (pager, _) = Pager::open(rt, &options()).unwrap();
            let large = LargeStore::open(40, large_root);
            let heap = Heap::open(&pager, 41, root, fields(), large).unwrap();
            let other = Heap::open(&pager, 42, other_root, fields(), large).unwrap();
            assert_eq!(heap.headers(&pager).unwrap(), unchanged);
            assert_eq!(
                heap.read(&pager, old).unwrap().unwrap().header.next,
                Some(next)
            );
            assert_eq!(
                heap.read(&pager, next).unwrap().unwrap().row,
                [Value::I64(2)]
            );
            assert!(other.headers(&pager).unwrap().is_empty());
        }),
        Outcome::Finished
    );
}

#[test]
fn stale_claims_preserve_same_page_successor_after_recovery() {
    rejected_changes(false);
}

#[test]
fn stale_claims_preserve_cross_page_successor_after_recovery() {
    rejected_changes(true);
}

#[test]
fn rollback_cannot_clear_another_writer_header() {
    let mut sim = Sim::new(0, Config::default());
    assert_eq!(
        sim.run(|rt| {
            let pager = Pager::create(rt, &options()).unwrap();
            let large = LargeStore::create(&pager, 40).unwrap();
            let heap = Heap::create(&pager, 41, fields(), large).unwrap();
            let old = heap.insert(&pager, 1, &[Value::I64(1)]).unwrap();
            heap.delete(&pager, old, 2).unwrap();
            let restore =
                WriteTarget::checked(old, heap.read(&pager, old).unwrap().unwrap().header);
            let new = heap.update(&pager, old, 3, &[Value::I64(3)], &[]).unwrap();
            let header = heap.read(&pager, old).unwrap().unwrap().header;
            assert_eq!(
                heap.restore(&pager, restore),
                Err(Error::Changed {
                    current: Some(header)
                })
            );
            assert_eq!(
                heap.read(&pager, old).unwrap().unwrap().header.next,
                Some(new.tid)
            );
        }),
        Outcome::Finished
    );
}
