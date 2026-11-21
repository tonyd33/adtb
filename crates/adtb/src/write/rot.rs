use std::fmt;

use crate::{btree, catalog};

use super::Error;

/// Damage in a stored structure that a statement reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rot {
    Index(btree::Error),
    NoKeyIndex(String),
    Catalog(catalog::Rot),
    /// The stored catalog has no root for the structure named.
    NoRoot(String),
    /// The relation of this qualified name holds no row of the key of a row the statement
    /// targets.
    Unkeyed(String),
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
            Rot::Catalog(r) => write!(f, "{r}"),
            Rot::NoRoot(what) => write!(f, "the stored catalog has no root for {what}"),
            Rot::Unkeyed(relation) => write!(
                f,
                "relation {relation} holds no row of the key of a row the statement targets"
            ),
        }
    }
}
