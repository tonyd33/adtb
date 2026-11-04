//! The migrations an open applies beyond the file's own.

use std::collections::BTreeSet;

use crate::catalog::{self, Catalog};
use crate::mvcc::Transactions;
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::cause::{Broken, corrupt, internal};
use super::{Error, Log, OpenError, RejectReason, registered};

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

fn refused(version: u64, reason: RejectReason) -> OpenError {
    OpenError::MigrationRefused {
        version,
        reason: Error::rejected(reason),
    }
}
