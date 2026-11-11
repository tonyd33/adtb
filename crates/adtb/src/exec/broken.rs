use std::fmt;

use adtb_lang::kernel::PathStep;
use adtb_lang::textual::PrintError;
use adtb_lang::types::Ty;

use crate::batch;
use crate::catalog::Step;
use crate::heap::{FieldType, HeapNo};
use crate::key::{KeyType, Scalar};

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
    NotScalar {
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
    Print(PrintError),
    NoArm(u32),
    RejectNotText,
    Unreachable,
    DecimalLiteral,
    IntegerLiteral,
    CtorNotSum,
    CtorField(String),
    KeyNotKeyTyped(Ty),
    RestrictNotSet,
    CheckedNotOption,
    IntegerResult,
    ConversionNotOption,
    ParseNotOption,
    NestedPast,
    OutsideScope(HeapNo),
    NotPosition(Val),
    NotInstant(Val),
    CtorLacksField(u32),
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
            Broken::NotScalar { value, scalar } => {
                write!(f, "{value:?} is not a value of {scalar:?}")
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
            Broken::Print(e) => f.write_str(&e.message),
            Broken::NoArm(c) => write!(f, "a case has an arm for constructor {c}"),
            Broken::RejectNotText => f.write_str("`reject` takes a Text"),
            Broken::Unreachable => f.write_str("an evaluation reaches no `Unreachable`"),
            Broken::DecimalLiteral => f.write_str("a decimal literal fits its type"),
            Broken::IntegerLiteral => f.write_str("an integer literal fits its type"),
            Broken::CtorNotSum => f.write_str("a constructor's type is a sum"),
            Broken::CtorField(n) => write!(f, "constructor field `{n}` is given"),
            Broken::KeyNotKeyTyped(ty) => write!(f, "`key` has a key type, found {ty:?}"),
            Broken::RestrictNotSet => f.write_str("a restriction is of a set"),
            Broken::CheckedNotOption => f.write_str("a checked primitive returns an Option"),
            Broken::IntegerResult => f.write_str("integer arithmetic has an integer result"),
            Broken::ConversionNotOption => f.write_str("a checked conversion returns an Option"),
            Broken::ParseNotOption => f.write_str("`parse` returns an Option"),
            Broken::NestedPast => f.write_str("a read of the past holds no other read of the past"),
            Broken::OutsideScope(heap) => write!(
                f,
                "a store reads only the heaps its reader's snapshot holds back, not heap {}",
                heap.0
            ),
            Broken::NotPosition(v) => {
                write!(f, "a position is an epoch and an LSN, found {v:?}")
            }
            Broken::NotInstant(v) => write!(f, "an `at instant` names an instant, found {v:?}"),
            Broken::CtorLacksField(c) => write!(f, "constructor {c} of the type has the field"),
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
