use std::time::Duration;

use adtb_lang::schema::Schema;
use adtb_lang::textual::{Form, print};
use adtb_lang::types::{RelId, Ty};

use crate::exec::{Exceeded, Val};
use crate::mvcc::Xid;
use crate::write::{self, Rejection};

pub use crate::exec::Fault;

use super::Position;

/// A failure the engine reports to a caller, as the prelude sum `Error` gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Retry { reason: RetryReason },
    Rejected { reason: RejectReason },
    Fatal { reason: FatalReason },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetryReason {
    /// `txn` names the caller's aborted transaction; `on` the constructor relation of the
    /// caller's own read or write on the conflicting edge, or `""` when it is not known.
    Serialization {
        txn: i64,
        on: String,
    },
    SafePointTimeout {
        waited: Duration,
    },
    Overloaded {
        retry_after: Duration,
    },
    StreamIdle {
        limit: Duration,
    },
    FileChanged {
        path: String,
    },
    Fenced,
    ReplayConflict {
        waited: Duration,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RejectReason {
    /// `report` holds the rows of the violated constraint's result.
    Violated {
        constraint: String,
        report: Vec<Val>,
        redacted: i64,
    },
    /// `key` is the key in the textual syntax.
    KeyViolation {
        relation: String,
        key: String,
    },
    KeyChanged {
        relation: String,
    },
    NotOpen {
        ctor: String,
        stage: String,
    },
    UserError {
        message: String,
    },
    Faulted {
        fault: Fault,
    },
    Denied {
        detail: String,
    },
    Invalid {
        detail: String,
    },
    SchemaRefused {
        operator: String,
        detail: String,
    },
    Untranslatable {
        query: String,
        detail: String,
    },
    BadInput {
        source: String,
        line: i64,
        detail: String,
    },
    SchemaMismatch {
        detail: String,
    },
    Cancelled,
    Terminated,
    DeadlineExceeded,
    StatementLimit {
        limit: Limit,
    },
    TransactionExpired {
        limit: Duration,
        idle: bool,
    },
    TransactionBusy,
    MixedVersions,
    UnknownQuery,
    UnknownTransaction,
    UnknownBundle {
        bundle: String,
    },
    VersionMismatch {
        bundle: String,
        version: i64,
    },
    CatalogMismatch {
        bundle: String,
        catalog: i64,
    },
    Retired {
        bundle: String,
    },
    BundleExpired {
        bundle: String,
        query: String,
    },
    BundleStale {
        bundle: String,
        ctor: String,
    },
    BundleSuperseded {
        bundle: String,
    },
    DowngradeRefused {
        cause: String,
    },
    TooOld {
        oldest: Position,
    },
    HistoryUnavailable {
        oldest: Position,
    },
    PositionNotOnHistory {
        position: Position,
    },
    WrongVersion {
        version: i64,
    },
    ReadOnlyReplica {
        primary: String,
    },
    RequestExpired,
    RequestReused,
    AuthFailed,
    UnsupportedProtocol {
        min: String,
        max: String,
    },
    UnknownDatabase {
        database: String,
    },
    EditionLocked,
    UnknownEdition {
        edition: String,
    },
    MigrationInProgress,
    DumpDamaged {
        offset: i64,
    },
    Internal {
        what: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Limit {
    Time { after: Duration },
    Memory { bytes: i64 },
    Rounds { rounds: i64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FatalReason {
    Malformed { detail: String },
    Quarantined { what: String },
    MissingFunction { function: String },
    Storage { detail: String },
}

#[derive(Debug)]
pub enum OpenError {
    Locked { pid: Option<u32> },
    AlreadyOpen,
    NotFound,
    KeyRequired,
    WrongKey,
    FormatTooNew { written_by: String },
    Corrupt { detail: String },
    Diverged { version: u64 },
    MigrationRefused { version: u64, reason: Error },
    RegistrationRefused { bundle: String, reason: Error },
    HostFnMissing { name: String },
    HostFnMismatch { name: String },
    DowngradeRefused { cause: String },
    Io(std::io::Error),
}

impl Error {
    pub(super) fn rejected(reason: RejectReason) -> Error {
        Error::Rejected { reason }
    }

    pub(super) fn invalid(detail: impl Into<String>) -> Error {
        Error::rejected(RejectReason::Invalid {
            detail: detail.into(),
        })
    }

    pub(super) fn storage(detail: impl Into<String>) -> Error {
        Error::Fatal {
            reason: FatalReason::Storage {
                detail: detail.into(),
            },
        }
    }

    pub fn is_serialization(&self) -> bool {
        matches!(
            self,
            Error::Retry {
                reason: RetryReason::Serialization { .. }
            }
        )
    }

    /// Returns the client error of write-path error `e`, met by transaction `txn` of a database
    /// of `schema`.
    pub(super) fn of_write(schema: &Schema, txn: Xid, e: write::Error) -> Error {
        match e {
            write::Error::Rejected(r) => Error::rejected(rejection(schema, r)),
            write::Error::Fault(fault) => Error::rejected(RejectReason::Faulted { fault }),
            write::Error::StatementLimit(x) => Error::rejected(RejectReason::StatementLimit {
                limit: match x {
                    Exceeded::Memory { limit } => Limit::Memory {
                        bytes: i64::try_from(limit).unwrap_or(i64::MAX),
                    },
                    Exceeded::Rounds { limit } => Limit::Rounds {
                        rounds: i64::try_from(limit).unwrap_or(i64::MAX),
                    },
                },
            }),
            write::Error::Serialization(_) => Error::Retry {
                reason: RetryReason::Serialization {
                    txn: i64::try_from(txn.0).unwrap_or(i64::MAX),
                    on: String::new(),
                },
            },
            write::Error::Page(p) => Error::storage(format!("{p:?}")),
            write::Error::Transactions(detail) | write::Error::Corrupt(detail) => {
                Error::storage(detail)
            }
            write::Error::Unsupported(what) => {
                Error::invalid(format!("{what} does not run in this engine yet"))
            }
            write::Error::Internal(what) => Error::rejected(RejectReason::Internal { what }),
            write::Error::Refused(why) => Error::rejected(RejectReason::SchemaRefused {
                operator: String::new(),
                detail: why
                    .into_iter()
                    .map(|(c, d)| format!("constraint {c} is not admitted: {d}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            }),
        }
    }
}

fn rejection(schema: &Schema, r: Rejection) -> RejectReason {
    match r {
        Rejection::Violated { constraint, report } => RejectReason::Violated {
            constraint,
            report: report.into_iter().collect(),
            redacted: 0,
        },
        Rejection::KeyViolation { relation, key } => RejectReason::KeyViolation {
            relation: qualified(schema, relation),
            key: print_key(schema, relation, &key),
        },
        Rejection::KeyChanged { relation } => RejectReason::KeyChanged {
            relation: qualified(schema, relation),
        },
        Rejection::NotOpen { ctor, stage } => RejectReason::NotOpen {
            ctor,
            stage: format!("{stage:?}"),
        },
        Rejection::UserError { message } => RejectReason::UserError { message },
        Rejection::BadInput {
            source,
            line,
            detail,
        } => RejectReason::BadInput {
            source,
            line,
            detail,
        },
    }
}

fn qualified(schema: &Schema, rel: RelId) -> String {
    let r = schema.relation(rel);
    format!("{}::{}", schema.module(r.module).name, r.name)
}

fn print_key(schema: &Schema, rel: RelId, key: &Val) -> String {
    let form = Form::of(schema, &Ty::Key { rel, ctors: None });
    print(key, &form)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broken_checker_guarantee_is_rejected_as_internal() {
        let schema = crate::catalog::engine_schema();
        let got = Error::of_write(&schema, Xid(7), write::Error::Internal("a set".into()));
        assert_eq!(
            Error::Rejected {
                reason: RejectReason::Internal {
                    what: "a set".into()
                }
            },
            got
        );
    }

    #[test]
    fn a_serialization_failure_names_the_callers_transaction() {
        let schema = crate::catalog::engine_schema();
        let f = crate::mvcc::SerializationFailure::new(crate::mvcc::Cause::LaterCommit, Vec::new());
        let got = Error::of_write(&schema, Xid(7), write::Error::Serialization(f));
        assert!(
            matches!(
                got,
                Error::Retry {
                    reason: RetryReason::Serialization { txn: 7, .. }
                }
            ),
            "{got:?}"
        );
    }
}
