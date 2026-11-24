//! The migrations an open applies beyond the file's own.

use std::collections::BTreeSet;

use adtb_lang::check::Compiled;
use adtb_lang::evolve::Privilege;
use adtb_lang::norm::{self, NormalProgram};

use crate::catalog::{self, Catalog};
use crate::faults::{self, Step};
use crate::mvcc::Transactions;
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::cause::{Broken, corrupt, internal};
use super::{Error, FatalReason, Log, OpenError, Opened, RejectReason, TxnId, registered};

/// An open of `pager` past its migrations, each durable once it committed. A later step of the
/// open runs through [`Migrated::then`] and fails the open with its own failure, every migration
/// still applied.
pub(super) struct Migrated<'p, R: Runtime> {
    pager: &'p Pager<R>,
}

impl<'p, R: Runtime> Migrated<'p, R> {
    /// Returns the token of an open of `pager` that applies no migration to a file it does not
    /// write.
    pub(super) fn unwritten(pager: &'p Pager<R>) -> Migrated<'p, R> {
        Migrated { pager }
    }

    /// Run `run`, a step of the open after its migrations. Returns its value, or fails with its
    /// own failure.
    #[track_caller]
    pub(super) fn then<T>(
        &self,
        run: impl FnOnce() -> Result<T, OpenError>,
    ) -> Result<T, OpenError> {
        let root = self.pager.catalog_root();
        match faults::damaged(Step::Opening, self.pager, root) {
            Some(e) => Err(corrupt(catalog::Error::from(e))),
            None => run(),
        }
    }
}

/// Apply the migrations of `log` beyond the file's own, each as its own schema change, stopping
/// at the first refused one or the first across which a bundle of `live` has no translation.
pub(super) fn migrate<'p, R: Runtime>(
    catalog: &mut Catalog,
    (pager, txns): (&'p Pager<R>, &Transactions<R>),
    log: &Log,
    live: &[registered::Live],
    replay: Option<&BTreeSet<(String, Privilege, String)>>,
) -> Result<Migrated<'p, R>, OpenError> {
    let stored = catalog.source().log().len();
    let deployed = catalog.committed_deployed(pager, txns).map_err(corrupt)?;
    for text in log.migrations.iter().skip(stored) {
        let version = catalog.source().version() + 1;
        registered::recheck(catalog, text, live, &deployed)
            .map_err(|reason| refused(version, reason))?;
        let applied = if let Some(held) = replay {
            catalog.replay_holding((pager, txns), text, held).map(drop)
        } else {
            catalog.migrate(pager, txns, text)
        };
        applied.map_err(|e| match e {
            catalog::Error::Refused(why) => refused(
                version,
                RejectReason::SchemaRefused {
                    operator: String::new(),
                    detail: why.join("\n"),
                },
            ),
            catalog::Error::RefusedBy { operator, why } => refused(
                version,
                RejectReason::SchemaRefused {
                    operator,
                    detail: why.join("\n"),
                },
            ),
            catalog::Error::Internal(what) => refused(version, internal(Broken::Catalog(*what))),
            catalog::Error::Quarantined(what) => OpenError::MigrationRefused {
                version,
                reason: Error::Fatal {
                    reason: FatalReason::Quarantined { what },
                },
            },
            catalog::Error::MissingFunction(name) => OpenError::HostFnMissing { name },
            catalog::Error::OtherVersion(name) => OpenError::HostFnMismatch { name },
            catalog::Error::Unchecked(built, ds) => refused(
                version,
                internal(Broken::Unchecked {
                    built,
                    messages: ds.into_iter().map(|d| d.message).collect(),
                }),
            ),
            other => corrupt(other),
        })?;
    }
    Ok(Migrated { pager })
}

/// Returns the version of the first migration of `log` past the file's, `None` when it has none.
pub(super) fn unapplied(catalog: &Catalog, log: &Log) -> Option<u64> {
    (log.migrations.len() > catalog.source().log().len()).then(|| catalog.source().version() + 1)
}

impl<R: Runtime> Opened<R> {
    /// Fail with `MigrationRefused` naming `unapplied`, the first migration the open left
    /// unapplied under the whole-database mark, its reason the failure every call on the database
    /// returns.
    pub(super) fn refusing_unapplied(self, unapplied: Option<u64>) -> Result<Opened<R>, OpenError> {
        let Some(version) = unapplied else {
            return Ok(self);
        };
        let refused = self
            .db
            .mapping()
            .refusal()
            .expect("the whole-database mark an open found holds until the engine closes");
        let schema = self.parts.compiled.committed();
        let reason = Error::of_write(&self.db, schema, TxnId(0), refused);
        Err(OpenError::MigrationRefused { version, reason })
    }
}

/// Returns the normal program of `compiled`, refusing the file when a body expands past this
/// build's limits.
pub(super) fn normal(compiled: &Compiled) -> Result<NormalProgram, OpenError> {
    norm::program(compiled)
        .map_err(|e| corrupt(catalog::not_normal(compiled.schema(), e, stored_not_normal)))
}

fn stored_not_normal(e: norm::Error) -> catalog::Error {
    catalog::Error::Internal(Box::new(catalog::Broken::StoredNotNormal(e)))
}

fn refused(version: u64, reason: RejectReason) -> OpenError {
    OpenError::MigrationRefused {
        version,
        reason: Error::rejected(reason),
    }
}
