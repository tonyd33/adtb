use std::fmt;

use adtb_lang::kernel::{Const, Invalid, Kind};
use adtb_lang::textual::PrintError;

use crate::catalog;
use crate::exec::{self, Val};
use crate::plan;

use crate::storage::Quarantine;

use super::{Error, Rejection};

/// A guarantee of the checker, the planner or the write path that does not hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Broken {
    /// Planning an incremental check breaks a guarantee of the checker or the normaliser.
    BoundedPlan(plan::Error),
    /// Planning a statement's read breaks a guarantee of the checker or the normaliser.
    StatementPlan(plan::Error),
    /// Planning the whole query of a constraint or check breaks a guarantee of the checker or the
    /// normaliser.
    WholePlan(plan::Error),
    /// Compiling a plan breaks a guarantee of the planner.
    Compile(exec::Error),
    DuplicateKey(Vec<u8>),
    /// A bulk build was handed this entry after one not below it.
    UnsortedBuild(Vec<u8>),
    DenialNotSet(Val),
    /// An incremental check of the named constraint whose triggering change is empty reports a
    /// violation.
    IdleDeltaReports(String),
    UniqueUnindexed {
        position: usize,
        relation: String,
    },
    /// A unique index of the catalog holds neither the key, a declared unique nor an old key.
    UniqueUnnamed {
        index: u64,
        relation: String,
    },
    KeyNotFields(String),
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
    ReferenceUnindexed {
        relation: String,
        field: String,
    },
    /// A constraint's normal form, or a statement's term as the write path rewrites it, breaks a
    /// kernel typing rule or reads a variable nothing binds.
    IllTyped(Invalid),
    Unprintable(PrintError),
    NoValueOutcome,
    NoSettingOutcome,
    NoPointOutcome,
    NoTrialOutcome,
    CallKeyNotInt,
    ExportNotSetTyped,
    ExportNotSet,
    SettingNotRecord,
    SettingField(String),
    /// A catalog call fails with a catalog error other than a damaged page, an index entry or
    /// node that does not decode, a stored row that does not decode, a missing root or a
    /// failure of the transactions layer.
    CatalogError(catalog::Error),
    DeploymentRefused,
    /// The normal form of a checked schema holds no term of the constraint or check of this
    /// qualified name.
    NoDenial(String),
    /// A heap refused as too large a row of a change set `check_sizes` admitted, `len` bytes long
    /// against the heap's `limit`.
    Unadmitted {
        len: usize,
        limit: usize,
    },
    /// A heap the write path writes has no directory.
    NoDirectory,
    /// A heap was handed a row its layout cannot encode.
    Unencodable(crate::heap::Unencodable),
    /// A version vacuum moves to its history heap is `len` bytes, past the `limit` of that heap.
    KeptTooLarge {
        len: usize,
        limit: usize,
    },
}

/// A term's kind, equal to another when the kinds are equal, a float constant by its bits.
#[derive(Clone, Debug)]
pub struct TermKind(pub Kind);

impl PartialEq for TermKind {
    fn eq(&self, other: &TermKind) -> bool {
        match (&self.0, &other.0) {
            (Kind::Const(Const::Float(a)), Kind::Const(Const::Float(b))) => {
                a.to_bits() == b.to_bits()
            }
            (a, b) => a == b,
        }
    }
}

impl Eq for TermKind {}

impl From<Broken> for Error {
    fn from(b: Broken) -> Error {
        Error::Internal(Box::new(b))
    }
}

/// Returns `e` as a person reads it, each quarantined object named by `name`.
pub(super) fn detail_naming(e: &Error, name: impl Fn(Quarantine) -> String) -> String {
    match e {
        Error::Rejected(r) => rejection(r),
        Error::Quarantined { objects, .. } => {
            format!("{} is quarantined", super::quarantine::named(objects, name))
        }
        Error::Transactions(e) => e.to_string(),
        Error::Unarmed(why) => why.to_string(),
        Error::Corrupt(r) => r.to_string(),
        Error::Denied(detail) => detail.clone(),
        Error::SequenceExhausted => "the generated counter exhausted its integer range".to_owned(),
        Error::Internal(b) => b.to_string(),
        Error::Exec(e) => exec::detail(e),
        Error::Refused(why) => why
            .iter()
            .map(|(c, d)| format!("constraint {c} is not admitted: {d}"))
            .collect::<Vec<_>>()
            .join("\n"),
        Error::FileChanged(path) => format!("`{path}` changed since the transaction read it"),
        Error::Closed => "the engine is closed".to_owned(),
        Error::Blocked { on, .. } => format!("the statement waits for transaction {}", on.0),
    }
}

/// Returns `e` as a person reads it, each quarantined object named by its kind and id.
pub fn detail(e: &Error) -> String {
    detail_naming(e, |o| match o {
        Quarantine::Database => "the database".to_owned(),
        Quarantine::Heap(id) => format!("heap {id}"),
        Quarantine::Index(id) => format!("index {id}"),
    })
}

fn rejection(r: &Rejection) -> String {
    match r {
        Rejection::Violated { constraint, .. } => format!("constraint {constraint} is violated"),
        Rejection::KeyViolation { relation, .. } => {
            format!("two rows of relation {} hold one key", relation.0)
        }
        Rejection::KeyChanged { relation } => {
            format!(
                "an update gave a row of relation {} another key",
                relation.0
            )
        }
        Rejection::NotOpen { ctor, stage } => {
            format!("{ctor} takes no write while {}", stage.name())
        }
        Rejection::UserError { message } => message.clone(),
        Rejection::Invalid { detail } | Rejection::BadInput { detail, .. } => detail.clone(),
        Rejection::RowTooLarge { size, limit, .. } => {
            format!("a row of {size} bytes is over the limit of {limit}")
        }
        Rejection::KeyTooLarge { size, limit, .. } => {
            format!("an index key of {size} bytes is over the limit of {limit}")
        }
    }
}

impl fmt::Display for Broken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Broken::BoundedPlan(plan::Error::Internal(b))
            | Broken::StatementPlan(plan::Error::Internal(b))
            | Broken::WholePlan(plan::Error::Internal(b)) => write!(f, "{b}"),
            Broken::BoundedPlan(e) | Broken::StatementPlan(e) | Broken::WholePlan(e) => {
                write!(f, "{e}")
            }
            Broken::Compile(e) => f.write_str(&exec::detail(e)),
            Broken::DuplicateKey(key) => write!(f, "a duplicate of key {key:?} in a unique index"),
            Broken::UnsortedBuild(entry) => {
                write!(
                    f,
                    "a bulk build's entry {entry:?} is not above the one before it"
                )
            }
            Broken::DenialNotSet(v) => write!(f, "a denial query returns a set, found {v:?}"),
            Broken::IdleDeltaReports(c) => write!(
                f,
                "an incremental check of `{c}` whose triggering change is empty reports a violation"
            ),
            Broken::UniqueUnindexed { position, relation } => {
                write!(f, "unique {position} of relation {relation} has an index")
            }
            Broken::UniqueUnnamed { index, relation } => {
                write!(
                    f,
                    "unique index {index} of relation {relation} holds a named unique"
                )
            }
            Broken::KeyNotFields(relation) => write!(f, "the key of {relation} is fields"),
            Broken::ReadOnly => f.write_str("a read-only transaction runs no write"),
            Broken::ScanPathNotText => f.write_str("a file scan's path is a Text"),
            Broken::RowsNotSet(v) => write!(f, "a write's rows are a set, found {v:?}"),
            Broken::FailNotText => f.write_str("`fail` takes a Text"),
            Broken::NoArm(c) => write!(f, "a case has an arm for constructor {c}"),
            Broken::Unadmitted { len, limit } => write!(
                f,
                "a heap writes every row its admission passed, found {len} bytes against the limit of {limit}"
            ),
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
            Broken::NoDirectory => f.write_str("a heap the write path writes has a directory"),
            Broken::Unencodable(e) => write!(f, "a heap is handed rows of its layout, found {e}"),
            Broken::KeptTooLarge { len, limit } => write!(
                f,
                "a version vacuum moves fits its history heap, found {len} bytes over the limit of {limit}"
            ),
            Broken::ReferenceUnindexed { relation, field } => write!(
                f,
                "a reference field has an index: {relation}.{field} has none"
            ),
            Broken::IllTyped(e) => write!(
                f,
                "a constraint's kernel term {:?} satisfies its kind's typing rule: {}",
                e.at(),
                e.reason()
            ),
            Broken::Unprintable(e) => write!(f, "printing a cell: {e:?}"),
            Broken::NoValueOutcome | Broken::NoSettingOutcome | Broken::NoPointOutcome => {
                f.write_str("a statement of one item has one outcome")
            }
            Broken::NoTrialOutcome => f.write_str("a trial of one statement has one outcome"),
            Broken::CallKeyNotInt => f.write_str("a key of `sys::calls` is an I64"),
            Broken::ExportNotSetTyped | Broken::ExportNotSet => {
                f.write_str("an export's source is a set")
            }
            Broken::SettingNotRecord => f.write_str("a setting's value is a record"),
            Broken::SettingField(name) => write!(f, "a setting's value has field `{name}`"),
            Broken::CatalogError(e) => f.write_str(&catalog::detail(e)),
            Broken::DeploymentRefused => f.write_str("a deployment command was refused"),
            Broken::NoDenial(name) => write!(f, "constraint or check {name} has a normal form"),
        }
    }
}

#[cfg(test)]
#[path = "broken_tests.rs"]
mod tests;
