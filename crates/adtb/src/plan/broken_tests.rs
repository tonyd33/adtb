use adtb_lang::kernel::Var;
use adtb_lang::types::{Base, Ty};

use super::super::{Error, HeapId, IndexId};
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
        Some(B::GroupFieldMissing) => (B::Unbound(Var(3)), "variable 3 is bound where it is read"),
        Some(B::Unbound(_)) => (B::CaseNotSum(Ty::Base(Base::I64)), "a case's scrutinee is of a sum, not Base(I64)"),
        Some(B::CaseNotSum(_)) => (B::NoArm(2), "a case has an arm for constructor 2"),
        Some(B::NoArm(_)) => (B::ArmTwice(2), "a case has one arm for constructor 2"),
        Some(B::ArmTwice(_)) => (B::ArmNotCtor(4), "a case's arm names a constructor of its scrutinee, not 4"),
        Some(B::ArmNotCtor(_)) => (B::ScanHeap(HeapId(5)), "a scan reads the heap of its constructor, not heap 5"),
        Some(B::ScanHeap(_)) => (B::UnknownIndex(IndexId(6)), "a plan names indexes of the catalog, not 6"),
        Some(B::UnknownIndex(_)) => (B::LookupNotKey(IndexId(6)), "a lookup probes its relation's key index, not index 6"),
        Some(B::LookupNotKey(_)) => (B::RouteNotOfIndex { index: IndexId(6), ctor: 1 }, "a route names a constructor of index 6, not 1"),
        Some(B::RouteNotOfIndex { .. }) => (B::RouteTwice { index: IndexId(6), ctor: 1 }, "a probe of index 6 has one route for constructor 1"),
        Some(B::RouteTwice { .. }) => (B::RouteHeap { ctor: 1, heap: HeapId(5) }, "a route of constructor 1 reads its constructor's heap, not heap 5"),
        Some(B::RouteHeap { .. }) => (B::RangeNotOrdered(IndexId(6)), "a range probes an ordered index, not 6"),
        Some(B::RangeNotOrdered(_)) => (B::RangeNotField(IndexId(6)), "a range of index 6 bounds a field after its key prefix"),
        Some(B::RangeNotField(_)) => (B::UnlistedHeap(HeapId(5)), "a plan's reads list heap 5"),
        Some(B::UnlistedHeap(_)) => (B::UnlistedIndex(IndexId(6)), "a plan's reads list index 6"),
        Some(B::UnlistedIndex(_)) => (B::PastUnlistedHeap(HeapId(5)), "a read of the past lists heap 5"),
        Some(B::PastUnlistedHeap(_)) => (B::PastUnlistedIndex(IndexId(6)), "a read of the past lists index 6"),
        Some(B::PastUnlistedIndex(_)) => (B::PastIndex(IndexId(6)), "a read of the past probes no index but by key lookups, not index 6"),
        Some(B::PastIndex(_)) => (B::NestedPast, "a read of the past holds no other read of the past"),
        Some(B::NestedPast) => return None,
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
