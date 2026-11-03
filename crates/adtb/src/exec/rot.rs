use std::fmt;

use crate::btree;
use crate::catalog::Step;

use super::Error;

/// Damage in a stored structure or a spill file that a plan reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rot {
    SpillShort,
    SpillCut,
    SpillUndecoded,
    SpillNotPair,
    DeferredNoVariable,
    Index(btree::Error),
    StoredPath(Vec<Step>),
    RetentionWindow(btree::Error),
    Markers(btree::Error),
}

impl From<Rot> for Error {
    fn from(r: Rot) -> Error {
        Error::Corrupt(r)
    }
}

/// Returns `e` as its `Debug` reads, with a damage cause as the `Debug` of its text.
pub fn detail(e: &Error) -> String {
    match e {
        Error::Corrupt(r) => format!("Corrupt({:?})", r.to_string()),
        e => format!("{e:?}"),
    }
}

impl fmt::Display for Rot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rot::SpillShort => f.write_str("a spilled extent reads back short"),
            Rot::SpillCut => f.write_str("a spilled record is cut short"),
            Rot::SpillUndecoded => f.write_str("a spilled value does not decode"),
            Rot::SpillNotPair => f.write_str("a spilled build record is not a pair"),
            Rot::DeferredNoVariable => f.write_str("a deferred row names no variable"),
            Rot::Index(e) | Rot::Markers(e) => write!(f, "{e:?}"),
            Rot::StoredPath(steps) => write!(f, "a stored row does not hold path {steps:?}"),
            Rot::RetentionWindow(e) => {
                write!(f, "the record of the retention window is damaged: {e:?}")
            }
        }
    }
}

#[cfg(test)]
#[path = "rot_tests.rs"]
mod tests;
