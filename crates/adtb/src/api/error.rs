use std::fmt;
use std::time::Duration;

use adtb_lang::schema::Schema;
use adtb_lang::span::{Diag, DiagKind};
use adtb_lang::textual::{Form, PrintError, print};
use adtb_lang::types::{Base, RelId, Ty};

use crate::exec::{Exceeded, Val};
use crate::write::{self, Rejection};
use crate::{catalog, heap};

pub use crate::exec::Fault;

use super::{Position, TxnId};

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
        txn: TxnId,
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
    RowTooLarge {
        relation: String,
        ctor: String,
        size: i64,
        limit: i64,
    },
    Closed,
    KeyTooLarge {
        relation: String,
        ctor: String,
        index: String,
        size: i64,
        limit: i64,
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

    pub(super) fn diagnostic(diag: Diag) -> Error {
        match diag.kind {
            DiagKind::Invalid => Self::invalid(diag.message),
            DiagKind::Denied => Self::rejected(RejectReason::Denied {
                detail: diag.message,
            }),
        }
    }

    pub(super) fn diagnostics(diags: Vec<Diag>) -> Error {
        let kind = if diags.iter().any(|diag| diag.kind == DiagKind::Denied) {
            DiagKind::Denied
        } else {
            DiagKind::Invalid
        };
        Self::diagnostic(Diag {
            kind,
            span: adtb_lang::span::Span::default(),
            message: diags
                .into_iter()
                .map(|diag| diag.message)
                .collect::<Vec<_>>()
                .join("\n"),
        })
    }

    pub(super) fn closed() -> Error {
        Error::rejected(RejectReason::Closed)
    }

    pub(super) fn storage(detail: impl Into<String>) -> Error {
        Error::Fatal {
            reason: FatalReason::Storage {
                detail: detail.into(),
            },
        }
    }

    /// Returns the error as its value of the prelude sum `Error`, on one line.
    pub fn value(&self) -> impl fmt::Display + '_ {
        Value(self)
    }

    pub fn is_serialization(&self) -> bool {
        matches!(
            self,
            Error::Retry {
                reason: RetryReason::Serialization { .. }
            }
        )
    }

    /// Returns the `KeyViolation` of two rows of `relation` with `key`, of a database of `schema`.
    pub(super) fn key_violation(schema: &Schema, relation: RelId, key: &Val) -> Error {
        Error::rejected(rejection(
            schema,
            Rejection::KeyViolation {
                relation,
                key: key.clone(),
            },
        ))
    }

    /// Returns the client error of `e`, met writing catalog rows in a transaction of the write
    /// path.
    pub(super) fn of_catalog_write(e: catalog::Error) -> Error {
        match e {
            catalog::Error::Page(p) | catalog::Error::Heap(heap::Error::Page(p)) => {
                Error::storage(format!("{p:?}"))
            }
            other => Error::rejected(RejectReason::Internal {
                what: format!("{other:?}"),
            }),
        }
    }

    /// Returns the client error of write-path error `e`, met by transaction `txn` of a database
    /// of `schema`.
    pub(super) fn of_write(schema: &Schema, txn: TxnId, e: write::Error) -> Error {
        match e {
            write::Error::Rejected(r) => Error::rejected(rejection(schema, r)),
            write::Error::Fault(fault) => Error::rejected(RejectReason::Faulted { fault }),
            write::Error::StatementLimit(x) => Error::rejected(RejectReason::StatementLimit {
                limit: match x {
                    Exceeded::Memory { limit } | Exceeded::Spill { grant: limit, .. } => {
                        Limit::Memory {
                            bytes: i64::try_from(limit).unwrap_or(i64::MAX),
                        }
                    }
                    Exceeded::Rounds { limit } => Limit::Rounds {
                        rounds: i64::try_from(limit).unwrap_or(i64::MAX),
                    },
                    Exceeded::Time { after } => Limit::Time { after },
                },
            }),
            write::Error::Denied(detail) => Error::rejected(RejectReason::Denied { detail }),
            write::Error::Cancelled => Error::rejected(RejectReason::Cancelled),
            write::Error::DeadlineExceeded => Error::rejected(RejectReason::DeadlineExceeded),
            write::Error::MissingFunction(function) => Error::Fatal {
                reason: FatalReason::MissingFunction { function },
            },
            write::Error::Serialization(_) => Error::Retry {
                reason: RetryReason::Serialization {
                    txn,
                    on: String::new(),
                },
            },
            write::Error::Quarantined(what) => Error::Fatal {
                reason: FatalReason::Quarantined { what },
            },
            write::Error::Page(p) => Error::storage(format!("{p:?}")),
            write::Error::Spill(io) => Error::storage(format!("the spill file failed: {io:?}")),
            write::Error::Transactions(detail) | write::Error::Corrupt(detail) => {
                Error::storage(detail)
            }
            write::Error::Unsupported(what) => {
                Error::invalid(format!("{what} does not run in this engine yet"))
            }
            write::Error::SequenceExhausted => {
                Error::invalid("the generated counter exhausted its integer range")
            }
            write::Error::Internal(what) => Error::rejected(RejectReason::Internal { what }),
            write::Error::Blocked { on, .. } => Error::rejected(RejectReason::Internal {
                what: format!("a statement suspended on {on:?} outside a resumable call"),
            }),
            write::Error::HistoryUnavailable { oldest } => {
                Error::rejected(RejectReason::HistoryUnavailable {
                    oldest: oldest.into(),
                })
            }
            write::Error::WrongVersion { version } => Error::rejected(RejectReason::WrongVersion {
                version: i64::try_from(version).unwrap_or(i64::MAX),
            }),
            write::Error::FileChanged(path) => Error::Retry {
                reason: RetryReason::FileChanged { path },
            },
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

/// Write constructor `name` with `fields` in the textual syntax of a sum value.
fn ctor(f: &mut fmt::Formatter<'_>, name: &str, fields: &[(&str, &dyn fmt::Debug)]) -> fmt::Result {
    let mut s = f.debug_struct(name);
    for (field, value) in fields {
        s.field(field, value);
    }
    s.finish()
}

struct Shown<T>(T);

impl<T: fmt::Display> fmt::Debug for Shown<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

struct Txt<'a>(&'a str);

impl fmt::Debug for Txt<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = print(&Val::Text(self.0.into()), &Form::Base(Base::Text));
        f.write_str(&text.map_err(|_| fmt::Error)?)
    }
}

struct Dur(Duration);

impl fmt::Debug for Dur {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let micros = i64::try_from(self.0.as_micros()).unwrap_or(i64::MAX);
        let text = print(&Val::Duration(micros), &Form::Base(Base::Duration));
        f.write_str(&text.map_err(|_| fmt::Error)?)
    }
}

struct Rows(usize);

impl fmt::Debug for Rows {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} rows", self.0)
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{ epoch: {}, lsn: {} }}", self.epoch, self.lsn)
    }
}

/// Write the error as messages for a person, one per line, without the sum's syntax.
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Retry { reason } => write!(f, "{reason}; retry"),
            Error::Rejected { reason } => write!(f, "{reason}"),
            Error::Fatal { reason } => write!(f, "fatal: {reason}"),
        }
    }
}

impl fmt::Display for RetryReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RetryReason::Serialization { txn, on } if on.is_empty() => write!(
                f,
                "transaction {} conflicts with a concurrent transaction",
                txn.0
            ),
            RetryReason::Serialization { txn, on } => write!(
                f,
                "transaction {} conflicts with a concurrent transaction on `{on}`",
                txn.0
            ),
            RetryReason::SafePointTimeout { waited } => {
                write!(f, "no safe point was reached in {:?}", Dur(*waited))
            }
            RetryReason::Overloaded { retry_after } => {
                write!(f, "the engine is overloaded for {:?}", Dur(*retry_after))
            }
            RetryReason::StreamIdle { limit } => {
                write!(f, "the result stream was idle for over {:?}", Dur(*limit))
            }
            RetryReason::FileChanged { path } => write!(f, "file {path} changed while it was read"),
            RetryReason::Fenced => f.write_str("this node is fenced"),
            RetryReason::ReplayConflict { waited } => {
                write!(
                    f,
                    "replay conflicted with the call after {:?}",
                    Dur(*waited)
                )
            }
        }
    }
}

impl fmt::Display for RejectReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use RejectReason as R;
        match self {
            R::Violated {
                constraint,
                report,
                redacted,
            } => {
                let rows = report.len() + usize::try_from(*redacted).unwrap_or(0);
                let s = if rows == 1 { "" } else { "s" };
                write!(f, "constraint `{constraint}` is violated by {rows} row{s}")
            }
            R::KeyViolation { relation, key } => {
                write!(f, "`{relation}` already holds a row with key `{key}`")
            }
            R::KeyChanged { relation } => {
                write!(f, "a write changes the key of a row of `{relation}`")
            }
            R::NotOpen { ctor, stage } => {
                write!(f, "constructor `{ctor}` is {stage}, not open for writes")
            }
            R::UserError { message } => f.write_str(message),
            R::Faulted { fault } => write!(f, "fault `{}`", fault.name()),
            R::Denied { detail } => write!(f, "denied: {detail}"),
            R::Invalid { detail } => f.write_str(detail),
            R::SchemaRefused { operator, detail } if operator.is_empty() => f.write_str(detail),
            R::SchemaRefused { operator, detail } => {
                let lines = detail.lines().map(|l| format!("`{operator}`: {l}"));
                f.write_str(&lines.collect::<Vec<_>>().join("\n"))
            }
            R::Untranslatable { query, detail } => {
                write!(f, "query `{query}` cannot be translated: {detail}")
            }
            R::BadInput {
                source,
                line,
                detail,
            } => write!(f, "{source}:{line}: {detail}"),
            R::SchemaMismatch { detail } => write!(f, "schema mismatch: {detail}"),
            R::Cancelled => f.write_str("the call was cancelled"),
            R::Terminated => f.write_str("the session was terminated"),
            R::DeadlineExceeded => f.write_str("the call's deadline passed"),
            R::StatementLimit { limit } => write!(f, "the statement exceeded its {limit}"),
            R::TransactionExpired { limit, idle: true } => {
                write!(f, "the transaction was idle for over {:?}", Dur(*limit))
            }
            R::TransactionExpired { limit, idle: false } => {
                write!(f, "the transaction ran for over {:?}", Dur(*limit))
            }
            R::TransactionBusy => f.write_str("the transaction is running another call"),
            R::MixedVersions => f.write_str("the transaction mixes schema versions"),
            R::UnknownQuery => f.write_str("no such query"),
            R::UnknownTransaction => f.write_str("no such transaction"),
            R::UnknownBundle { bundle } => write!(f, "no bundle `{bundle}`"),
            R::VersionMismatch { bundle, version } => {
                write!(f, "bundle `{bundle}` does not match version {version}")
            }
            R::CatalogMismatch { bundle, catalog } => {
                write!(f, "bundle `{bundle}` does not match catalog {catalog}")
            }
            R::Retired { bundle } => write!(f, "bundle `{bundle}` is retired"),
            R::BundleExpired { bundle, query } if query.is_empty() => {
                write!(f, "the lease of bundle `{bundle}` expired")
            }
            R::BundleExpired { bundle, query } => {
                write!(f, "the lease of bundle `{bundle}` expired before `{query}`")
            }
            R::BundleStale { bundle, ctor } => {
                write!(f, "bundle `{bundle}` cannot read constructor `{ctor}`")
            }
            R::BundleSuperseded { bundle } => write!(f, "bundle `{bundle}` is superseded"),
            R::DowngradeRefused { cause } => write!(f, "downgrade refused: {cause}"),
            R::TooOld { oldest } => write!(f, "the position is older than {oldest}"),
            R::HistoryUnavailable { oldest } => write!(f, "no history is kept before {oldest}"),
            R::PositionNotOnHistory { position } => {
                write!(f, "position {position} is not on this database's history")
            }
            R::WrongVersion { version } => write!(f, "the database is at version {version}"),
            R::ReadOnlyReplica { primary } => {
                write!(f, "this replica is read-only; write to {primary}")
            }
            R::RequestExpired => f.write_str("the request expired"),
            R::RequestReused => f.write_str("the request number was reused"),
            R::AuthFailed => f.write_str("authentication failed"),
            R::UnsupportedProtocol { min, max } => {
                write!(f, "the protocol must be from {min} to {max}")
            }
            R::UnknownDatabase { database } => write!(f, "no database `{database}`"),
            R::EditionLocked => f.write_str("the edition is locked"),
            R::UnknownEdition { edition } => write!(f, "no edition `{edition}`"),
            R::MigrationInProgress => f.write_str("a migration is in progress"),
            R::DumpDamaged { offset } => write!(f, "the dump is damaged at byte {offset}"),
            R::Internal { what } => write!(f, "internal error: {what}"),
            R::RowTooLarge {
                relation,
                ctor,
                size,
                limit,
            } => write!(
                f,
                "a row of `{relation}.{ctor}` takes {size} bytes, over the limit of {limit}"
            ),
            R::Closed => f.write_str("the database is closed"),
            R::KeyTooLarge {
                relation,
                ctor,
                index,
                size,
                limit,
            } => write!(
                f,
                "index `{relation}.{index}` for `{ctor}` needs a key payload of {size} bytes, over the limit of {limit}"
            ),
        }
    }
}

impl fmt::Display for Limit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Limit::Time { after } => write!(f, "time limit of {:?}", Dur(*after)),
            Limit::Memory { bytes } => write!(f, "memory limit of {bytes} bytes"),
            Limit::Rounds { rounds } => write!(f, "limit of {rounds} fixpoint rounds"),
        }
    }
}

impl fmt::Display for FatalReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FatalReason::Malformed { detail } => write!(f, "malformed: {detail}"),
            FatalReason::Quarantined { what } => write!(f, "{what} is quarantined"),
            FatalReason::MissingFunction { function } => {
                write!(f, "host function `{function}` is missing")
            }
            FatalReason::Storage { detail } => write!(f, "storage failure: {detail}"),
        }
    }
}

/// Write `head`, then `body` after a colon when it is one line, or indented below when it is
/// several.
fn headed(f: &mut fmt::Formatter<'_>, head: &str, body: &dyn fmt::Display) -> fmt::Result {
    let body = body.to_string();
    if !body.contains('\n') {
        return write!(f, "{head}: {body}");
    }
    write!(f, "{head}:")?;
    body.lines().try_for_each(|l| write!(f, "\n  {l}"))
}

struct Value<T>(T);

impl fmt::Display for Value<&Error> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Error::Retry { reason } => ctor(f, "Retry", &[("reason", &Shown(Value(reason)))]),
            Error::Rejected { reason } => ctor(f, "Rejected", &[("reason", &Shown(Value(reason)))]),
            Error::Fatal { reason } => ctor(f, "Fatal", &[("reason", &Shown(Value(reason)))]),
        }
    }
}

impl fmt::Display for Value<&RetryReason> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (name, fields): (&str, &[(&str, &dyn fmt::Debug)]) = match self.0 {
            RetryReason::Serialization { txn, on } => {
                ("Serialization", &[("txn", &txn.0), ("on", &Txt(on))])
            }
            RetryReason::SafePointTimeout { waited } => {
                ("SafePointTimeout", &[("waited", &Dur(*waited))])
            }
            RetryReason::Overloaded { retry_after } => {
                ("Overloaded", &[("retry_after", &Dur(*retry_after))])
            }
            RetryReason::StreamIdle { limit } => ("StreamIdle", &[("limit", &Dur(*limit))]),
            RetryReason::FileChanged { path } => ("FileChanged", &[("path", &Txt(path))]),
            RetryReason::Fenced => ("Fenced", &[]),
            RetryReason::ReplayConflict { waited } => {
                ("ReplayConflict", &[("waited", &Dur(*waited))])
            }
        };
        ctor(f, name, fields)
    }
}

impl fmt::Display for Value<&RejectReason> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use RejectReason as R;
        let (name, fields): (&str, &[(&str, &dyn fmt::Debug)]) = match self.0 {
            R::Violated {
                constraint,
                report,
                redacted,
            } => (
                "Violated",
                &[
                    ("constraint", &Txt(constraint)),
                    ("report", &Rows(report.len())),
                    ("redacted", redacted),
                ],
            ),
            R::KeyViolation { relation, key } => (
                "KeyViolation",
                &[("relation", &Txt(relation)), ("key", &Txt(key))],
            ),
            R::KeyChanged { relation } => ("KeyChanged", &[("relation", &Txt(relation))]),
            R::NotOpen { ctor, stage } => {
                ("NotOpen", &[("ctor", &Txt(ctor)), ("stage", &Txt(stage))])
            }
            R::UserError { message } => ("UserError", &[("message", &Txt(message))]),
            R::Faulted { fault } => ("Faulted", &[("fault", fault)]),
            R::Denied { detail } => ("Denied", &[("detail", &Txt(detail))]),
            R::Invalid { detail } => ("Invalid", &[("detail", &Txt(detail))]),
            R::SchemaRefused { operator, detail } => (
                "SchemaRefused",
                &[("operator", &Txt(operator)), ("detail", &Txt(detail))],
            ),
            R::Untranslatable { query, detail } => (
                "Untranslatable",
                &[("query", &Txt(query)), ("detail", &Txt(detail))],
            ),
            R::BadInput {
                source,
                line,
                detail,
            } => (
                "BadInput",
                &[
                    ("source", &Txt(source)),
                    ("line", line),
                    ("detail", &Txt(detail)),
                ],
            ),
            R::SchemaMismatch { detail } => ("SchemaMismatch", &[("detail", &Txt(detail))]),
            R::Cancelled => ("Cancelled", &[]),
            R::Terminated => ("Terminated", &[]),
            R::DeadlineExceeded => ("DeadlineExceeded", &[]),
            R::StatementLimit { limit } => ("StatementLimit", &[("limit", &Shown(Value(limit)))]),
            R::TransactionExpired { limit, idle } => (
                "TransactionExpired",
                &[("limit", &Dur(*limit)), ("idle", idle)],
            ),
            R::TransactionBusy => ("TransactionBusy", &[]),
            R::MixedVersions => ("MixedVersions", &[]),
            R::UnknownQuery => ("UnknownQuery", &[]),
            R::UnknownTransaction => ("UnknownTransaction", &[]),
            R::UnknownBundle { bundle } => ("UnknownBundle", &[("bundle", &Txt(bundle))]),
            R::VersionMismatch { bundle, version } => (
                "VersionMismatch",
                &[("bundle", &Txt(bundle)), ("version", version)],
            ),
            R::CatalogMismatch { bundle, catalog } => (
                "CatalogMismatch",
                &[("bundle", &Txt(bundle)), ("catalog", catalog)],
            ),
            R::Retired { bundle } => ("Retired", &[("bundle", &Txt(bundle))]),
            R::BundleExpired { bundle, query } => (
                "BundleExpired",
                &[("bundle", &Txt(bundle)), ("query", &Txt(query))],
            ),
            R::BundleStale { bundle, ctor } => (
                "BundleStale",
                &[("bundle", &Txt(bundle)), ("ctor", &Txt(ctor))],
            ),
            R::BundleSuperseded { bundle } => ("BundleSuperseded", &[("bundle", &Txt(bundle))]),
            R::DowngradeRefused { cause } => ("DowngradeRefused", &[("cause", &Txt(cause))]),
            R::TooOld { oldest } => ("TooOld", &[("oldest", &Shown(oldest))]),
            R::HistoryUnavailable { oldest } => {
                ("HistoryUnavailable", &[("oldest", &Shown(oldest))])
            }
            R::PositionNotOnHistory { position } => {
                ("PositionNotOnHistory", &[("position", &Shown(position))])
            }
            R::WrongVersion { version } => ("WrongVersion", &[("version", version)]),
            R::ReadOnlyReplica { primary } => ("ReadOnlyReplica", &[("primary", &Txt(primary))]),
            R::RequestExpired => ("RequestExpired", &[]),
            R::RequestReused => ("RequestReused", &[]),
            R::AuthFailed => ("AuthFailed", &[]),
            R::UnsupportedProtocol { min, max } => (
                "UnsupportedProtocol",
                &[("min", &Txt(min)), ("max", &Txt(max))],
            ),
            R::UnknownDatabase { database } => ("UnknownDatabase", &[("database", &Txt(database))]),
            R::EditionLocked => ("EditionLocked", &[]),
            R::UnknownEdition { edition } => ("UnknownEdition", &[("edition", &Txt(edition))]),
            R::MigrationInProgress => ("MigrationInProgress", &[]),
            R::DumpDamaged { offset } => ("DumpDamaged", &[("offset", offset)]),
            R::Internal { what } => ("Internal", &[("what", &Txt(what))]),
            R::RowTooLarge {
                relation,
                ctor,
                size,
                limit,
            } => (
                "RowTooLarge",
                &[
                    ("relation", &Txt(relation)),
                    ("ctor", &Txt(ctor)),
                    ("size", size),
                    ("limit", limit),
                ],
            ),
            R::Closed => ("Closed", &[]),
            R::KeyTooLarge {
                relation,
                ctor,
                index,
                size,
                limit,
            } => (
                "KeyTooLarge",
                &[
                    ("relation", &Txt(relation)),
                    ("ctor", &Txt(ctor)),
                    ("index", &Txt(index)),
                    ("size", size),
                    ("limit", limit),
                ],
            ),
        };
        ctor(f, name, fields)
    }
}

impl fmt::Display for Value<&Limit> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Limit::Time { after } => ctor(f, "Time", &[("after", &Dur(*after))]),
            Limit::Memory { bytes } => ctor(f, "Memory", &[("bytes", bytes)]),
            Limit::Rounds { rounds } => ctor(f, "Rounds", &[("rounds", rounds)]),
        }
    }
}

impl fmt::Display for Value<&FatalReason> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (name, fields): (&str, &[(&str, &dyn fmt::Debug)]) = match self.0 {
            FatalReason::Malformed { detail } => ("Malformed", &[("detail", &Txt(detail))]),
            FatalReason::Quarantined { what } => ("Quarantined", &[("what", &Txt(what))]),
            FatalReason::MissingFunction { function } => {
                ("MissingFunction", &[("function", &Txt(function))])
            }
            FatalReason::Storage { detail } => ("Storage", &[("detail", &Txt(detail))]),
        };
        ctor(f, name, fields)
    }
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::Locked { pid: Some(pid) } => {
                write!(f, "the file is locked by process {pid}")
            }
            OpenError::Locked { pid: None } => f.write_str("the file is locked by another process"),
            OpenError::AlreadyOpen => f.write_str("the file is already open in this process"),
            OpenError::NotFound => f.write_str("the file does not exist"),
            OpenError::KeyRequired => f.write_str("the file is encrypted and needs a key"),
            OpenError::WrongKey => f.write_str("the key does not decrypt the file"),
            OpenError::FormatTooNew { written_by } => {
                write!(f, "the file was written by the newer {written_by}")
            }
            OpenError::Corrupt { detail } => write!(f, "the file is corrupt: {detail}"),
            OpenError::Diverged { version } => {
                write!(f, "the file's migration {version} differs from the log's")
            }
            OpenError::MigrationRefused { version, reason } => {
                headed(f, &format!("migration {version} is refused"), reason)
            }
            OpenError::RegistrationRefused { bundle, reason } => {
                headed(f, &format!("bundle {bundle} is refused"), reason)
            }
            OpenError::HostFnMissing { name } => write!(f, "host function {name} is not bound"),
            OpenError::HostFnMismatch { name } => {
                write!(f, "host function {name} is bound at another type")
            }
            OpenError::DowngradeRefused { cause } => write!(f, "downgrade refused: {cause}"),
            OpenError::Io(e) => write!(f, "{e}"),
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
        Rejection::KeyViolation { relation, key } => match print_key(schema, relation, &key) {
            Ok(key) => RejectReason::KeyViolation {
                relation: qualified(schema, relation),
                key,
            },
            Err(e) => RejectReason::Internal { what: e.message },
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
        Rejection::KeyTooLarge {
            relation,
            ctor,
            index,
            size,
            limit,
        } => RejectReason::KeyTooLarge {
            relation: qualified(schema, relation),
            ctor: schema.ty(schema.relation(relation).elem_def).ctors()[ctor as usize]
                .name
                .clone(),
            index,
            size: i64::try_from(size).unwrap_or(i64::MAX),
            limit: i64::try_from(limit).unwrap_or(i64::MAX),
        },
        Rejection::RowTooLarge {
            relation,
            ctor,
            size,
            limit,
        } => RejectReason::RowTooLarge {
            relation: qualified(schema, relation),
            ctor: schema.ty(schema.relation(relation).elem_def).ctors()[ctor as usize]
                .name
                .clone(),
            size: i64::try_from(size).unwrap_or(i64::MAX),
            limit: i64::try_from(limit).unwrap_or(i64::MAX),
        },
    }
}

fn qualified(schema: &Schema, rel: RelId) -> String {
    let r = schema.relation(rel);
    format!("{}::{}", schema.module(r.module).name, r.name)
}

fn print_key(schema: &Schema, rel: RelId, key: &Val) -> Result<String, PrintError> {
    let form = Form::of(schema, &Ty::Key { rel, ctors: None });
    print(key, &form)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broken_checker_guarantee_is_rejected_as_internal() {
        let schema = catalog::engine_schema();
        let got = Error::of_write(&schema, TxnId(7), write::Error::Internal("a set".into()));
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
    fn a_damaged_page_met_writing_catalog_rows_is_a_storage_failure() {
        let damaged = crate::storage::PageError {
            pid: crate::storage::Pid(3),
            damage: crate::storage::Damage::Unreadable,
        };
        for e in [
            catalog::Error::Page(damaged),
            catalog::Error::Heap(heap::Error::Page(damaged)),
        ] {
            let got = Error::of_catalog_write(e);
            assert!(
                matches!(
                    got,
                    Error::Fatal {
                        reason: FatalReason::Storage { .. }
                    }
                ),
                "{got:?}"
            );
        }
        let got = Error::of_catalog_write(catalog::Error::Heap(heap::Error::TooLarge {
            len: 9000,
            limit: 8000,
        }));
        assert!(
            matches!(
                got,
                Error::Rejected {
                    reason: RejectReason::Internal { .. }
                }
            ),
            "{got:?}"
        );
    }

    #[test]
    fn a_serialization_failure_names_the_callers_transaction() {
        let schema = catalog::engine_schema();
        let f = crate::mvcc::SerializationFailure::new(crate::mvcc::Cause::LaterCommit, Vec::new());
        let got = Error::of_write(&schema, TxnId(7), write::Error::Serialization(f));
        assert!(
            matches!(
                got,
                Error::Retry {
                    reason: RetryReason::Serialization { txn: TxnId(7), .. }
                }
            ),
            "{got:?}"
        );
    }

    #[test]
    fn an_error_displays_one_message_per_line_without_the_sums_syntax() {
        let refused = |operator: &str| {
            Error::rejected(RejectReason::SchemaRefused {
                operator: operator.into(),
                detail: "no type `Int`\nno type `P`".into(),
            })
        };
        assert_eq!("no type `Int`\nno type `P`", refused("").to_string());
        assert_eq!(
            "`add type P`: no type `Int`\n`add type P`: no type `P`",
            refused("add type P").to_string()
        );
        let violated = Error::rejected(RejectReason::Violated {
            constraint: "app::users.unique(email)".into(),
            report: vec![Val::Int(1)],
            redacted: 0,
        });
        assert_eq!(
            "constraint `app::users.unique(email)` is violated by 1 row",
            violated.to_string()
        );
        let limit = Error::rejected(RejectReason::StatementLimit {
            limit: Limit::Time {
                after: Duration::from_millis(1500),
            },
        });
        assert_eq!(
            "the statement exceeded its time limit of 1500ms",
            limit.to_string()
        );
        let retry = Error::Retry {
            reason: RetryReason::Serialization {
                txn: TxnId(7),
                on: String::new(),
            },
        };
        assert_eq!(
            "transaction 7 conflicts with a concurrent transaction; retry",
            retry.to_string()
        );
    }

    #[test]
    fn a_refused_migration_names_its_version_then_each_message() {
        let open = |detail: &str| OpenError::MigrationRefused {
            version: 1,
            reason: Error::rejected(RejectReason::SchemaRefused {
                operator: String::new(),
                detail: detail.into(),
            }),
        };
        assert_eq!(
            "migration 1 is refused: no type `Int`",
            open("no type `Int`").to_string()
        );
        assert_eq!(
            "migration 1 is refused:\n  no type `Int`\n  no type `P`",
            open("no type `Int`\nno type `P`").to_string()
        );
    }

    #[test]
    fn an_error_value_prints_as_the_prelude_sum_on_one_line() {
        let refused = Error::rejected(RejectReason::SchemaRefused {
            operator: String::new(),
            detail: "no \"users\"\nno `ids`".into(),
        });
        assert_eq!(
            r#"Rejected { reason: SchemaRefused { operator: "", detail: "no \"users\"\nno `ids`" } }"#,
            refused.value().to_string()
        );
        let limit = Error::rejected(RejectReason::StatementLimit {
            limit: Limit::Time {
                after: Duration::from_millis(1500),
            },
        });
        assert_eq!(
            "Rejected { reason: StatementLimit { limit: Time { after: 1500ms } } }",
            limit.value().to_string()
        );
        assert_eq!(
            "Rejected { reason: Cancelled }",
            Error::rejected(RejectReason::Cancelled).value().to_string()
        );
        let old = Error::rejected(RejectReason::TooOld {
            oldest: Position { epoch: 1, lsn: 40 },
        });
        assert_eq!(
            "Rejected { reason: TooOld { oldest: { epoch: 1, lsn: 40 } } }",
            old.value().to_string()
        );
    }
}
