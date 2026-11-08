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
mod archive_tests;
#[cfg(test)]
mod backup_tests;
#[cfg(test)]
mod checkpointer_tests;
#[cfg(test)]
mod clog_lost_tests;
#[cfg(test)]
mod commit_tests;
#[cfg(test)]
mod damage_log_tests;
#[cfg(test)]
mod damage_race_tests;
#[cfg(test)]
mod dump_quarantine_tests;
#[cfg(test)]
mod dump_tests;
#[cfg(test)]
mod evidence_tests;
#[cfg(test)]
mod faults_tests;
#[cfg(test)]
mod file_damage_tests;
#[cfg(test)]
mod in_session_tests;
#[cfg(test)]
mod incarnation_tests;
#[cfg(test)]
mod lost_bitmap_tests;
#[cfg(test)]
mod open_damage_tests;
#[cfg(test)]
mod page_past_log_end_tests;
#[cfg(test)]
mod pager_tests;
#[cfg(test)]
mod quarantine_tests;
#[cfg(test)]
mod real_file_tests;
#[cfg(test)]
mod recycle_tests;
#[cfg(test)]
mod tests;

pub use alloc::Reservation;
#[cfg(any(test, feature = "testing"))]
pub use archive::Status as ArchiveStatus;
pub use archive::{Archiver, recycle_at_open};
pub use backup::{
    BackupError, BackupRun, Envelope, Manifest, Proved, check_backup, make_scratch, manifest_hash,
    read_manifest, read_proof, remove_scratch, restore_file, scratch_file, withdraw_proof,
    write_proof,
};
#[cfg(test)]
pub use backup::{MANIFEST, PROOF, Part};
pub use block::{LogFinding, LogProblem, Resolutions};
#[cfg(any(test, feature = "testing"))]
pub use crc::Crc32c;
pub use create::creating;
pub(crate) use create::{Created, Hold, Unpublished, create, refuse_foreign_log, remove_leftover};
#[cfg(any(test, feature = "testing"))]
pub use create::{aside, marker, marker_bytes};
pub(crate) use driver::Driver;
pub use dump::{
    DumpManifest, discard_aside, is_dump, publish_database, read_dump, restore_aside, write_dump,
};
pub use dump_crypt::Passphrase;
pub use error::{Damage, OpenError};
#[cfg(test)]
pub use header::HISTORY_CAPACITY;
pub use page::{
    FIRST_EDITABLE, HEADER_LEN, PageKind, flags, format, header_edit, kind, lsn, owner,
};
#[cfg(any(test, feature = "testing"))]
pub use page::{flip_owner, seal};
pub use pager::{
    LATCHES_PER_THREAD, Options, PageError, PageExclusive, PageOptimistic, PageShared, Pager,
};
pub(crate) use quarantine::StaleCopy;
pub use quarantine::{Holders, Quarantine, Stopped};
#[cfg(test)]
pub use record::Edit;
pub(crate) use record::{Malformed, Reader};
#[cfg(any(test, feature = "testing"))]
pub use verify::Verification;
pub use verify::{Finding, RetainedLog};

/// How a paced walk ended, with the number of things it did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Walk {
    Done(usize),
    /// Its pacer said to stop before the walk was done.
    Stopped(usize),
}

impl Walk {
    #[cfg(any(test, feature = "testing"))]
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
