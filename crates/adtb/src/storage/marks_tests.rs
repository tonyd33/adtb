use std::collections::BTreeSet;

use adtb_sys::os::{Backend, Faults, OsRuntime};
use adtb_testkit::scratch::Scratch;

use super::{Marks, Quarantine, Unwritable};
use crate::storage::Pid;
use crate::storage::pager::{Options, Pager};

const PAGE: Pid = Pid(5);

#[test]
fn the_any_flag_follows_the_marks_after_every_change() {
    let scratch = Scratch::new("storage", "marks-any");
    let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
    let marks = Marks::new(PAGE);
    marks.load(&rt, PAGE, Ok(BTreeSet::new()), true);
    assert!(!marks.any());
    let heap = Quarantine::Heap(1);
    let index = Quarantine::Index(2);
    let set = marks.change_on_page(&rt, |s| {
        s.insert(heap);
    });
    assert_eq!(BTreeSet::from([heap]), set);
    assert!(marks.any());
    marks.change_on_page(&rt, |s| {
        s.insert(index);
    });
    marks.change_on_page(&rt, |s| {
        s.remove(&heap);
    });
    assert!(marks.any());
    let set = marks.change_on_page(&rt, |s| {
        s.remove(&index);
    });
    assert_eq!(BTreeSet::new(), set);
    assert!(!marks.any());
    marks.hold(&rt, Quarantine::Database);
    assert!(marks.any());
    marks.change(&rt, |s| {
        s.insert(heap);
    });
    marks.change(&rt, |s| {
        s.remove(&heap);
    });
    assert!(marks.any());
}

#[test]
fn marks_set_for_a_page_that_does_not_decode_take_no_write() {
    let scratch = Scratch::new("storage", "marks-unreadable");
    let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
    let marks = Marks::new(PAGE);
    assert!(!marks.any());
    marks.load(&rt, PAGE, Err(BTreeSet::from([Quarantine::Database])), true);
    assert!(marks.any());
    assert_eq!(BTreeSet::from([Quarantine::Database]), marks.all(&rt));
    assert_eq!(Err(Unwritable::Unreadable(PAGE)), marks.writable_page(&rt));
}

#[test]
fn a_mark_set_while_the_open_holds_the_database_survives_a_crash() {
    let scratch = Scratch::new("storage", "marks-under-held");
    let rt = || {
        OsRuntime::with_faults(
            scratch.path(),
            Backend::Pool { threads: 1 },
            Faults::none(4096),
        )
    };
    let options = Options {
        pool_frames: 16,
        max_pages: 1024,
        ..Options::new("marks-under-held.adt")
    };
    let pager = Pager::create(rt(), &options).expect("create pager");
    let opening = crate::storage::Opening::begin(&pager);
    pager.hold_quarantine(&opening, Quarantine::Database);
    pager
        .plant_mark(Quarantine::Heap(1), &opening)
        .expect("mark the heap");
    drop(opening);
    drop(pager);
    let (pager, _) = Pager::open(rt(), &options).expect("reopen");
    assert_eq!(BTreeSet::from([Quarantine::Heap(1)]), pager.quarantined());
    pager.close();
}
