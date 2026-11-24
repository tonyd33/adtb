//! The schema versions a text states for its reads of the past and its file scans.

use adtb_lang::kernel::{Kind, Prim, Terms};

use crate::catalog::VersionHash;

use super::cause::Broken;
use super::error::Error;

/// Refuse a version an `at` block or a file scan of `terms` states without its hash, past the
/// log, or with a hash other than the log's, read from `hash`, the hash of the version the
/// database is at.
pub(super) fn held(terms: &Terms, hash: &VersionHash) -> Result<(), Error> {
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
        let logged = hash
            .at(v.number())
            .map_err(|e| Error::internal(Broken::CatalogError(e)))?;
        let Some(logged) = logged else {
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
