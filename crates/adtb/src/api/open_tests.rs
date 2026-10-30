use adtb_sys::os::{Backend, OsRuntime};
use adtb_testkit::scratch::Scratch;

use crate::catalog::Catalog;
use crate::storage::{Damage, Options, Pager, Quarantine};

use super::{mark_open_damage, open_damage};

fn unreadable_root(scratch: &Scratch) -> (OsRuntime, Pager<OsRuntime>) {
    let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
    let pager = Pager::create(rt.clone(), &Options::new("unreadable.adt")).unwrap();
    drop(Catalog::open(rt.clone(), &pager).unwrap());
    pager.damage(pager.catalog_root(), Damage::Unreadable);
    (rt, pager)
}

#[test]
fn an_unreadable_catalog_root_marks_nothing_and_opens_nothing() {
    let scratch = Scratch::new("open-damage", "unreadable-unmarked");
    let (rt, pager) = unreadable_root(&scratch);
    mark_open_damage(rt, &pager);
    assert!(!pager.is_quarantined(Quarantine::Database));
    assert!(open_damage(&pager).is_none());
    pager.close();
}

#[test]
fn an_unreadable_page_on_a_marked_file_opens_nothing() {
    let scratch = Scratch::new("open-damage", "unreadable-marked");
    let (_rt, pager) = unreadable_root(&scratch);
    pager.quarantine(Quarantine::Database).unwrap();
    assert!(open_damage(&pager).is_none());
    pager.close();
}
