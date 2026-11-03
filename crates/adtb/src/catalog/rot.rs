use std::fmt;

use crate::exec;
use crate::key::Value;
use crate::storage::Pid;
use crate::verify::Finding;

use super::Broken;
use super::Error;
use super::ids::CatIndexId;

/// Damage in a stored page or row of the catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rot {
    NegativeId(i64),
    NotAnId(Value),
    RootsRow(Vec<Value>),
    HeapIdWide(u64),
    OrderKey(Vec<u8>),
    RootsCtor {
        ctor: u32,
        fields: Vec<Value>,
    },
    /// A pending drop's row whose fields do not have its constructor's shape.
    PendingRow {
        ctor: u32,
        fields: Vec<Value>,
    },
    NoOrderKey(u64),
    OrderKeyOrder(u64),
    /// A search for leaked pages could not read these structures whole.
    Unfinished(Vec<Finding>),
    MarkedNoCatalog,
    RootPage {
        pid: Pid,
        found: (u16, u16),
    },
    /// The stored indexes do not follow from the stored schema and pending drops.
    KeptIndexes(Box<Broken>),
    /// A stored index does not key under the stored schema.
    IndexKeying(Box<Error>),
    GrantField {
        field: &'static str,
        found: Option<Value>,
    },
    GrantNoId,
    RoleRow(Vec<Value>),
    UntaggedGrant(i64),
    KeptRow(Vec<Value>),
    KeptFormat(u64),
    DroppedTwice {
        module: String,
        name: String,
        first: u64,
        second: u64,
    },
    DropVersion(u64),
    UniqueNotDropped {
        index: CatIndexId,
        version: u64,
    },
    Origin(u64),
    DeploymentRow {
        ctor: u32,
        fields: Vec<Value>,
    },
    MigrationRow,
    MigrationHash,
    NoMigration(u64),
    Exec(exec::Rot),
}

impl From<Rot> for Error {
    fn from(r: Rot) -> Error {
        Error::Corrupt(r)
    }
}

/// Returns `e` as its `Debug` reads, with an internal or damage cause as the `Debug` of its text.
pub fn detail(e: &Error) -> String {
    match e {
        Error::Internal(b) => format!("Internal({:?})", b.to_string()),
        Error::Corrupt(r) => format!("Corrupt({:?})", r.to_string()),
        e => format!("{e:?}"),
    }
}

impl fmt::Display for Rot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rot::NegativeId(n) => write!(f, "a roots row holds negative id {n}"),
            Rot::NotAnId(v) => write!(f, "a roots row holds {v:?} for an id"),
            Rot::RootsRow(row) => write!(f, "a roots row is {row:?}"),
            Rot::HeapIdWide(id) => write!(f, "heap id {id} is past 32 bits"),
            Rot::OrderKey(key) => write!(f, "order key {key:?} is not valid"),
            Rot::RootsCtor { ctor, fields } | Rot::PendingRow { ctor, fields } => {
                write!(f, "a roots row of constructor {ctor} holds {fields:?}")
            }
            Rot::NoOrderKey(id) => write!(f, "constructor {id} has no stored order key"),
            Rot::OrderKeyOrder(id) => write!(
                f,
                "the order key of constructor {id} does not follow the one before it"
            ),
            Rot::Unfinished(findings) => write!(f, "{findings:?}"),
            Rot::MarkedNoCatalog => {
                f.write_str("a marked database has no whole catalog and writes none")
            }
            Rot::RootPage { pid, found } => write!(
                f,
                "the header's catalog root {pid:?} has kind and format {found:?}"
            ),
            Rot::KeptIndexes(b) => write!(f, "{b}"),
            Rot::IndexKeying(e) => f.write_str(&detail(e)),
            Rot::GrantField { field, found } => write!(f, "a grant row's {field} is {found:?}"),
            Rot::GrantNoId => f.write_str("a grant row has no I64 id"),
            Rot::RoleRow(row) => write!(f, "a role row is {row:?}"),
            Rot::UntaggedGrant(id) => write!(f, "grant row {id} records no grant of the log"),
            Rot::KeptRow(fields) => write!(f, "a kept row holds {fields:?}"),
            Rot::KeptFormat(format) => write!(
                f,
                "a kept row has format {format}; this engine reads format {}",
                super::kept::ROW_FORMAT
            ),
            Rot::DroppedTwice {
                module,
                name,
                first,
                second,
            } => write!(
                f,
                "{module}::{name} is held as dropped at versions {first} and {second}"
            ),
            Rot::DropVersion(v) => write!(
                f,
                "a pending drop names version {v}, which the log does not hold"
            ),
            Rot::UniqueNotDropped { index, version } => write!(
                f,
                "unique {index} is held as dropped at version {version}, which does not drop it"
            ),
            Rot::Origin(n) => write!(f, "registration origin {n}"),
            Rot::DeploymentRow { ctor, fields } => {
                write!(f, "a deployment row of constructor {ctor} holds {fields:?}")
            }
            Rot::MigrationRow => f.write_str("malformed migration row"),
            Rot::MigrationHash => f.write_str("a migration hash must hold 32 bytes"),
            Rot::NoMigration(v) => write!(f, "missing migration {v}"),
            Rot::Exec(r) => write!(f, "{r}"),
        }
    }
}

#[cfg(test)]
#[path = "rot_tests.rs"]
mod tests;
