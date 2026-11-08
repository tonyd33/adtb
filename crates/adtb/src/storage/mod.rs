//! Hold the file, pages and log: the pager, allocation, recovery, backup, dump and quarantine.

mod alloc;
mod archive;
mod backup;
mod block;
mod checkpoint;
mod crc;
mod create;
mod damage_log;
mod digest;
mod driver;
mod dump;
mod dump_crypt;
mod error;
mod evidence;
mod header;
mod holding;
mod log;
mod lsnmap;
mod page;
mod pager;
mod pagetable;
mod quarantine;
mod record;
mod recovery;
mod verify;

#[cfg(test)]
mod checkpointer_tests;
#[cfg(test)]
mod damage_log_tests;
#[cfg(test)]
mod damage_race_tests;
#[cfg(test)]
mod evidence_tests;
#[cfg(test)]
mod file_damage_tests;
#[cfg(test)]
mod in_session_tests;
#[cfg(test)]
mod incarnation_tests;
#[cfg(test)]
mod pager_tests;
#[cfg(test)]
mod real_file_tests;
#[cfg(test)]
mod recycle_tests;
#[cfg(test)]
mod tests;

pub use alloc::Reservation;
pub use archive::{Archiver, Status as ArchiveStatus, recycle_at_open};
pub use backup::{
    BackupError, BackupRun, Envelope, FORMAT, MANIFEST, Manifest, PROOF, Part, Proved,
    check_backup, make_scratch, manifest_hash, read_manifest, read_proof, remove_scratch,
    restore_file, scratch_file, withdraw_proof, write_proof,
};
pub use block::{LogFinding, LogProblem, Resolutions};
pub use crc::{Crc32c, crc32c};
pub(crate) use create::{Created, Hold, Unpublished, create, refuse_foreign_log, remove_leftover};
pub use create::{aside, creating, marker, marker_bytes};
pub use damage_log::Recorded;
pub(crate) use driver::Driver;
pub use dump::{
    DUMP_FORMAT, DumpManifest, discard_aside, is_dump, publish_database, read_dump, restore_aside,
    write_dump,
};
pub use dump_crypt::Passphrase;
pub use error::{Damage, OpenError};
pub use header::HISTORY_CAPACITY;
pub use page::{
    FIRST_EDITABLE, HEADER_LEN, MAX_PAGE_SIZE, MIN_PAGE_SIZE, PageKind, flags, flip_owner, format,
    header_edit, kind, lsn, owner, seal,
};
pub use pager::{
    ArchiveHook, LATCHES_PER_THREAD, Options, PageError, PageExclusive, PageOptimistic, PageShared,
    Pager,
};
pub use quarantine::{Holders, Quarantine, Stopped};
pub use record::Edit;
pub(crate) use record::{Malformed, Reader};
pub use recovery::Recovery;
pub use verify::{Finding, RetainedLog, Verification};

/// How a paced walk ended, with the number of things it did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Walk {
    Done(usize),
    /// Its pacer said to stop before the walk was done.
    Stopped(usize),
}

impl Walk {
    pub fn count(self) -> usize {
        match self {
            Walk::Done(n) | Walk::Stopped(n) => n,
        }
    }
}

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

impl From<Pid> for u64 {
    fn from(pid: Pid) -> u64 {
        pid.0
    }
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
