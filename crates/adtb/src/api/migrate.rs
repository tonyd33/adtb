//! The migrations an open applies beyond the file's own.

use std::collections::BTreeSet;

use crate::catalog::{self, Catalog};
use crate::mvcc::Transactions;
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::cause::{Broken, corrupt, internal};
use super::{Error, Log, OpenError, Opened, RejectReason, registered};

/// Apply the migrations of `log` beyond the file's own, each as its own schema change, stopping
/// at the first refused one or the first across which a bundle of `live` has no translation.
pub(super) fn migrate<R: Runtime>(
    catalog: &mut Catalog,
    (pager, txns): (&Pager<R>, &Transactions<R>),
    log: &Log,
    live: &[registered::Live],
    replay: Option<&BTreeSet<(String, String, String)>>,
) -> Result<(), OpenError> {
    let stored = catalog.source().log.len();
    let deployed = catalog.committed_deployed(pager, txns).map_err(corrupt)?;
    for text in log.migrations.iter().skip(stored) {
        let version = catalog.source().version + 1;
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
    Ok(())
}

/// Returns the version of the first migration of `log` past the file's, `None` when it has none.
pub(super) fn unapplied(catalog: &Catalog, log: &Log) -> Option<u64> {
    (log.migrations.len() > catalog.source().log.len()).then(|| catalog.source().version + 1)
}

impl<R: Runtime> Opened<R> {
    /// Fail with `MigrationRefused` naming `unapplied`, the first migration the open left
    /// unapplied under the whole-database mark, its reason the failure every call on the database
    /// returns.
    pub(super) fn refusing_unapplied(self, unapplied: Option<u64>) -> Result<Opened<R>, OpenError> {
        let Some(version) = unapplied else {
            return Ok(self);
        };
        let reason = Error::of_whole_mark(&self.db, &self.parts.schema)
            .expect("the whole-database mark an open found holds until the engine closes");
        Err(OpenError::MigrationRefused { version, reason })
    }
}

fn refused(version: u64, reason: RejectReason) -> OpenError {
    OpenError::MigrationRefused {
        version,
        reason: Error::rejected(reason),
    }
}
