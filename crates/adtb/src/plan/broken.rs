use std::fmt;

use adtb_lang::kernel::Var;
use adtb_lang::types::Ty;

use super::{Error, HeapId, IndexId};

/// A guarantee of the checker or the normaliser that a term given to the planner breaks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Broken {
    DerefNotKey,
    KnownArmMissing,
    FunctionValue,
    AggregateArity,
    RangeNotSet(Ty),
    GroupNotRecord,
    GroupFieldMissing,
    Unbound(Var),
    CaseNotSum(Ty),
    NoArm(u32),
    ArmTwice(u32),
    ArmNotCtor(u32),
    ScanHeap(HeapId),
    UnknownIndex(IndexId),
    LookupNotKey(IndexId),
    RouteNotOfIndex { index: IndexId, ctor: u32 },
    RouteTwice { index: IndexId, ctor: u32 },
    RouteHeap { ctor: u32, heap: HeapId },
    RangeNotOrdered(IndexId),
    RangeNotField(IndexId),
    UnlistedHeap(HeapId),
    UnlistedIndex(IndexId),
    PastUnlistedHeap(HeapId),
    PastUnlistedIndex(IndexId),
    PastIndex(IndexId),
    NestedPast,
}

impl From<Broken> for Error {
    fn from(b: Broken) -> Error {
        Error::Internal(b)
    }
}

impl fmt::Display for Broken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Broken::DerefNotKey => f.write_str("a dereference follows a key"),
            Broken::KnownArmMissing => {
                f.write_str("a case names the constructor its scrutinee holds")
            }
            Broken::FunctionValue => f.write_str("a normal form holds no function value"),
            Broken::AggregateArity => {
                f.write_str("an aggregate's step takes two values and its finish one")
            }
            Broken::RangeNotSet(t) => write!(f, "a comprehension ranges over a set, not {t:?}"),
            Broken::GroupNotRecord => f.write_str("a group is a record of its key and rows"),
            Broken::GroupFieldMissing => f.write_str("a group has a key and rows"),
            Broken::Unbound(v) => write!(f, "variable {} is bound where it is read", v.0),
            Broken::CaseNotSum(t) => write!(f, "a case's scrutinee is of a sum, not {t:?}"),
            Broken::NoArm(c) => write!(f, "a case has an arm for constructor {c}"),
            Broken::ArmTwice(c) => write!(f, "a case has one arm for constructor {c}"),
            Broken::ArmNotCtor(c) => {
                write!(
                    f,
                    "a case's arm names a constructor of its scrutinee, not {c}"
                )
            }
            Broken::ScanHeap(h) => {
                write!(
                    f,
                    "a scan reads the heap of its constructor, not heap {}",
                    h.0
                )
            }
            Broken::UnknownIndex(i) => {
                write!(f, "a plan names indexes of the catalog, not {}", i.0)
            }
            Broken::LookupNotKey(i) => {
                write!(
                    f,
                    "a lookup probes its relation's key index, not index {}",
                    i.0
                )
            }
            Broken::RouteNotOfIndex { index, ctor } => write!(
                f,
                "a route names a constructor of index {}, not {ctor}",
                index.0
            ),
            Broken::RouteTwice { index, ctor } => write!(
                f,
                "a probe of index {} has one route for constructor {ctor}",
                index.0
            ),
            Broken::RouteHeap { ctor, heap } => write!(
                f,
                "a route of constructor {ctor} reads its constructor's heap, not heap {}",
                heap.0
            ),
            Broken::RangeNotOrdered(i) => write!(f, "a range probes an ordered index, not {}", i.0),
            Broken::RangeNotField(i) => write!(
                f,
                "a range of index {} bounds a field after its key prefix",
                i.0
            ),
            Broken::UnlistedHeap(h) => write!(f, "a plan's reads list heap {}", h.0),
            Broken::UnlistedIndex(i) => write!(f, "a plan's reads list index {}", i.0),
            Broken::PastUnlistedHeap(h) => {
                write!(f, "a read of the past lists heap {}", h.0)
            }
            Broken::PastUnlistedIndex(i) => {
                write!(f, "a read of the past lists index {}", i.0)
            }
            Broken::PastIndex(i) => write!(
                f,
                "a read of the past probes no index but by key lookups, not index {}",
                i.0
            ),
            Broken::NestedPast => f.write_str("a read of the past holds no other read of the past"),
        }
    }
}

#[cfg(test)]
#[path = "broken_tests.rs"]
mod tests;
