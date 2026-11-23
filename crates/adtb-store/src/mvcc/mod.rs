//! Run transactions: snapshots, the commit log, write conflicts and serializable checks.

#[cfg(any(test, feature = "testing"))]
pub mod clog;
#[cfg(not(any(test, feature = "testing")))]
mod clog;
mod conflict;
mod fate;
mod past;
mod reader;
mod snapshot;
mod ssi;
mod txn;
mod vacuuming;
mod wait;
mod watermark;
mod writers;

use crate::storage::PageError;

#[cfg(test)]
mod tests;

pub(crate) use clog::Settling;
pub use clog::{CommitLog, Owes};
pub use conflict::{Probe, Write};
pub use fate::{Fate, Horizon, Rule};
pub use past::{Marker, Past};
pub use reader::{Logged, Reader};
pub use snapshot::Snapshot;
#[cfg(any(test, feature = "testing"))]
pub use ssi::Lock;
pub use ssi::{Access, Limits, Safety, ScanId, SerialId, Serializer, StopsEarly};
#[cfg(test)]
pub use txn::RESERVATION;
pub use txn::{Begun, Committed, Hold, PastError, Pinned, Taken, Transactions};
pub use vacuuming::{Busy, Holder, Vacuuming};
pub use wait::{Gate, Gated, Waits, Watch};
use watermark::SnapshotId;
pub use watermark::{Holds, Scope};
pub use writers::Writers;

/// A transaction id. Ids are assigned in increasing order and never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Xid(pub u64);

impl Xid {
    /// The id the encoding of [`Writers`] holds for a version no transaction deleted.
    pub const NONE: Xid = Xid(0);
    pub const FIRST: Xid = Xid(1);
    /// The ids no `begin` assigns, each free for a caller to give its own meaning.
    pub const SENTINELS: [Xid; 2] = [Xid(u64::MAX - 1), Xid(u64::MAX)];
}

/// What the commit log records for one transaction id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    InProgress = 0,
    Committed = 1,
    Aborted = 2,
}

impl Status {
    /// Returns the status two bits of a status page record, `None` for bits naming none.
    fn from_bits(bits: u8) -> Option<Status> {
        match bits {
            0 => Some(Status::InProgress),
            1 => Some(Status::Committed),
            2 => Some(Status::Aborted),
            _ => None,
        }
    }
}

/// How this process counts a transaction id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    Committed,
    Aborted,
    Running,
    /// Ended with its abort unrecorded: in progress in the commit log, aborted for this process
    /// and settled aborted by the next open.
    Unrecorded,
}

impl From<Status> for Standing {
    /// Returns the standing of an id whose status is `status` and that is running while it is in
    /// progress.
    fn from(status: Status) -> Standing {
        match status {
            Status::Committed => Standing::Committed,
            Status::Aborted => Standing::Aborted,
            Status::InProgress => Standing::Running,
        }
    }
}

impl Standing {
    /// Returns the status readers, writes and pruners count it as: an end left unrecorded as
    /// aborted.
    pub fn counted(self) -> Status {
        match self {
            Standing::Committed => Status::Committed,
            Standing::Aborted | Standing::Unrecorded => Status::Aborted,
            Standing::Running => Status::InProgress,
        }
    }
}

/// Why the commit log gives no status for an id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusError {
    /// A page the lookup reads is damaged.
    Page(PageError),
    /// The id is 0 or at or past the reservation.
    Unreserved(Xid),
}

impl From<PageError> for StatusError {
    fn from(e: PageError) -> StatusError {
        StatusError::Page(e)
    }
}

/// Why the transactions layer refuses a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A commit log page failed verification, the one named, and the database is quarantined.
    Corrupt(PageError),
    /// Every transaction id is spent; no read-write transaction can start.
    Exhausted,
    /// A page of the record of the retention window is damaged.
    History(crate::btree::Error),
    /// The pager is closed.
    Closed,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Corrupt(e) => write!(f, "a commit log page is damaged: {e}"),
            Error::Exhausted => f.write_str("every transaction id is spent"),
            Error::History(e) => write!(f, "the record of the retention window is damaged: {e}"),
            Error::Closed => f.write_str("the engine is closed"),
        }
    }
}

/// A conflict the serializer, the waits or a write check found: the transaction must abort, and
/// may be retried at a new snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    cause: Cause,
    with: Vec<Xid>,
    heap: Option<u32>,
}

impl Conflict {
    fn new(cause: Cause, with: Vec<Xid>, heap: Option<u32>) -> Conflict {
        Conflict { cause, with, heap }
    }

    /// Returns the conflict of a write to heap `heap` of a version whose header changed between
    /// its write check and its exclusive latch, the version's deleters now `with`.
    pub fn changed(heap: u32, with: Vec<Xid>) -> Conflict {
        Conflict::new(Cause::ConcurrentWrite, with, Some(heap))
    }

    pub fn cause(&self) -> Cause {
        self.cause
    }

    /// Returns the other transactions in the conflict that have xids, in order. A read-only
    /// transaction has none and is not named.
    pub fn with(&self) -> &[Xid] {
        &self.with
    }

    /// Returns the heap of the version or entry the caller's read or write on the conflicting
    /// edge is of: `None` for a wait at a gate or a conflict on a schema version, which no heap
    /// holds.
    pub fn heap(&self) -> Option<u32> {
        self.heap
    }
}

/// Why a transaction failed with a serialization failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// A version changed between its write check and acquiring its exclusive latch.
    ConcurrentWrite,
    /// A row or key it writes was written by a transaction that committed after its snapshot.
    LaterCommit,
    /// A rw-antidependency completed a dangerous structure that it was chosen to break.
    Dangerous,
    /// Waiting would have closed a cycle of transactions waiting for each other.
    Deadlock,
}
