use adtb_lang::types::{Base, Ty};

use super::super::Error;
use super::{Broken, Broken as B};

/// Returns the case that follows `b`, the first when `b` is `None`, and the text it reads as.
/// A walk from `None` reaches every variant.
#[rustfmt::skip]
fn after(b: Option<&Broken>) -> Option<(Broken, &'static str)> {
    Some(match b {
        None => (B::DerefNotKey, "a dereference follows a key"),
        Some(B::DerefNotKey) => (B::KnownArmMissing, "a case names the constructor its scrutinee holds"),
        Some(B::KnownArmMissing) => (B::FunctionValue, "a normal form holds no function value"),
        Some(B::FunctionValue) => (B::AggregateArity, "an aggregate's step takes two values and its finish one"),
        Some(B::AggregateArity) => (B::RangeNotSet(Ty::Base(Base::I64)), "a comprehension ranges over a set, not Base(I64)"),
        Some(B::RangeNotSet(_)) => (B::GroupNotRecord, "a group is a record of its key and rows"),
        Some(B::GroupNotRecord) => (B::GroupFieldMissing, "a group has a key and rows"),
        Some(B::GroupFieldMissing) => return None,
    })
}

#[test]
fn each_broken_guarantee_reads_as_its_text() {
    let mut seen = Vec::new();
    for (broken, text) in std::iter::successors(after(None), |(b, _)| after(Some(b))) {
        assert!(!seen.contains(&broken), "the walk returns to {broken:?}");
        assert_eq!(text, broken.to_string(), "{broken:?}");
        seen.push(broken);
    }
}

#[test]
fn an_internal_plan_error_reads_as_a_broken_invariant() {
    let text = "a broken invariant: a group has a key and rows";
    assert_eq!(text, Error::from(Broken::GroupFieldMissing).to_string());
}
