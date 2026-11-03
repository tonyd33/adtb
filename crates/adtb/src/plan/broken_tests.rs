use adtb_lang::types::{Base, Ty};

use super::super::Error;
use super::{Broken, Broken as B};

/// Returns the text the case of `b`'s variant in `cases` reads as.
fn expected(b: &Broken) -> &'static str {
    match b {
        B::DerefNotKey => "a dereference follows a key",
        B::KnownArmMissing => "a case names the constructor its scrutinee holds",
        B::FunctionValue => "a normal form holds no function value",
        B::AggregateArity => "an aggregate's step takes two values and its finish one",
        B::RangeNotSet(_) => "a comprehension ranges over a set, not Base(I64)",
        B::GroupNotRecord => "a group is a record of its key and rows",
        B::GroupFieldMissing => "a group has a key and rows",
    }
}

#[test]
fn each_broken_guarantee_reads_as_its_text() {
    let cases = [
        B::DerefNotKey,
        B::KnownArmMissing,
        B::FunctionValue,
        B::AggregateArity,
        B::RangeNotSet(Ty::Base(Base::I64)),
        B::GroupNotRecord,
        B::GroupFieldMissing,
    ];
    for broken in cases {
        assert_eq!(expected(&broken), broken.to_string(), "{broken:?}");
    }
}

#[test]
fn an_internal_plan_error_reads_as_a_broken_invariant() {
    let text = "a broken invariant: a group has a key and rows";
    assert_eq!(text, Error::from(Broken::GroupFieldMissing).to_string());
}
