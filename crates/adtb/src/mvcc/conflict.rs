use super::{Cause, SerializationFailure, Xid};

pub(super) fn later(x: Xid) -> SerializationFailure {
    SerializationFailure::new(Cause::LaterCommit, vec![x])
}

/// What a uniqueness probe does about one version it found under the probed key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Probe {
    /// The version does not hold the key in the latest committed state.
    Skip,
    /// The version holds the key: the insert is a key violation.
    Conflict,
    /// Wait for the transaction to end, then probe the version again.
    Wait(Xid),
    Fail(SerializationFailure),
}

/// What a write does about the version it updates, deletes or retags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Write {
    Proceed,
    /// Wait for the transaction to end, then check the version again.
    Wait(Xid),
    Fail(SerializationFailure),
}
