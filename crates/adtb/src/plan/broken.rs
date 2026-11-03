use std::fmt;

use adtb_lang::types::Ty;

use super::Error;

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
        }
    }
}

#[cfg(test)]
#[path = "broken_tests.rs"]
mod tests;
