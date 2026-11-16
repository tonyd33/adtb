use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use adtb_lang::ids::{CatCtorId, engine_ctor as k};
use adtb_lang::schema::{Prelude, Schema, TypeDefKind};
use adtb_lang::types::{Base, Ty, TypeId};

use crate::catalog::codec::{Field, Fields, Listed, field, listed};
use crate::exec::Val;

use super::cause::Broken;
use super::server::CallOrigin;

use super::error::{Error, FatalReason, Fault, Limit, RejectReason, RetryReason};
use super::{Position, TxnId};

/// What keeps an error from being a value of the prelude sum `Error`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum NoValue {
    /// The report of a `Violated` error, which has no encoding as `Report` bytes.
    Report,
}

/// The fields of each constructor of the prelude sums of errors, call origins and call outcomes,
/// in the order its encoder gives them.
const SHAPES: &[(CatCtorId, Fields)] = &[
    (k::ERROR_RETRY, &["reason"]),
    (k::ERROR_REJECTED, &["reason"]),
    (k::ERROR_FATAL, &["reason"]),
    (k::RETRY_SERIALIZATION, &["txn", "on"]),
    (k::RETRY_SAFE_POINT_TIMEOUT, &["waited"]),
    (k::RETRY_OVERLOADED, &["retry_after"]),
    (k::RETRY_STREAM_IDLE, &["limit"]),
    (k::RETRY_FILE_CHANGED, &["path"]),
    (k::RETRY_REPLAY_CONFLICT, &["waited"]),
    (k::REJECT_VIOLATED, &["constraint", "report", "redacted"]),
    (k::REJECT_KEY_VIOLATION, &["relation", "key"]),
    (k::REJECT_KEY_CHANGED, &["relation"]),
    (k::REJECT_NOT_OPEN, &["ctor", "stage"]),
    (k::REJECT_USER_ERROR, &["message"]),
    (k::REJECT_FAULTED, &["fault"]),
    (k::REJECT_DENIED, &["detail"]),
    (k::REJECT_INVALID, &["detail"]),
    (k::REJECT_SCHEMA_REFUSED, &["operator", "detail"]),
    (k::REJECT_UNTRANSLATABLE, &["query", "detail"]),
    (k::REJECT_BAD_INPUT, &["source", "line", "detail"]),
    (k::REJECT_SCHEMA_MISMATCH, &["detail"]),
    (k::REJECT_STATEMENT_LIMIT, &["limit"]),
    (k::REJECT_TRANSACTION_EXPIRED, &["limit", "idle"]),
    (k::REJECT_TOO_OLD, &["oldest"]),
    (k::REJECT_HISTORY_UNAVAILABLE, &["oldest"]),
    (k::REJECT_POSITION_NOT_ON_HISTORY, &["position"]),
    (k::REJECT_RESTORE_POINT_UNAVAILABLE, &["oldest", "newest"]),
    (
        k::REJECT_ROW_TOO_LARGE,
        &["relation", "ctor", "size", "limit"],
    ),
    (
        k::REJECT_KEY_TOO_LARGE,
        &["relation", "ctor", "index", "size", "limit"],
    ),
    (k::REJECT_UNKNOWN_BUNDLE, &["bundle"]),
    (k::REJECT_VERSION_MISMATCH, &["bundle", "version"]),
    (k::REJECT_CATALOG_MISMATCH, &["bundle", "catalog"]),
    (k::REJECT_RETIRED, &["bundle"]),
    (k::REJECT_BUNDLE_EXPIRED, &["bundle", "query"]),
    (k::REJECT_BUNDLE_STALE, &["bundle", "ctor"]),
    (k::REJECT_BUNDLE_SUPERSEDED, &["bundle"]),
    (k::REJECT_DOWNGRADE_REFUSED, &["cause"]),
    (k::REJECT_WRONG_VERSION, &["version"]),
    (k::REJECT_READ_ONLY_REPLICA, &["primary"]),
    (k::REJECT_UNSUPPORTED_PROTOCOL, &["min", "max"]),
    (k::REJECT_UNKNOWN_DATABASE, &["database"]),
    (k::REJECT_UNKNOWN_EDITION, &["edition"]),
    (k::REJECT_DUMP_DAMAGED, &["part"]),
    (k::REJECT_BACKUP_DAMAGED, &["part"]),
    (k::REJECT_INTERNAL, &["what"]),
    (k::LIMIT_TIME, &["after"]),
    (k::LIMIT_MEMORY, &["bytes"]),
    (k::LIMIT_ROUNDS, &["rounds"]),
    (k::FATAL_MALFORMED, &["detail"]),
    (k::FATAL_QUARANTINED, &["what"]),
    (k::FATAL_MISSING_FUNCTION, &["function"]),
    (k::FATAL_STORAGE, &["detail"]),
    (k::POSITION, &["epoch", "lsn"]),
    (k::ORIGIN_REGISTERED, &["bundle", "query"]),
    (k::ORIGIN_FUNCTION, &["name"]),
    (k::ORIGIN_AD_HOC, &["text"]),
    (k::OUTCOME_FAILED, &["error"]),
    (k::RETRY_FENCED, &[]),
    (k::REJECT_CANCELLED, &[]),
    (k::REJECT_TERMINATED, &[]),
    (k::REJECT_DEADLINE_EXCEEDED, &[]),
    (k::REJECT_TRANSACTION_BUSY, &[]),
    (k::REJECT_MIXED_VERSIONS, &[]),
    (k::REJECT_UNKNOWN_QUERY, &[]),
    (k::REJECT_UNKNOWN_TRANSACTION, &[]),
    (k::REJECT_REQUEST_EXPIRED, &[]),
    (k::REJECT_REQUEST_REUSED, &[]),
    (k::REJECT_AUTH_FAILED, &[]),
    (k::REJECT_EDITION_LOCKED, &[]),
    (k::REJECT_MIGRATION_IN_PROGRESS, &[]),
    (k::REJECT_CLOSED, &[]),
    (k::OUTCOME_SUCCEEDED, &[]),
];

/// Returns the fields [`SHAPES`] lists for constructor `id`. Fails to compile in a constant when
/// it lists none.
const fn shape(id: CatCtorId) -> Fields {
    let mut at = 0;
    while at < SHAPES.len() {
        if SHAPES[at].0.raw() == id.raw() {
            return SHAPES[at].1;
        }
        at += 1;
    }
    panic!("an encoder gives a constructor `SHAPES` lists")
}

/// Returns the part of constructor `id` holding each named field. Fails to compile when `SHAPES`
/// lists no such field of `id`.
macro_rules! ctor {
    ($id:expr $(, $name:literal: $part:expr)* $(,)?) => {
        Part::Ctor($id, vec![$((const { field(shape($id), $name) }, $part)),*])
    };
}

/// The constructors of the prelude sums of errors, call origins and call outcomes, each with
/// where its fields sit in one schema.
pub(super) struct Sums {
    prelude: Prelude,
    ctors: BTreeMap<CatCtorId, Layout>,
}

/// Where a constructor sits in its type, and where each field [`SHAPES`] lists for it sits.
struct Layout {
    def: TypeId,
    index: u32,
    listed: Listed,
}

impl Sums {
    /// Returns the constructors [`SHAPES`] lists as `schema` declares them.
    ///
    /// Preconditions: `schema` declares each constructor `SHAPES` lists with exactly the fields
    /// it lists, in any order, and each sum among them with no other constructor.
    pub(super) fn new(schema: &Schema) -> Sums {
        let prelude = schema.prelude();
        assert!(
            matches!(
                &schema.ty(prelude.txn_id()).kind,
                TypeDefKind::Newtype {
                    base: Ty::Base(Base::I64),
                    ..
                }
            ),
            "the prelude's `TxnId` is a newtype of `I64`"
        );
        let mut ctors = BTreeMap::new();
        let mut listed: BTreeMap<TypeId, usize> = BTreeMap::new();
        for &(id, fields) in SHAPES {
            let c = schema
                .ids()
                .ctor_of(id)
                .unwrap_or_else(|| panic!("the prelude declares constructor {id}"));
            let def = schema.ty(c.def());
            let declared: Vec<(String, Ty)> = def
                .header()
                .iter()
                .chain(&def.ctors()[c.index() as usize].fields)
                .map(|f| (f.name.clone(), f.ty.clone()))
                .collect();
            let layout = Listed::new(fields, &declared, || {
                format!("`SHAPES` lists the fields of constructor {id}")
            });
            *listed.entry(c.def()).or_default() += 1;
            ctors.insert(
                id,
                Layout {
                    def: c.def(),
                    index: c.index(),
                    listed: layout,
                },
            );
        }
        for (def, n) in listed {
            assert_eq!(
                schema.ty(def).ctors().len(),
                n,
                "`SHAPES` lists each constructor of {}",
                schema.ty(def).name
            );
        }
        Sums { prelude, ctors }
    }

    /// Returns the value of `origin` of the prelude sum `Origin`.
    pub(super) fn origin(&self, origin: &CallOrigin) -> Val {
        let part = match origin {
            CallOrigin::Registered { bundle, query } => ctor!(
                k::ORIGIN_REGISTERED,
                "bundle": Part::Text(bundle),
                "query": Part::Int(i64::from(*query)),
            ),
            CallOrigin::Function { name } => ctor!(k::ORIGIN_FUNCTION, "name": Part::Text(name)),
            CallOrigin::AdHoc(text) => ctor!(k::ORIGIN_AD_HOC, "text": Part::Bytes(text)),
        };
        self.whole(part, self.prelude.origin())
    }

    /// Returns the value of the prelude sum `Outcome` of a call that failed with `failed`, or
    /// succeeded. A failure that holds a report is recorded as `Internal` naming it.
    pub(super) fn outcome(&self, failed: Option<&Error>) -> Val {
        let Some(error) = failed else {
            return self.whole(ctor!(k::OUTCOME_SUCCEEDED), self.prelude.outcome());
        };
        let internal;
        let error = match error.prelude_value(self) {
            Ok(_) => error,
            Err(NoValue::Report) => {
                internal = Error::internal(Broken::NoValue(error.clone()));
                &internal
            }
        };
        let failed = ctor!(k::OUTCOME_FAILED, "error": error.part());
        self.whole(failed, self.prelude.outcome())
    }

    fn whole(&self, part: Part<'_>, def: TypeId) -> Val {
        match self.part(part, &data(def)) {
            Ok(v) => v,
            Err(NoValue::Report) => unreachable!("only a `Violated` error holds a report"),
        }
    }
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
    Bytes(&'a [u8]),
    Report,
    Ctor(CatCtorId, Vec<(Field, Part<'a>)>),
}

impl Error {
    /// Returns the error as its value of the prelude sum `Error` of `sums`.
    pub(super) fn prelude_value(&self, sums: &Sums) -> Result<Val, NoValue> {
        sums.part(self.part(), &data(sums.prelude.error()))
    }

    fn part(&self) -> Part<'_> {
        match self {
            Error::Retry { reason } => ctor!(k::ERROR_RETRY, "reason": reason.part()),
            Error::Rejected { reason } => {
                ctor!(k::ERROR_REJECTED, "reason": reason.part())
            }
            Error::Fatal { reason } => ctor!(k::ERROR_FATAL, "reason": reason.part()),
        }
    }
}

impl RetryReason {
    fn part(&self) -> Part<'_> {
        use Part::{Duration as Dur, Text, Txn};
        match self {
            RetryReason::Serialization { txn, on } => {
                ctor!(k::RETRY_SERIALIZATION, "txn": Txn(*txn), "on": Text(on))
            }
            RetryReason::SafePointTimeout { waited } => {
                ctor!(k::RETRY_SAFE_POINT_TIMEOUT, "waited": Dur(*waited))
            }
            RetryReason::Overloaded { retry_after } => {
                ctor!(k::RETRY_OVERLOADED, "retry_after": Dur(*retry_after))
            }
            RetryReason::StreamIdle { limit } => {
                ctor!(k::RETRY_STREAM_IDLE, "limit": Dur(*limit))
            }
            RetryReason::FileChanged { path } => {
                ctor!(k::RETRY_FILE_CHANGED, "path": Text(path))
            }
            RetryReason::Fenced => ctor!(k::RETRY_FENCED),
            RetryReason::ReplayConflict { waited } => {
                ctor!(k::RETRY_REPLAY_CONFLICT, "waited": Dur(*waited))
            }
        }
    }
}

impl RejectReason {
    fn part(&self) -> Part<'_> {
        use Part::{Int, Text};
        use RejectReason as R;
        match self {
            R::Violated {
                constraint,
                redacted,
                ..
            } => {
                ctor!(k::REJECT_VIOLATED, "constraint": Text(constraint), "report": Part::Report, "redacted": Int(*redacted))
            }
            R::KeyViolation { relation, key } => {
                ctor!(k::REJECT_KEY_VIOLATION, "relation": Text(relation), "key": Text(key))
            }
            R::KeyChanged { relation } => {
                ctor!(k::REJECT_KEY_CHANGED, "relation": Text(relation))
            }
            R::NotOpen { ctor, stage } => {
                ctor!(k::REJECT_NOT_OPEN, "ctor": Text(ctor), "stage": Text(stage))
            }
            R::UserError { message } => {
                ctor!(k::REJECT_USER_ERROR, "message": Text(message))
            }
            R::Faulted { fault } => ctor!(k::REJECT_FAULTED, "fault": Part::Fault(*fault)),
            R::Denied { detail } => ctor!(k::REJECT_DENIED, "detail": Text(detail)),
            R::Invalid { detail } => ctor!(k::REJECT_INVALID, "detail": Text(detail)),
            R::SchemaRefused { operator, detail } => {
                ctor!(k::REJECT_SCHEMA_REFUSED, "operator": Text(operator), "detail": Text(detail))
            }
            R::Untranslatable { query, detail } => {
                ctor!(k::REJECT_UNTRANSLATABLE, "query": Text(query), "detail": Text(detail))
            }
            R::BadInput {
                source,
                line,
                detail,
            } => {
                ctor!(k::REJECT_BAD_INPUT, "source": Text(source), "line": Int(*line), "detail": Text(detail))
            }
            R::SchemaMismatch { detail } => {
                ctor!(k::REJECT_SCHEMA_MISMATCH, "detail": Text(detail))
            }
            R::StatementLimit { limit } => {
                ctor!(k::REJECT_STATEMENT_LIMIT, "limit": limit.part())
            }
            R::TransactionExpired { limit, idle } => {
                ctor!(k::REJECT_TRANSACTION_EXPIRED, "limit": Part::Duration(*limit), "idle": Part::Bool(*idle))
            }
            R::TooOld { oldest } => {
                ctor!(k::REJECT_TOO_OLD, "oldest": Part::Position(*oldest))
            }
            R::HistoryUnavailable { oldest } => {
                ctor!(k::REJECT_HISTORY_UNAVAILABLE, "oldest": Part::Position(*oldest))
            }
            R::PositionNotOnHistory { position } => {
                ctor!(k::REJECT_POSITION_NOT_ON_HISTORY, "position": Part::Position(*position))
            }
            R::RestorePointUnavailable { oldest, newest } => {
                ctor!(k::REJECT_RESTORE_POINT_UNAVAILABLE, "oldest": Part::Position(*oldest), "newest": Part::Position(*newest))
            }
            R::RowTooLarge {
                relation,
                ctor,
                size,
                limit,
            } => {
                ctor!(k::REJECT_ROW_TOO_LARGE, "relation": Text(relation), "ctor": Text(ctor), "size": Int(*size), "limit": Int(*limit))
            }
            R::KeyTooLarge {
                relation,
                ctor,
                index,
                size,
                limit,
            } => {
                ctor!(k::REJECT_KEY_TOO_LARGE, "relation": Text(relation), "ctor": Text(ctor), "index": Text(index), "size": Int(*size), "limit": Int(*limit))
            }
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
            | R::Closed => Part::Ctor(self.unit_id(), Vec::new()),
        }
    }

    fn named_part(&self) -> Part<'_> {
        use Part::{Int, Text};
        use RejectReason as R;
        match self {
            R::UnknownBundle { bundle } => {
                ctor!(k::REJECT_UNKNOWN_BUNDLE, "bundle": Text(bundle))
            }
            R::VersionMismatch { bundle, version } => {
                ctor!(k::REJECT_VERSION_MISMATCH, "bundle": Text(bundle), "version": Int(*version))
            }
            R::CatalogMismatch { bundle, catalog } => {
                ctor!(k::REJECT_CATALOG_MISMATCH, "bundle": Text(bundle), "catalog": Int(*catalog))
            }
            R::Retired { bundle } => ctor!(k::REJECT_RETIRED, "bundle": Text(bundle)),
            R::BundleExpired { bundle, query } => {
                ctor!(k::REJECT_BUNDLE_EXPIRED, "bundle": Text(bundle), "query": Text(query))
            }
            R::BundleStale { bundle, ctor } => {
                ctor!(k::REJECT_BUNDLE_STALE, "bundle": Text(bundle), "ctor": Text(ctor))
            }
            R::BundleSuperseded { bundle } => {
                ctor!(k::REJECT_BUNDLE_SUPERSEDED, "bundle": Text(bundle))
            }
            R::DowngradeRefused { cause } => {
                ctor!(k::REJECT_DOWNGRADE_REFUSED, "cause": Text(cause))
            }
            R::WrongVersion { version } => {
                ctor!(k::REJECT_WRONG_VERSION, "version": Int(*version))
            }
            R::ReadOnlyReplica { primary } => {
                ctor!(k::REJECT_READ_ONLY_REPLICA, "primary": Text(primary))
            }
            R::UnsupportedProtocol { min, max } => {
                ctor!(k::REJECT_UNSUPPORTED_PROTOCOL, "min": Text(min), "max": Text(max))
            }
            R::UnknownDatabase { database } => {
                ctor!(k::REJECT_UNKNOWN_DATABASE, "database": Text(database))
            }
            R::UnknownEdition { edition } => {
                ctor!(k::REJECT_UNKNOWN_EDITION, "edition": Text(edition))
            }
            R::DumpDamaged { part } => ctor!(k::REJECT_DUMP_DAMAGED, "part": Text(part)),
            R::BackupDamaged { part } => ctor!(k::REJECT_BACKUP_DAMAGED, "part": Text(part)),
            R::Internal { what } => ctor!(k::REJECT_INTERNAL, "what": Text(what)),
            other => {
                unreachable!("`part` sends only a reason of one text or number here, not {other:?}")
            }
        }
    }

    fn unit_id(&self) -> CatCtorId {
        use RejectReason as R;
        match self {
            R::Cancelled => k::REJECT_CANCELLED,
            R::Terminated => k::REJECT_TERMINATED,
            R::DeadlineExceeded => k::REJECT_DEADLINE_EXCEEDED,
            R::TransactionBusy => k::REJECT_TRANSACTION_BUSY,
            R::MixedVersions => k::REJECT_MIXED_VERSIONS,
            R::UnknownQuery => k::REJECT_UNKNOWN_QUERY,
            R::UnknownTransaction => k::REJECT_UNKNOWN_TRANSACTION,
            R::RequestExpired => k::REJECT_REQUEST_EXPIRED,
            R::RequestReused => k::REJECT_REQUEST_REUSED,
            R::AuthFailed => k::REJECT_AUTH_FAILED,
            R::EditionLocked => k::REJECT_EDITION_LOCKED,
            R::MigrationInProgress => k::REJECT_MIGRATION_IN_PROGRESS,
            R::Closed => k::REJECT_CLOSED,
            other => unreachable!("`part` sends only a reason without fields here, not {other:?}"),
        }
    }
}

impl Limit {
    fn part(&self) -> Part<'_> {
        match self {
            Limit::Time { after } => {
                ctor!(k::LIMIT_TIME, "after": Part::Duration(*after))
            }
            Limit::Memory { bytes } => {
                ctor!(k::LIMIT_MEMORY, "bytes": Part::Int(*bytes))
            }
            Limit::Rounds { rounds } => {
                ctor!(k::LIMIT_ROUNDS, "rounds": Part::Int(*rounds))
            }
        }
    }
}

impl FatalReason {
    fn part(&self) -> Part<'_> {
        use Part::Text;
        match self {
            FatalReason::Malformed { detail } => {
                ctor!(k::FATAL_MALFORMED, "detail": Text(detail))
            }
            FatalReason::Quarantined { what } => {
                ctor!(k::FATAL_QUARANTINED, "what": Text(what))
            }
            FatalReason::MissingFunction { function } => {
                ctor!(k::FATAL_MISSING_FUNCTION, "function": Text(function))
            }
            FatalReason::Storage { detail } => {
                ctor!(k::FATAL_STORAGE, "detail": Text(detail))
            }
        }
    }
}

impl Sums {
    /// Returns the value of `part` of type `ty`.
    fn part(&self, part: Part<'_>, ty: &Ty) -> Result<Val, NoValue> {
        let prelude = self.prelude;
        let def = match ty {
            Ty::Data { def, .. } => Some(*def),
            _ => None,
        };
        match (part, ty) {
            (Part::Report, _) => Err(NoValue::Report),
            (Part::Ctor(id, fields), _) => self.of_ctor(id, fields, ty),
            (Part::Text(t), Ty::Base(Base::Text)) => Ok(Val::text(t)),
            (Part::Int(n), Ty::Base(Base::I64)) => Ok(Val::Int(n)),
            (Part::Int(n), Ty::Base(Base::I32)) if i32::try_from(n).is_ok() => Ok(Val::Int(n)),
            (Part::Bytes(b), Ty::Base(Base::Bytes)) => Ok(Val::Bytes(Arc::from(b))),
            (Part::Duration(d), Ty::Base(Base::Duration)) => Ok(Val::Duration(
                i64::try_from(d.as_micros()).unwrap_or(i64::MAX),
            )),
            (Part::Txn(t), _) if def == Some(prelude.txn_id()) => Ok(Val::Int(t.0)),
            (Part::Bool(b), _) if def == Some(prelude.bool()) => Ok(Val::bool(b)),
            (Part::Fault(f), _) if def == Some(prelude.fault()) => Ok(Val::tag(f.ctor())),
            (Part::Position(p), _) => {
                let int = |n| Part::Int(i64::try_from(n).unwrap_or(i64::MAX));
                let fields = ctor!(k::POSITION, "epoch": int(p.epoch), "lsn": int(p.lsn));
                self.part(fields, ty)
            }
            (_, ty) => {
                panic!("the prelude's error sums declare each error field, not one of {ty:?}")
            }
        }
    }

    fn of_ctor(
        &self,
        id: CatCtorId,
        fields: Vec<(Field, Part<'_>)>,
        ty: &Ty,
    ) -> Result<Val, NoValue> {
        let Some(layout) = self.ctors.get(&id) else {
            panic!("`SHAPES` lists constructor {id}")
        };
        assert!(
            matches!(ty, Ty::Data { def, .. } if *def == layout.def),
            "constructor {id} is of type {ty:?}"
        );
        let parts = listed(layout.listed.fields(), fields);
        let values = layout
            .listed
            .handles()
            .zip(parts)
            .map(|(f, part)| self.part(part, layout.listed.ty(f)))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Val::ctor(layout.index, layout.listed.declared(values)))
    }
}

fn data(def: TypeId) -> Ty {
    Ty::Data {
        def,
        args: Vec::new(),
        ctors: None,
    }
}
