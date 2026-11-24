//! The schema versions a text states for its reads of the past and its file scans.

use std::sync::Arc;

use adtb_lang::kernel::{Kind, Prim, Terms};

use crate::catalog::{self, Hash, VersionHash};

use super::error::Error;

/// How a call fails whose read of a version's stored hash failed.
type Unread = Arc<dyn Fn(catalog::Error) -> Error + Send + Sync>;

/// The hash of each version of the log, failing a call whose read of one fails.
#[derive(Clone)]
pub(super) struct Hashes {
    hash: VersionHash,
    unread: Unread,
}

impl Hashes {
    /// Returns `hash`, a call whose read of it fails with `e` failing with `unread(e)`.
    pub(super) fn new(
        hash: VersionHash,
        unread: impl Fn(catalog::Error) -> Error + Send + Sync + 'static,
    ) -> Hashes {
        Hashes {
            hash,
            unread: Arc::new(unread),
        }
    }

    /// Returns the stored hashes, for a caller that maps their failure as the write path does.
    pub(super) fn stored(&self) -> &VersionHash {
        &self.hash
    }

    /// Returns the hash of the version the log is at.
    pub(super) fn get(&self) -> Result<Hash, Error> {
        self.hash.get().map_err(|e| (self.unread)(e))
    }

    /// Returns the hash of version `v`, `None` past the log's.
    pub(super) fn at(&self, v: u64) -> Result<Option<Hash>, Error> {
        self.hash.at(v).map_err(|e| (self.unread)(e))
    }

    pub(super) fn version(&self) -> u64 {
        self.hash.version()
    }

    /// Returns the hashes of the log up to version `v`.
    ///
    /// Preconditions: `v` is at most [`Hashes::version`].
    pub(super) fn upto(&self, v: u64) -> Hashes {
        Hashes {
            hash: self.hash.upto(v),
            unread: self.unread.clone(),
        }
    }
}

/// Refuse a version an `at` block or a file scan of `terms` states without its hash, past the
/// log, or with a hash other than the log's, read from `hash`, the hash of the version the
/// database is at.
pub(super) fn held(terms: &Terms, hash: &Hashes) -> Result<(), Error> {
    for node in terms.iter() {
        let (what, v) = match &node.kind {
            Kind::At { pins: Some(v), .. } => ("an `at` block", v),
            Kind::Prim(Prim::ReadFile { pins: Some(v), .. }, _) => ("a file scan", v),
            _ => continue,
        };
        let Some(stated) = v.hash() else {
            return Err(Error::invalid(format!(
                "{what} states its pinned version's hash"
            )));
        };
        let Some(logged) = hash.at(v.number())? else {
            return Err(unheld(what, v.number(), hash.version()));
        };
        if logged != *stated {
            return Err(differs(what, v.number(), &hex(stated), &hex(&logged)));
        }
    }
    Ok(())
}

/// Returns the refusal of `what` pinning version `v` with hash `stated`, which the log holds with
/// hash `logged`.
pub(super) fn differs(what: &str, v: u64, stated: &str, logged: &str) -> Error {
    Error::invalid(format!(
        "{what} pins version {v} with hash {stated}, and the log's version {v} has hash {logged}"
    ))
}

/// Returns the refusal of `what` pinning version `v`, past the log's version `at`.
pub(super) fn unheld(what: &str, v: u64, at: u64) -> Error {
    Error::invalid(format!(
        "{what} pins version {v}, which the log does not hold: it is at version {at}"
    ))
}

/// Returns `bytes` as lowercase hex digits.
pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
