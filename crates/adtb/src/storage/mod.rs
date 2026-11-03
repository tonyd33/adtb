//! Hold the file, pages and log: the pager, allocation, recovery, backup and quarantine.

mod alloc;
mod backup;
mod block;
mod crc;
mod create;
mod driver;
mod evicted;
mod header;
mod log;
mod page;
mod pager;
mod quarantine;
mod record;
mod recovery;
mod verify;

#[cfg(test)]
mod tests;

pub use alloc::Reservation;
pub use backup::{
    BackupError, BackupRun, Envelope, FORMAT, MANIFEST, Manifest, PROOF, Part, Proved,
    check_backup, make_scratch, manifest_hash, read_manifest, read_proof, remove_scratch,
    restore_file, scratch_file, withdraw_proof, write_proof,
};
pub use block::{LogFinding, LogProblem};
pub use crc::{Crc32c, crc32c};
pub(crate) use create::{Created, Hold, Unpublished, create};
pub use create::{aside, creating, marker, marker_bytes};
pub(crate) use driver::Driver;
pub use driver::{FATAL_IO, FatalIo};
pub use header::HISTORY_CAPACITY;
pub use page::{
    FIRST_EDITABLE, HEADER_LEN, MAX_PAGE_SIZE, MIN_PAGE_SIZE, PageKind, flags, flip_owner, format,
    header_edit, kind, lsn, owner, seal,
};
pub use pager::{
    Damage, LATCHES_PER_THREAD, OpenError, Options, PageError, PageExclusive, PageOptimistic,
    PageShared, Pager,
};
pub use quarantine::{Holders, Quarantine, Stopped};
pub use record::Edit;
pub(crate) use record::{Malformed, Reader};
pub use recovery::Recovery;
pub use verify::{Finding, RetainedLog, Verification};

/// A page id: the page's index in the data file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pid(pub u64);

impl Lsn {
    /// The `prev` of a full-page image over a damaged page, whose last LSN cannot be known.
    pub const UNCHAINED: Lsn = Lsn(u64::MAX);
}

impl Pid {
    /// The first page id that is not a file header copy.
    pub const FIRST_DATA: Pid = Pid(2);
}

/// The place of a record in the primary's log: the epoch, which rises at each promotion and
/// restore, and the LSN within that epoch's history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    pub epoch: u64,
    pub lsn: Lsn,
}

/// A byte position in the redo log.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Lsn(pub u64);
