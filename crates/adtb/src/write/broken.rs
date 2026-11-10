use std::fmt;

use adtb_lang::kernel::Kind;
use adtb_lang::textual::PrintError;
use adtb_lang::types::{RelId, Ty};

use crate::exec::Val;
use crate::plan::{self, IndexId};
use crate::{api, catalog};

use super::Error;

/// A guarantee of the checker, the planner or the write path that does not hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Broken {
    /// Planning an incremental check breaks a guarantee of the checker or the normaliser.
    BoundedPlan(plan::Error),
    /// Planning a statement's read breaks a guarantee of the checker or the normaliser.
    StatementPlan(plan::Error),
    ReindexNoHeaps(RelId),
    VacuumNoHeaps(RelId),
    UnstoredIndex {
        position: usize,
        relation: String,
    },
    /// A rebuild of indexes panicked with this text.
    Panicked(String),
    NotAnIndex {
        index: IndexId,
        relation: String,
    },
    DuplicateKey(Vec<u8>),
    DenialNotSet(Val),
    UniqueUnindexed {
        position: usize,
        relation: String,
    },
    UndeclaredIndex {
        index: IndexId,
        relation: String,
    },
    KeyNotFields(String),
    VacuumTooLarge {
        len: usize,
        limit: usize,
    },
    HeapNoDirectory,
    ReadOnly,
    ScanPathNotText,
    RowsNotSet(Val),
    FailNotText,
    NoArm(u32),
    Unreachable,
    NotAWrite(TermKind),
    ReplaceNotLam,
    RejectsWraps(TermKind),
    NotABoundWrite(TermKind),
    ScanUnread,
    RejectedRowNotRecord,
    NoUnique(u32),
    UniqueIndexMissing,
    ReferenceUnindexed {
        relation: String,
        field: String,
    },
    TopNotFor,
    RowsNotSetTyped(Ty),
    InputNotSet(Ty),
    TopBinder(TermKind),
    PriorNotSet(Ty),
    Unprintable(PrintError),
    Undeclared {
        module: String,
        name: String,
    },
    NoValueOutcome,
    NoSettingOutcome,
    NoPointOutcome,
    NoTrialOutcome,
    /// Recording the refusal of a call fails.
    RefusalUnrecorded(Box<api::Error>),
    /// Recording the denial of a verify's witnesses fails.
    WitnessDenialUnrecorded(Box<api::Error>),
    CallKeyNotInt,
    ExportNotSetTyped,
    ExportNotSet,
    NoViolation,
    SettingNotRecord,
    SettingField(String),
    /// Reading the denials of a verify fails.
    Denials(catalog::Error),
    /// Writing deployment facts fails.
    Facts(catalog::Error),
    DeploymentRefused,
}

/// A term's kind, equal to another when their `Debug` texts are equal.
#[derive(Clone, Debug)]
pub struct TermKind(pub Kind);

impl PartialEq for TermKind {
    fn eq(&self, other: &TermKind) -> bool {
        format!("{:?}", self.0) == format!("{:?}", other.0)
    }
}

impl Eq for TermKind {}

impl From<Broken> for Error {
    fn from(b: Broken) -> Error {
        Error::Internal(Box::new(b))
    }
}

/// Returns `e` as its `Debug` reads, with an internal or damage cause as the `Debug` of its text
/// and a quarantined failure as the `Debug` of its objects' names, each object named by its own
/// `Debug`.
pub fn detail(e: &Error) -> String {
    match e {
        Error::Quarantined { objects, .. } => {
            format!(
                "Quarantined({:?})",
                super::quarantine::named(objects, |o| format!("{o:?}"))
            )
        }
        Error::Internal(b) => format!("Internal({:?})", b.to_string()),
        Error::Corrupt(r) => format!("Corrupt({:?})", r.to_string()),
        e => format!("{e:?}"),
    }
}

impl fmt::Display for Broken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Broken::BoundedPlan(plan::Error::Internal(b))
            | Broken::StatementPlan(plan::Error::Internal(b)) => write!(f, "{b}"),
            Broken::BoundedPlan(e) | Broken::StatementPlan(e) => write!(f, "{e}"),
            Broken::ReindexNoHeaps(rel) => write!(f, "reindex of {rel:?}, which has no heaps"),
            Broken::VacuumNoHeaps(rel) => write!(f, "vacuum of {rel:?}, which has no heaps"),
            Broken::UnstoredIndex { position, relation } => {
                write!(f, "index {position} of {relation} is stored")
            }
            Broken::Panicked(text) => f.write_str(text),
            Broken::NotAnIndex { index, relation } => {
                write!(f, "{index:?} is an index of {relation}")
            }
            Broken::DuplicateKey(key) => write!(f, "a duplicate of key {key:?} in a unique index"),
            Broken::DenialNotSet(v) => write!(f, "a denial query returns a set, found {v:?}"),
            Broken::UniqueUnindexed { position, relation } => {
                write!(f, "unique {position} of relation {relation} has an index")
            }
            Broken::UndeclaredIndex { index, relation } => write!(
                f,
                "index {} of relation {relation} is neither declared nor an old key",
                index.0
            ),
            Broken::KeyNotFields(relation) => write!(f, "the key of {relation} is fields"),
            Broken::VacuumTooLarge { len, limit } => write!(
                f,
                "a version vacuum moves fits its history heap, as it fit its current heap: \
                 {len} bytes over the limit of {limit}"
            ),
            Broken::HeapNoDirectory => f.write_str("a heap the write path writes has a directory"),
            Broken::ReadOnly => f.write_str("a read-only transaction runs no write"),
            Broken::ScanPathNotText => f.write_str("a file scan's path is a Text"),
            Broken::RowsNotSet(v) => write!(f, "a write's rows are a set, found {v:?}"),
            Broken::FailNotText => f.write_str("`fail` takes a Text"),
            Broken::NoArm(c) => write!(f, "a case has an arm for constructor {c}"),
            Broken::Unreachable => f.write_str(
                "evaluation reaches no position that exhaustiveness or a dominating test excludes",
            ),
            Broken::NotAWrite(k) => write!(f, "a write term is a write, found {:?}", k.0),
            Broken::ReplaceNotLam => f.write_str("a replace's new value is a `Lam`"),
            Broken::RejectsWraps(k) => {
                write!(f, "`rejects into` wraps an insert or upsert, not {:?}", k.0)
            }
            Broken::NotABoundWrite(k) => write!(
                f,
                "a bound write is a replace, an upsert or `rejects into`, found {:?}",
                k.0
            ),
            Broken::ScanUnread => {
                f.write_str("every file scan of a statement is read before it runs")
            }
            Broken::RejectedRowNotRecord => f.write_str("`RejectedRow` is a declared record"),
            Broken::NoUnique(i) => write!(f, "an upsert names unique {i} of its relation"),
            Broken::UniqueIndexMissing => f.write_str("a unique's index is one of its relation's"),
            Broken::ReferenceUnindexed { relation, field } => write!(
                f,
                "a reference field has an index: {relation}.{field} has none"
            ),
            Broken::TopNotFor => f.write_str("a top level read is the source of a `For`"),
            Broken::RowsNotSetTyped(t) => write!(f, "a read of rows is a set, not {t:?}"),
            Broken::InputNotSet(t) => write!(f, "a change set input is a set, not {t:?}"),
            Broken::TopBinder(k) => {
                write!(f, "a top level binder is a `For` or a `Let`, not {:?}", k.0)
            }
            Broken::PriorNotSet(t) => write!(f, "a prior relation is a set, not {t:?}"),
            Broken::Unprintable(e) => write!(f, "printing a cell: {e:?}"),
            Broken::Undeclared { module, name } => {
                write!(f, "the engine declares `{module}::{name}`")
            }
            Broken::NoValueOutcome | Broken::NoSettingOutcome | Broken::NoPointOutcome => {
                f.write_str("a statement of one item has one outcome")
            }
            Broken::NoTrialOutcome => f.write_str("a trial of one statement has one outcome"),
            Broken::RefusalUnrecorded(e) | Broken::WitnessDenialUnrecorded(e) => {
                write!(f, "recording a denied call: {e:?}")
            }
            Broken::CallKeyNotInt => f.write_str("a key of `sys::calls` is an I64"),
            Broken::ExportNotSetTyped | Broken::ExportNotSet => {
                f.write_str("an export's source is a set")
            }
            Broken::NoViolation => f.write_str("`Finding` declares `Violation`"),
            Broken::SettingNotRecord => f.write_str("a setting's value is a record"),
            Broken::SettingField(name) => write!(f, "a setting's value has field `{name}`"),
            Broken::Denials(e) | Broken::Facts(e) => f.write_str(&catalog::detail(e)),
            Broken::DeploymentRefused => f.write_str("a deployment command was refused"),
        }
    }
}

#[cfg(test)]
#[path = "broken_tests.rs"]
mod tests;
