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

use adtb_lang::types::RelId;

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
pub use txn::{Begun, DurableCommit, Hold, PastError, Pinned, Taken, Transactions};
pub use vacuuming::{Busy, Holder, Vacuuming};
pub use wait::{Waits, Watch};
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
}

/// What the commit log records for one transaction id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    InProgress = 0,
    Committed = 1,
    Aborted = 2,
}

impl Status {
    fn from_bits(bits: u8) -> Status {
        match bits {
            0 => Status::InProgress,
            1 => Status::Committed,
            2 => Status::Aborted,
            _ => panic!("commit log status bits {bits:#b} name no status"),
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

/// The transaction must abort, and may be retried at a new snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerializationFailure {
    pub cause: Cause,
    /// The other transactions in the conflict that have xids, in order. A read-only transaction
    /// has none and is not named.
    pub with: Vec<Xid>,
    on: Option<(RelId, u32)>,
}

impl SerializationFailure {
    pub fn new(cause: Cause, with: Vec<Xid>) -> SerializationFailure {
        SerializationFailure {
            cause,
            with,
            on: None,
        }
    }

    /// Returns the relation and constructor position of the caller's read or write on the
    /// conflicting edge, `None` when not known.
    pub fn on(&self) -> Option<(RelId, u32)> {
        self.on
    }

    /// Returns this failure on constructor `ctor` of `rel`, unless it names one already.
    #[must_use]
    pub fn at(self, rel: RelId, ctor: u32) -> SerializationFailure {
        SerializationFailure {
            on: self.on.or(Some((rel, ctor))),
            ..self
        }
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
