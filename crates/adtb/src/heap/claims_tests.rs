use adtb_sim::{Config, Outcome, Sim, SimRuntime};

use super::{Error, FieldType, Heap, LargeStore, WriteTarget};
use crate::key::{Scalar, Tid, Value};
use crate::mvcc::Xid;
use crate::runtime::Threads;
use crate::storage::{Damage, FIRST_EDITABLE, Options, PageError, Pager, Pid, Shape};

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
            let old = heap.insert(&pager, Xid(1), &[Value::I64(1)]).unwrap();
            if cross_page {
                while heap.pages(&pager).unwrap().len() == 1 {
                    heap.insert(&pager, Xid(1), &[Value::I64(9)]).unwrap();
                }
            }
            let stale = WriteTarget::checked(old, heap.read(&pager, old).unwrap().unwrap().header);
            let next = heap
                .update(&pager, stale, Xid(2), &[Value::I64(2)], &[0])
                .unwrap();
            assert_eq!(next.tid.page() != old.page(), cross_page);
            let header = heap.read(&pager, old).unwrap().unwrap().header;
            assert_eq!(header, super::tests::header(1, 2, Some(next.tid)));
            let unchanged = heap.headers(&pager).unwrap();
            let expected = Error::Changed {
                current: Some(header),
            };
            assert_eq!(
                heap.update(&pager, stale, Xid(3), &[Value::I64(3)], &[]),
                Err(expected)
            );
            assert_eq!(heap.delete(&pager, stale, Xid(3)), Err(expected));
            assert_eq!(
                heap.retag(&pager, stale, Xid(3), &other, &[Value::I64(3)]),
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
            let old = heap.insert(&pager, Xid(1), &[Value::I64(1)]).unwrap();
            heap.delete(&pager, old, Xid(2)).unwrap();
            let restore =
                WriteTarget::checked(old, heap.read(&pager, old).unwrap().unwrap().header);
            let new = heap
                .update(&pager, old, Xid(3), &[Value::I64(3)], &[])
                .unwrap();
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

/// Overwrite page `pid` with page `from` past the bytes the pager keeps, owner and kind included.
fn plant(pager: &Pager<SimRuntime>, pid: Pid, from: Pid) {
    let bytes = pager.read(from).unwrap().to_vec();
    pager
        .update(&[pid], |p| {
            p[0][FIRST_EDITABLE..].copy_from_slice(&bytes[FIRST_EDITABLE..]);
        })
        .unwrap();
}

const FORMAT_AT: usize = FIRST_EDITABLE;

fn in_sim(f: impl FnOnce(SimRuntime)) {
    let mut sim = Sim::new(0, Config::default());
    assert_eq!(sim.run(f), Outcome::Finished);
}

type Write = fn(&Heap, &Pager<SimRuntime>, WriteTarget, &Heap) -> Result<(), Error>;

/// Plant a page of another heap holding a version with the same header over the page of a
/// version of a heap, then run `write` on a target checked against that version. Returns its
/// answer and whether the planted page kept its bytes.
fn foreign_target(write: Write) -> (Result<(), Error>, bool) {
    let mut out = None;
    in_sim(|rt| {
        let pager = Pager::create(rt, &options()).unwrap();
        let large = LargeStore::create(&pager, 40).unwrap();
        let heap = Heap::create(&pager, 41, fields(), large).unwrap();
        let other = Heap::create(&pager, 42, fields(), large).unwrap();
        let old = heap.insert(&pager, Xid(1), &[Value::I64(1)]).unwrap();
        let theirs = other.insert(&pager, Xid(1), &[Value::I64(2)]).unwrap();
        assert_eq!(
            old.slot(),
            theirs.slot(),
            "the planted version takes the target's slot"
        );
        let header = heap.read(&pager, old).unwrap().unwrap().header;
        assert_eq!(header, other.read(&pager, theirs).unwrap().unwrap().header);
        let pid = Pid(old.page());
        plant(&pager, pid, Pid(theirs.page()));
        let planted = pager.read(pid).unwrap().to_vec();
        let answer = write(&heap, &pager, WriteTarget::checked(old, header), &other);
        out = Some((answer, planted == pager.read(pid).unwrap().to_vec()));
    });
    out.unwrap()
}

#[test]
fn a_checked_delete_or_restore_of_a_version_whose_page_another_heap_now_owns_is_changed() {
    let writes: [(&str, Write); 2] = [
        ("delete", |h, p, t, _| h.delete(p, t, Xid(3))),
        ("restore", |h, p, t, _| h.restore(p, t)),
    ];
    for (what, write) in writes {
        let changed = Err(Error::Changed { current: None });
        assert_eq!((changed, true), foreign_target(write), "{what}");
    }
}

#[test]
fn a_checked_update_or_retag_of_a_version_whose_page_another_heap_now_owns_is_changed() {
    let writes: [(&str, Write); 2] = [
        ("update", |h, p, t, _| {
            h.update(p, t, Xid(3), &[Value::I64(3)], &[0]).map(drop)
        }),
        ("retag", |h, p, t, to| {
            h.retag(p, t, Xid(3), to, &[Value::I64(3)]).map(drop)
        }),
    ];
    for (what, write) in writes {
        let changed = Err(Error::Changed { current: None });
        assert_eq!((changed, true), foreign_target(write), "{what}");
    }
}

#[test]
fn headers_and_out_of_line_values_skip_a_page_another_heap_owns_that_the_directory_names() {
    in_sim(|rt| {
        let pager = Pager::create(rt, &options()).unwrap();
        let large = LargeStore::create(&pager, 40).unwrap();
        let heap = Heap::create(&pager, 41, fields(), large).unwrap();
        let other = Heap::create(&pager, 42, fields(), large).unwrap();
        let old = heap.insert(&pager, Xid(1), &[Value::I64(1)]).unwrap();
        let theirs = other.insert(&pager, Xid(1), &[Value::I64(2)]).unwrap();
        let pid = Pid(old.page());
        plant(&pager, pid, Pid(theirs.page()));
        assert_eq!(vec![pid], heap.pages(&pager).unwrap());
        assert_eq!(Ok(Vec::new()), heap.headers(&pager));
        assert_eq!(Ok(None), heap.out_of_line(&pager));
    });
}

#[test]
fn a_page_whose_format_byte_names_no_format_of_the_heap_is_refused_as_damage() {
    type Read = fn(&Heap, &Pager<SimRuntime>, Tid) -> Result<(), Error>;
    let reads: [(&str, Read); 3] = [
        ("read", |h, p, t| {
            h.read(p, t).map(drop).map_err(Error::from)
        }),
        ("out_of_line", |h, p, _| {
            h.out_of_line(p).map(drop).map_err(Error::from)
        }),
        ("update", |h, p, t| {
            h.update(p, t, Xid(3), &[Value::I64(3)], &[0]).map(drop)
        }),
    ];
    for byte in [0u16, 2] {
        for (what, read) in reads {
            in_sim(|rt| {
                let pager = Pager::create(rt, &options()).unwrap();
                let large = LargeStore::create(&pager, 40).unwrap();
                let heap = Heap::create(&pager, 41, fields(), large).unwrap();
                let old = heap.insert(&pager, Xid(1), &[Value::I64(1)]).unwrap();
                let pid = Pid(old.page());
                pager
                    .update(&[pid], |p| {
                        p[0][FORMAT_AT..FORMAT_AT + 2].copy_from_slice(&byte.to_le_bytes());
                    })
                    .unwrap();
                let damage = Damage::Shape(Shape::Format);
                let refused = Error::Page(PageError { pid, damage });
                assert_eq!(
                    Err(refused),
                    read(&heap, &pager, old),
                    "{what} of format {byte}"
                );
                assert_eq!(
                    Some(&damage),
                    pager.damaged().get(&pid),
                    "{what} of format {byte}"
                );
            });
        }
    }
}

#[test]
fn a_store_page_found_refused_leaves_a_page_no_longer_of_the_store_alone() {
    in_sim(|rt| {
        let pager = Pager::create(rt, &options()).unwrap();
        let large = LargeStore::create(&pager, 40).unwrap();
        let other = Heap::create(&pager, 42, fields(), large).unwrap();
        large.write(&pager, Xid(1), &[1; 100]).unwrap();
        let pages = large.pages(&pager).unwrap();
        assert_eq!(1, pages.len(), "one store page");
        plant(&pager, pages[0], other.root());
        let planted = pager.read(pages[0]).unwrap().to_vec();
        assert_eq!(Ok(()), large.refused(&pager, pages[0]));
        assert_eq!(planted, pager.read(pages[0]).unwrap().to_vec());
        assert!(pager.damaged().is_empty(), "{:?}", pager.damaged());
    });
}
