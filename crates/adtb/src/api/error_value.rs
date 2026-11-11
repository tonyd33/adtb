use std::time::Duration;

use adtb_lang::schema::{Schema, TypeDefKind};
use adtb_lang::types::{Base, Ty};

use crate::exec::Val;

use super::error::{Error, FatalReason, Fault, Limit, RejectReason, RetryReason};
use super::{Position, TxnId};

/// What keeps an error from being a value of the prelude sum `Error`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum NoValue {
    /// The report of a `Violated` error, which has no encoding as `Report` bytes.
    Report,
}

/// A field of an error's value, or the value itself.
enum Part<'a> {
    Text(&'a str),
    Int(i64),
    Bool(bool),
    Duration(Duration),
    Txn(TxnId),
    Position(Position),
    Fault(Fault),
    Report,
    Ctor(&'static str, Vec<(&'static str, Part<'a>)>),
}

impl Error {
    /// Returns the error as its value of the prelude sum `Error` of `schema`.
    ///
    /// Preconditions: the prelude of `schema` declares every constructor and field of the error.
    pub(super) fn prelude_value(&self, schema: &Schema) -> Result<Val, NoValue> {
        let ty = Ty::Data {
            def: schema.prelude_type("Error"),
            args: Vec::new(),
            ctors: None,
        };
        value(schema, self.part(), &ty)
    }

    fn part(&self) -> Part<'_> {
        match self {
            Error::Retry { reason } => Part::Ctor("Retry", vec![("reason", reason.part())]),
            Error::Rejected { reason } => Part::Ctor("Rejected", vec![("reason", reason.part())]),
            Error::Fatal { reason } => Part::Ctor("Fatal", vec![("reason", reason.part())]),
        }
    }
}

impl RetryReason {
    fn part(&self) -> Part<'_> {
        use Part::{Ctor, Duration as Dur, Text, Txn};
        match self {
            RetryReason::Serialization { txn, on } => {
                Ctor("Serialization", vec![("txn", Txn(*txn)), ("on", Text(on))])
            }
            RetryReason::SafePointTimeout { waited } => {
                Ctor("SafePointTimeout", vec![("waited", Dur(*waited))])
            }
            RetryReason::Overloaded { retry_after } => {
                Ctor("Overloaded", vec![("retry_after", Dur(*retry_after))])
            }
            RetryReason::StreamIdle { limit } => Ctor("StreamIdle", vec![("limit", Dur(*limit))]),
            RetryReason::FileChanged { path } => Ctor("FileChanged", vec![("path", Text(path))]),
            RetryReason::Fenced => Ctor("Fenced", Vec::new()),
            RetryReason::ReplayConflict { waited } => {
                Ctor("ReplayConflict", vec![("waited", Dur(*waited))])
            }
        }
    }
}

impl RejectReason {
    fn part(&self) -> Part<'_> {
        use Part::{Ctor, Int, Text};
        use RejectReason as R;
        match self {
            R::Violated {
                constraint,
                redacted,
                ..
            } => Ctor(
                "Violated",
                vec![
                    ("constraint", Text(constraint)),
                    ("report", Part::Report),
                    ("redacted", Int(*redacted)),
                ],
            ),
            R::KeyViolation { relation, key } => Ctor(
                "KeyViolation",
                vec![("relation", Text(relation)), ("key", Text(key))],
            ),
            R::KeyChanged { relation } => Ctor("KeyChanged", vec![("relation", Text(relation))]),
            R::NotOpen { ctor, stage } => Ctor(
                "NotOpen",
                vec![("ctor", Text(ctor)), ("stage", Text(stage))],
            ),
            R::UserError { message } => Ctor("UserError", vec![("message", Text(message))]),
            R::Faulted { fault } => Ctor("Faulted", vec![("fault", Part::Fault(*fault))]),
            R::Denied { detail } => Ctor("Denied", vec![("detail", Text(detail))]),
            R::Invalid { detail } => Ctor("Invalid", vec![("detail", Text(detail))]),
            R::SchemaRefused { operator, detail } => Ctor(
                "SchemaRefused",
                vec![("operator", Text(operator)), ("detail", Text(detail))],
            ),
            R::Untranslatable { query, detail } => Ctor(
                "Untranslatable",
                vec![("query", Text(query)), ("detail", Text(detail))],
            ),
            R::BadInput {
                source,
                line,
                detail,
            } => Ctor(
                "BadInput",
                vec![
                    ("source", Text(source)),
                    ("line", Int(*line)),
                    ("detail", Text(detail)),
                ],
            ),
            R::SchemaMismatch { detail } => Ctor("SchemaMismatch", vec![("detail", Text(detail))]),
            R::StatementLimit { limit } => Ctor("StatementLimit", vec![("limit", limit.part())]),
            R::TransactionExpired { limit, idle } => Ctor(
                "TransactionExpired",
                vec![
                    ("limit", Part::Duration(*limit)),
                    ("idle", Part::Bool(*idle)),
                ],
            ),
            R::TooOld { oldest } => Ctor("TooOld", vec![("oldest", Part::Position(*oldest))]),
            R::HistoryUnavailable { oldest } => Ctor(
                "HistoryUnavailable",
                vec![("oldest", Part::Position(*oldest))],
            ),
            R::PositionNotOnHistory { position } => Ctor(
                "PositionNotOnHistory",
                vec![("position", Part::Position(*position))],
            ),
            R::RestorePointUnavailable { oldest, newest } => Ctor(
                "RestorePointUnavailable",
                vec![
                    ("oldest", Part::Position(*oldest)),
                    ("newest", Part::Position(*newest)),
                ],
            ),
            R::RowTooLarge {
                relation,
                ctor,
                size,
                limit,
            } => Ctor(
                "RowTooLarge",
                vec![
                    ("relation", Text(relation)),
                    ("ctor", Text(ctor)),
                    ("size", Int(*size)),
                    ("limit", Int(*limit)),
                ],
            ),
            R::KeyTooLarge {
                relation,
                ctor,
                index,
                size,
                limit,
            } => Ctor(
                "KeyTooLarge",
                vec![
                    ("relation", Text(relation)),
                    ("ctor", Text(ctor)),
                    ("index", Text(index)),
                    ("size", Int(*size)),
                    ("limit", Int(*limit)),
                ],
            ),
            R::UnknownBundle { .. }
            | R::VersionMismatch { .. }
            | R::CatalogMismatch { .. }
            | R::Retired { .. }
            | R::BundleExpired { .. }
            | R::BundleStale { .. }
            | R::BundleSuperseded { .. }
            | R::DowngradeRefused { .. }
            | R::WrongVersion { .. }
            | R::ReadOnlyReplica { .. }
            | R::UnsupportedProtocol { .. }
            | R::UnknownDatabase { .. }
            | R::UnknownEdition { .. }
            | R::DumpDamaged { .. }
            | R::BackupDamaged { .. }
            | R::Internal { .. } => self.named_part(),
            R::Cancelled
            | R::Terminated
            | R::DeadlineExceeded
            | R::TransactionBusy
            | R::MixedVersions
            | R::UnknownQuery
            | R::UnknownTransaction
            | R::RequestExpired
            | R::RequestReused
            | R::AuthFailed
            | R::EditionLocked
            | R::MigrationInProgress
            | R::Closed => Ctor(self.unit_name(), Vec::new()),
        }
    }

    fn named_part(&self) -> Part<'_> {
        use Part::{Ctor, Int, Text};
        use RejectReason as R;
        match self {
            R::UnknownBundle { bundle } => Ctor("UnknownBundle", vec![("bundle", Text(bundle))]),
            R::VersionMismatch { bundle, version } => Ctor(
                "VersionMismatch",
                vec![("bundle", Text(bundle)), ("version", Int(*version))],
            ),
            R::CatalogMismatch { bundle, catalog } => Ctor(
                "CatalogMismatch",
                vec![("bundle", Text(bundle)), ("catalog", Int(*catalog))],
            ),
            R::Retired { bundle } => Ctor("Retired", vec![("bundle", Text(bundle))]),
            R::BundleExpired { bundle, query } => Ctor(
                "BundleExpired",
                vec![("bundle", Text(bundle)), ("query", Text(query))],
            ),
            R::BundleStale { bundle, ctor } => Ctor(
                "BundleStale",
                vec![("bundle", Text(bundle)), ("ctor", Text(ctor))],
            ),
            R::BundleSuperseded { bundle } => {
                Ctor("BundleSuperseded", vec![("bundle", Text(bundle))])
            }
            R::DowngradeRefused { cause } => Ctor("DowngradeRefused", vec![("cause", Text(cause))]),
            R::WrongVersion { version } => Ctor("WrongVersion", vec![("version", Int(*version))]),
            R::ReadOnlyReplica { primary } => {
                Ctor("ReadOnlyReplica", vec![("primary", Text(primary))])
            }
            R::UnsupportedProtocol { min, max } => Ctor(
                "UnsupportedProtocol",
                vec![("min", Text(min)), ("max", Text(max))],
            ),
            R::UnknownDatabase { database } => {
                Ctor("UnknownDatabase", vec![("database", Text(database))])
            }
            R::UnknownEdition { edition } => {
                Ctor("UnknownEdition", vec![("edition", Text(edition))])
            }
            R::DumpDamaged { part } => Ctor("DumpDamaged", vec![("part", Text(part))]),
            R::BackupDamaged { part } => Ctor("BackupDamaged", vec![("part", Text(part))]),
            R::Internal { what } => Ctor("Internal", vec![("what", Text(what))]),
            other => {
                unreachable!("`part` sends only a reason of one text or number here, not {other:?}")
            }
        }
    }

    fn unit_name(&self) -> &'static str {
        use RejectReason as R;
        match self {
            R::Cancelled => "Cancelled",
            R::Terminated => "Terminated",
            R::DeadlineExceeded => "DeadlineExceeded",
            R::TransactionBusy => "TransactionBusy",
            R::MixedVersions => "MixedVersions",
            R::UnknownQuery => "UnknownQuery",
            R::UnknownTransaction => "UnknownTransaction",
            R::RequestExpired => "RequestExpired",
            R::RequestReused => "RequestReused",
            R::AuthFailed => "AuthFailed",
            R::EditionLocked => "EditionLocked",
            R::MigrationInProgress => "MigrationInProgress",
            R::Closed => "Closed",
            other => unreachable!("`part` sends only a reason without fields here, not {other:?}"),
        }
    }
}

impl Limit {
    fn part(&self) -> Part<'_> {
        match self {
            Limit::Time { after } => Part::Ctor("Time", vec![("after", Part::Duration(*after))]),
            Limit::Memory { bytes } => Part::Ctor("Memory", vec![("bytes", Part::Int(*bytes))]),
            Limit::Rounds { rounds } => Part::Ctor("Rounds", vec![("rounds", Part::Int(*rounds))]),
        }
    }
}

impl FatalReason {
    fn part(&self) -> Part<'_> {
        use Part::{Ctor, Text};
        match self {
            FatalReason::Malformed { detail } => Ctor("Malformed", vec![("detail", Text(detail))]),
            FatalReason::Quarantined { what } => Ctor("Quarantined", vec![("what", Text(what))]),
            FatalReason::MissingFunction { function } => {
                Ctor("MissingFunction", vec![("function", Text(function))])
            }
            FatalReason::Storage { detail } => Ctor("Storage", vec![("detail", Text(detail))]),
        }
    }
}

fn value(schema: &Schema, part: Part<'_>, ty: &Ty) -> Result<Val, NoValue> {
    match (part, ty) {
        (Part::Text(t), Ty::Base(Base::Text)) => Ok(Val::text(t)),
        (Part::Int(n), Ty::Base(Base::I64)) => Ok(Val::Int(n)),
        (Part::Duration(d), Ty::Base(Base::Duration)) => Ok(Val::Duration(
            i64::try_from(d.as_micros()).unwrap_or(i64::MAX),
        )),
        (Part::Report, _) => Err(NoValue::Report),
        (part, Ty::Data { def, .. }) => match (part, &schema.ty(*def).kind) {
            (Part::Txn(t), TypeDefKind::Newtype { base, .. }) => {
                value(schema, Part::Int(t.0), base)
            }
            (Part::Bool(b), _) => ctor(schema, ty, if b { "True" } else { "False" }, Vec::new()),
            (Part::Fault(f), _) => ctor(schema, ty, f.name(), Vec::new()),
            (Part::Position(p), _) => ctor(
                schema,
                ty,
                "Position",
                vec![
                    (
                        "epoch",
                        Part::Int(i64::try_from(p.epoch).unwrap_or(i64::MAX)),
                    ),
                    ("lsn", Part::Int(i64::try_from(p.lsn).unwrap_or(i64::MAX))),
                ],
            ),
            (Part::Ctor(name, fields), _) => ctor(schema, ty, name, fields),
            (_, kind) => panic!("the prelude's error sums declare each error field, not {kind:?}"),
        },
        (_, ty) => panic!("the prelude's error sums declare each error field, not one of {ty:?}"),
    }
}

/// Returns the value of constructor `name` of the sum `ty` with `fields`, each placed where the
/// constructor declares it.
fn ctor(
    schema: &Schema,
    ty: &Ty,
    name: &str,
    mut fields: Vec<(&str, Part<'_>)>,
) -> Result<Val, NoValue> {
    let undeclared = || -> ! {
        panic!("the prelude's error sums declare constructor {name} with the error's fields")
    };
    let Ty::Data { def, .. } = ty else {
        undeclared()
    };
    let def = schema.ty(*def);
    let TypeDefKind::Sum { header, ctors, .. } = &def.kind else {
        undeclared()
    };
    let Some(index) = def.ctor_index(name) else {
        undeclared()
    };
    let declared: Vec<_> = header.iter().chain(&ctors[index as usize].fields).collect();
    if declared.len() != fields.len() {
        undeclared()
    }
    let values = declared
        .iter()
        .map(|field| {
            let Some(at) = fields.iter().position(|(f, _)| *f == field.name) else {
                undeclared()
            };
            value(schema, fields.swap_remove(at).1, &field.ty)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Val::ctor(index, values))
}
