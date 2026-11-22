use std::fmt;

use adtb_lang::kernel::{Invalid, PathStep};
use adtb_lang::textual::PrintError;
use adtb_lang::types::Ty;

use super::layout::Step;
use crate::batch;
use crate::heap::{FieldType, HeapNo};
use crate::key::{EncodeError, KeyType, Scalar, Value};
use crate::plan::{HeapId, IndexId};

use super::{Error, Val};

/// An invariant of the plan or of a value that the checker guarantees and that does not hold.
#[derive(Clone, PartialEq, Eq)]
pub enum Broken {
    Mistyped {
        want: &'static str,
        found: Val,
    },
    Operand {
        want: &'static str,
        found: Val,
    },
    Operands {
        want: &'static str,
        found: (Val, Val),
    },
    Ranged {
        want: &'static str,
        found: Val,
    },
    EmptyRange,
    NotStored {
        value: Val,
        ty: FieldType,
    },
    NotKeyed {
        value: Val,
        ty: KeyType,
    },
    NotConverted(Value),
    /// A key the engine builds does not encode under its index's key schema.
    Unencodable(EncodeError),
    /// A stored row of the constructor of this id has no key.
    UnkeyedRow(u32),
    /// A row whose value in one column passes 2^31 elements or value bytes alone, which no row
    /// within the value limit of 2^31 - 1 bytes holds.
    PastOffsets,
    NotScalar {
        value: Val,
        scalar: Scalar,
    },
    OutsideDomain {
        value: Val,
        scalar: Scalar,
    },
    NotBatchTyped {
        value: Val,
        ty: batch::Type,
    },
    NotBatchable(Val),
    FieldCount {
        values: Vec<Val>,
        types: Vec<batch::Type>,
    },
    KeyComponents {
        key: usize,
        types: usize,
    },
    Print(PrintError),
    IllTyped(Invalid),
    Unreachable,
    IntegerResult,
    NestedPast,
    OutsideScope(HeapNo),
    UnlistedHeap(HeapId),
    UnlistedIndex(IndexId),
    UnboundHeap(HeapId),
    UnboundIndex(IndexId),
    UnboundParam(String),
    NotPosition(Val),
    NotInstant(Val),
    RecordLacksField {
        ty: Ty,
        name: String,
    },
    NoCtorHasField {
        ty: Ty,
        name: String,
    },
    PathStart(Vec<PathStep>),
    PathRowField {
        path: Vec<PathStep>,
        field: String,
    },
    PathCtor {
        path: Vec<PathStep>,
        ctor: u32,
    },
    PathValueField {
        path: Vec<PathStep>,
        field: String,
    },
    StepsStart(Vec<Step>),
    NoNormal {
        index: usize,
        element: usize,
    },
    OtherVersion(String),
}

impl From<Broken> for Error {
    fn from(b: Broken) -> Error {
        Error::Internal(Box::new(b))
    }
}

impl From<EncodeError> for Error {
    fn from(e: EncodeError) -> Error {
        Broken::Unencodable(e).into()
    }
}

impl fmt::Debug for Broken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.to_string())
    }
}

impl fmt::Display for Broken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Broken::Mistyped { want, found } => write!(f, "expected {want}, found {found:?}"),
            Broken::Operand { want, found } => {
                write!(f, "a primitive's operands are {want}, found {found:?}")
            }
            Broken::Operands { want, found } => {
                write!(f, "a primitive's operands are {want}, found {found:?}")
            }
            Broken::Ranged { want, found } => {
                write!(f, "an aggregate's range binds {want}, found {found:?}")
            }
            Broken::EmptyRange => {
                f.write_str("an aggregate that is not an Option ranges over a nonempty range")
            }
            Broken::NotStored { value, ty } => write!(f, "{value:?} is not a value of {ty:?}"),
            Broken::NotKeyed { value, ty } => write!(f, "{value:?} is not a value of {ty:?}"),
            Broken::NotConverted(value) => {
                write!(f, "{value:?} is not a value of its field's type")
            }
            Broken::Unencodable(e) => match e {
                EncodeError::Count { expected, found } => write!(
                    f,
                    "a key has {found} values where its index's key schema has {expected}"
                ),
                EncodeError::Type => {
                    f.write_str("a key holds a value not of its type in its index's key schema")
                }
            },
            Broken::UnkeyedRow(c) => {
                write!(f, "a stored row of constructor {c} has no key")
            }
            Broken::PastOffsets => {
                f.write_str("a row passes 2^31 elements or value bytes in one column alone")
            }
            Broken::NotScalar { value, scalar } => {
                write!(f, "{value:?} is not a value of {scalar:?}")
            }
            Broken::OutsideDomain { value, scalar } => {
                write!(f, "{value:?} lies outside the domain of {scalar:?}")
            }
            Broken::NotBatchTyped { value, ty } => {
                write!(f, "expected a value of batch type {ty:?}, found {value:?}")
            }
            Broken::NotBatchable(v) => {
                write!(f, "a result in batches is a set or a list, found {v:?}")
            }
            Broken::FieldCount { values, types } => write!(
                f,
                "{values:?} has {} fields, its batch type {types:?} has {}",
                values.len(),
                types.len()
            ),
            Broken::KeyComponents { key, types } => {
                write!(
                    f,
                    "an index key has one type per component: {key} components, {types} types"
                )
            }
            Broken::Print(e) => f.write_str(&e.message),
            Broken::IllTyped(e) => write!(
                f,
                "kernel term {:?} satisfies its kind's typing rule: {}",
                e.at(),
                e.reason()
            ),
            Broken::Unreachable => f.write_str("an evaluation reaches no `Unreachable`"),
            Broken::IntegerResult => f.write_str("integer arithmetic has an integer result"),
            Broken::NestedPast => f.write_str("a read of the past holds no other read of the past"),
            Broken::OutsideScope(heap) => write!(
                f,
                "a store reads only the heaps its reader's snapshot holds back, not heap {}",
                heap.0
            ),
            Broken::UnlistedHeap(heap) => write!(
                f,
                "a store reads only the heaps its reads list, not heap {}",
                heap.0
            ),
            Broken::UnlistedIndex(index) => write!(
                f,
                "a store reads only the indexes its reads list, not index {}",
                index.0
            ),
            Broken::UnboundHeap(heap) => {
                write!(
                    f,
                    "every heap a store's reads list is bound, not heap {}",
                    heap.0
                )
            }
            Broken::UnboundIndex(index) => write!(
                f,
                "every index a store's reads list is bound, not index {}",
                index.0
            ),
            Broken::UnboundParam(name) => {
                write!(
                    f,
                    "a caller binds every parameter of its program, not `{name}`"
                )
            }
            Broken::NotPosition(v) => {
                write!(f, "a position is an epoch and an LSN, found {v:?}")
            }
            Broken::NotInstant(v) => write!(f, "an `at instant` names an instant, found {v:?}"),
            Broken::RecordLacksField { ty, name } => {
                write!(f, "a record of type {ty:?} has field `{name}`")
            }
            Broken::NoCtorHasField { ty, name } => {
                write!(f, "some constructor of {ty:?} has field `{name}`")
            }
            Broken::PathStart(path) => write!(f, "an index path starts at a field: {path:?}"),
            Broken::PathRowField { path, field } => write!(
                f,
                "an index path names a field `{field}` of the row: {path:?}"
            ),
            Broken::PathCtor { path, ctor } => write!(
                f,
                "an index path names a constructor {ctor} of the value: {path:?}"
            ),
            Broken::PathValueField { path, field } => write!(
                f,
                "an index path reaches a value with field `{field}`: {path:?}"
            ),
            Broken::StepsStart(steps) => {
                write!(f, "index path steps start at a field: {steps:?}")
            }
            Broken::NoNormal { index, element } => write!(
                f,
                "derived element {element} of index {index} has a normal program"
            ),
            Broken::OtherVersion(name) => write!(
                f,
                "host function `{name}` is bound at the version its declaration names"
            ),
        }
    }
}
