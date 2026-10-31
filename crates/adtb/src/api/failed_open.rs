use crate::storage;

use super::{OpenError, corrupt};

/// A failed `Engine::open`.
pub(super) struct FailedOpen {
    pub(super) error: OpenError,
    /// The open's `Pager::create` took the log directory.
    pub(super) log_claimed: bool,
}

/// Returns the failed open that a failure of `Pager::create` makes.
pub(super) fn failed_create(e: storage::OpenError) -> FailedOpen {
    let (error, log_claimed) = match e {
        storage::OpenError::Exists => (corrupt("neither copy of the file header verifies"), false),
        storage::OpenError::LogPresent => (
            corrupt("the log directory holds a log that no file here wrote"),
            false,
        ),
        storage::OpenError::Unformatted(page) => (corrupt(page), true),
        other => (corrupt(other), false),
    };
    FailedOpen { error, log_claimed }
}
