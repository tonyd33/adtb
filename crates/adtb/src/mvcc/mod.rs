mod clog;
mod conflict;
mod latch;
mod snapshot;
mod ssi;
mod txn;
mod wait;
mod watermark;

#[cfg(test)]
mod tests;

pub use clog::{CommitLog, PAGE_FORMAT};
pub use conflict::{Probe, Write, check_write, probe_unique, read_before};
pub use snapshot::Snapshot;
pub use ssi::{Access, Limits, Safety, SerialId, Serializer};
pub use txn::{RESERVATION, Transactions};
pub use wait::Waits;
pub use watermark::{Holds, Scope, SnapshotId};

/// A transaction id. Ids are assigned in increasing order and never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Xid(pub u64);

impl Xid {
    /// The id a version header holds in `xmax` when no transaction deleted the version.
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

/// The transaction must abort, and may be retried at a new snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SerializationFailure;
