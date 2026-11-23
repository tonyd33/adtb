use std::fmt;

use crate::exec;
use crate::key::{Value, hex, listed};
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
    /// A stored index has this id, which no index takes.
    IndexOwner(u64),
    /// The stored indexes do not follow from the stored schema and pending drops.
    KeptIndexes(Box<Broken>),
    /// A stored index does not key under the stored schema.
    IndexKeying(Box<Error>),
    /// A row of a catalog relation, `row` naming the relation and its constructor when it has one,
    /// that does not have the shape the relation declares.
    CatalogRow {
        row: String,
        detail: String,
    },
    /// Field `field` of a row of a catalog relation, `row` naming the relation and its
    /// constructor, holds a value outside the field's range.
    OutOfRange {
        row: String,
        field: &'static str,
    },
    UntaggedGrant(i64),
    KeptRow(Vec<Value>),
    /// Two rows hold this keyed fact.
    Twice(String),
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
    NoMigration(u64),
    Exec(exec::Rot),
}

impl From<Rot> for Error {
    fn from(r: Rot) -> Error {
        Error::Corrupt(r)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&detail(self))
    }
}

/// Returns `e` as a person reads it.
pub fn detail(e: &Error) -> String {
    match e {
        Error::Page(e) => e.to_string(),
        Error::Heap(e) => e.to_string(),
        Error::Index(e) => e.to_string(),
        Error::Txn(e) => e.to_string(),
        Error::Missing(what) => format!("the stored catalog has no root for {what}"),
        Error::Refused(why) => why.join("\n"),
        Error::RefusedBy { operator, why } => why
            .iter()
            .map(|w| format!("`{operator}`: {w}"))
            .collect::<Vec<_>>()
            .join("\n"),
        Error::Corrupt(r) => r.to_string(),
        Error::Unaccepted(version, why) => {
            format!(
                "stored migration {version} does not apply: {}",
                why.join("; ")
            )
        }
        Error::Internal(b) => b.to_string(),
        Error::MissingFunction(name) => format!("host function {name} has no body bound"),
        Error::OtherVersion(name) => {
            format!("host function {name} is bound at another version than declared")
        }
        Error::Unchecked(built, diags) => format!(
            "{built} does not check: {}",
            diags
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        ),
        Error::PastLimits(object) => format!("{object} expands past this build's limits"),
        Error::Conflict(_) => {
            "another transaction wrote a key of the catalog this one writes".into()
        }
        Error::FormatTooOld(format) => format!(
            "the file is of catalog format {format}; this engine opens only format {}",
            super::root::FORMAT
        ),
    }
}

impl fmt::Display for Rot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rot::NegativeId(n) => write!(f, "a roots row holds negative id {n}"),
            Rot::NotAnId(v) => write!(f, "a roots row holds {v} for an id"),
            Rot::RootsRow(row) => write!(f, "a roots row is {}", listed(row)),
            Rot::HeapIdWide(id) => write!(f, "heap id {id} is past 32 bits"),
            Rot::OrderKey(key) => write!(f, "order key {} is not valid", hex(key)),
            Rot::RootsCtor { ctor, fields } | Rot::PendingRow { ctor, fields } => {
                write!(
                    f,
                    "a roots row of constructor {ctor} holds {}",
                    listed(fields)
                )
            }
            Rot::NoOrderKey(id) => write!(f, "constructor {id} has no stored order key"),
            Rot::OrderKeyOrder(id) => write!(
                f,
                "the order key of constructor {id} does not follow the one before it"
            ),
            Rot::Unfinished(findings) => {
                f.write_str("a search for leaked pages could not read every structure:")?;
                for (at, finding) in findings.iter().enumerate() {
                    let sep = if at == 0 { " " } else { "; " };
                    write!(f, "{sep}{finding}")?;
                }
                Ok(())
            }
            Rot::MarkedNoCatalog => {
                f.write_str("a marked database has no whole catalog and writes none")
            }
            Rot::RootPage { pid, found } => write!(
                f,
                "the header's catalog root, page {}, has kind {} and format {}",
                pid.0, found.0, found.1
            ),
            Rot::IndexOwner(id) => write!(f, "a stored index has id {id}, which no index takes"),
            Rot::KeptIndexes(b) => write!(f, "{b}"),
            Rot::IndexKeying(e) => f.write_str(&detail(e)),
            Rot::CatalogRow { row, detail } => write!(f, "a row of {row} {detail}"),
            Rot::OutOfRange { row, field } => {
                write!(f, "a row of {row} holds `{field}` outside its range")
            }
            Rot::UntaggedGrant(id) => write!(f, "grant row {id} records no grant of the log"),
            Rot::KeptRow(fields) => write!(f, "a kept row holds {}", listed(fields)),
            Rot::Twice(what) => write!(f, "two rows hold {what}"),
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
                write!(
                    f,
                    "a deployment row of constructor {ctor} holds {}",
                    listed(fields)
                )
            }
            Rot::NoMigration(v) => write!(f, "missing migration {v}"),
            Rot::Exec(r) => write!(f, "{r}"),
        }
    }
}
