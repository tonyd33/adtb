use std::fmt;

use super::layout::Step;
use crate::btree;

use super::{Error, Exceeded};
use crate::storage::io_text;

/// Damage in a stored structure or a spill file that a plan reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rot {
    SpillShort,
    SpillCut,
    SpillUndecoded,
    SpillNotPair,
    DeferredNoVariable,
    /// A deferred row binds a variable twice, out of order or past the plan's variables.
    DeferredShape,
    Index(btree::Error),
    StoredPath(Vec<Step>),
    RetentionWindow(btree::Error),
    Markers(btree::Error),
    /// A migration the record of the retention window holds names a writer that is not reserved
    /// and that no slot of its page names.
    MigrationWriter(u64),
}

impl From<Rot> for Error {
    fn from(r: Rot) -> Error {
        Error::Corrupt(r)
    }
}

/// Returns `e` as a person reads it.
pub fn detail(e: &Error) -> String {
    match e {
        Error::Fault(fault) => format!("the statement faulted with {}", fault.name()),
        Error::StatementLimit(Exceeded::Memory { limit }) => {
            format!("the statement exceeded its memory grant of {limit} bytes")
        }
        Error::StatementLimit(Exceeded::Spill { limit, grant }) => format!(
            "the statement exceeded its spill bound of {limit} bytes past its memory grant of \
             {grant} bytes"
        ),
        Error::StatementLimit(Exceeded::Rounds { limit }) => {
            format!("a fixpoint ran past {limit} rounds")
        }
        Error::StatementLimit(Exceeded::Time { after }) => {
            format!(
                "the statement ran past its time limit of {} ms",
                after.as_millis()
            )
        }
        Error::Serialization(_) => "the transaction conflicts with a concurrent one".to_owned(),
        Error::Page(p) => p.to_string(),
        Error::OrphanEntry(index) => format!("an entry of index {index} names no version"),
        Error::Spill(e) => format!("the spill file failed: {}", io_text(*e)),
        Error::Corrupt(r) => r.to_string(),
        Error::Unsupported(what) => format!("{what} does not run in this engine yet"),
        Error::NoCaller => "the plan reads `caller` and the statement has none".to_owned(),
        Error::Rejected(text) => text.clone(),
        Error::Cancelled => "the call was cancelled".to_owned(),
        Error::Overloaded => "the queue of calls waiting for memory was full".to_owned(),
        Error::DeadlineExceeded => "the call's deadline passed".to_owned(),
        Error::MissingFunction(name) => format!("host function {name} has no body bound"),
        Error::OtherVersion(name) => {
            format!("host function {name} is bound at another version than declared")
        }
        Error::Internal(b) => b.to_string(),
        Error::HistoryUnavailable { oldest } => format!(
            "the engine keeps no history before epoch {} LSN {}",
            oldest.epoch, oldest.lsn.0
        ),
        Error::WrongVersion { version } => {
            format!("the state read is typed at another version than its version {version}")
        }
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
            Rot::DeferredShape => f.write_str(
                "a deferred row binds a variable twice, out of order or past the plan's variables",
            ),
            Rot::Index(e) => write!(f, "{e}"),
            Rot::Markers(e) => write!(f, "the safe snapshot markers are damaged: {e}"),
            Rot::StoredPath(steps) => {
                f.write_str("a stored row does not hold path")?;
                for step in steps {
                    match step {
                        Step::Field(at) => write!(f, " .{at}")?,
                        Step::Ctor(c) => write!(f, " #{c}")?,
                    }
                }
                Ok(())
            }
            Rot::RetentionWindow(e) => {
                write!(f, "the record of the retention window is damaged: {e}")
            }
            Rot::MigrationWriter(x) => write!(
                f,
                "a migration the retention window records names transaction {x}, which is not \
                 reserved"
            ),
        }
    }
}
