use std::fmt;

use crate::{btree, exec};

use super::Error;

/// Damage in a stored structure that a statement reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rot {
    Index(btree::Error),
    NoKeyIndex(String),
    Exec(exec::Rot),
}

impl From<Rot> for Error {
    fn from(r: Rot) -> Error {
        Error::Corrupt(r)
    }
}

impl fmt::Display for Rot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rot::Index(e) => write!(f, "{e:?}"),
            Rot::NoKeyIndex(relation) => write!(f, "relation {relation} has no key index"),
            Rot::Exec(r) => write!(f, "{r}"),
        }
    }
}

#[cfg(test)]
#[path = "rot_tests.rs"]
mod tests;
