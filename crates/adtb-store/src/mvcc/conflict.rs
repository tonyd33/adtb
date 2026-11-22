use super::{Cause, Conflict, Xid};

/// Returns the conflict of a write of a version of heap `heap` that `x` wrote and committed
/// after the writer's snapshot.
pub(super) fn later(x: Xid, heap: u32) -> Conflict {
    Conflict::new(Cause::LaterCommit, vec![x], Some(heap))
}

/// Returns the conflict of a transaction whose wait for others in `cycle` would close it, its
/// write of a version of `heap`, or of none for a wait at a gate.
pub(super) fn deadlock(cycle: Vec<Xid>, heap: Option<u32>) -> Conflict {
    Conflict::new(Cause::Deadlock, cycle, heap)
}

/// Returns the conflict of a transaction chosen to break a dangerous structure among `with`,
/// its read or write on the new edge of `heap`.
pub(super) fn dangerous(with: Vec<Xid>, heap: Option<u32>) -> Conflict {
    Conflict::new(Cause::Dangerous, with, heap)
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
    Fail(Conflict),
}

/// What a write does about the version it updates, deletes or retags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Write {
    Proceed,
    /// Wait for the transaction to end, then check the version again.
    Wait(Xid),
    Fail(Conflict),
}
