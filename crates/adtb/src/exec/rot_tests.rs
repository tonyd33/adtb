use crate::btree;
use crate::catalog::Step;
use crate::storage::Pid;

use super::super::Error;
use super::{Rot, Rot as R, detail};

/// Returns the case that follows `r`, the first when `r` is `None`, and the text it reads as.
/// A walk from `None` reaches every variant.
#[rustfmt::skip]
fn after(r: Option<&Rot>) -> Option<(Rot, &'static str)> {
    let bad = || btree::Error::Malformed(Pid(4));
    Some(match r {
        None => (R::SpillShort, "a spilled extent reads back short"),
        Some(R::SpillShort) => (R::SpillCut, "a spilled record is cut short"),
        Some(R::SpillCut) => (R::SpillUndecoded, "a spilled value does not decode"),
        Some(R::SpillUndecoded) => (R::SpillNotPair, "a spilled build record is not a pair"),
        Some(R::SpillNotPair) => (R::DeferredNoVariable, "a deferred row names no variable"),
        Some(R::DeferredNoVariable) => (R::Index(bad()), "Malformed(Pid(4))"),
        Some(R::Index(_)) => (R::StoredPath(vec![Step::Field(0), Step::Ctor(1)]), "a stored row does not hold path [Field(0), Ctor(1)]"),
        Some(R::StoredPath(_)) => (R::RetentionWindow(bad()), "the record of the retention window is damaged: Malformed(Pid(4))"),
        Some(R::RetentionWindow(_)) => (R::Markers(bad()), "Malformed(Pid(4))"),
        Some(R::Markers(_)) => return None,
    })
}

#[test]
fn each_damage_reads_as_its_text() {
    let mut seen = Vec::new();
    for (rot, text) in std::iter::successors(after(None), |(r, _)| after(Some(r))) {
        assert!(!seen.contains(&rot), "the walk returns to {rot:?}");
        assert_eq!(text, rot.to_string(), "{rot:?}");
        seen.push(rot);
    }
}

#[test]
fn detail_reads_a_damage_cause_as_the_debug_of_its_text() {
    let text = "Corrupt(\"a spilled record is cut short\")";
    assert_eq!(text, detail(&Error::from(Rot::SpillCut)));
    assert_eq!("Cancelled", detail(&Error::Cancelled));
}
