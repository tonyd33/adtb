use std::collections::BTreeSet;

use adtb_sys::os::{Backend, Faults, OsRuntime};
use adtb_testkit::scratch::Scratch;

use super::Quarantine;
use crate::storage::pager::{Options, Pager};
use crate::storage::policy::{self, Fault, Holder, Phase};

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
    let held = policy::mark(Fault::Unarmed, Holder::LsnMap, Phase::Opening);
    pager.hold_quarantine(&opening, held);
    pager
        .plant_mark(Quarantine::Heap(1), &opening)
        .expect("mark the heap");
    drop(opening);
    drop(pager);
    let (pager, _) = Pager::open(rt(), &options).expect("reopen");
    assert_eq!(BTreeSet::from([Quarantine::Heap(1)]), pager.quarantined());
    pager.close();
}
