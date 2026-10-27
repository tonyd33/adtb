mod alloc;
mod block;
mod crc;
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
pub use block::{LogFinding, LogProblem};
pub use crc::{Crc32c, crc32c};
pub(crate) use driver::Driver;
pub use page::{
    FIRST_EDITABLE, HEADER_LEN, MAX_PAGE_SIZE, MIN_PAGE_SIZE, PageKind, flags, format, header_edit,
    kind, lsn, owner,
};
pub use pager::{
    Damage, LATCHES_PER_THREAD, OpenError, Options, PageError, PageExclusive, PageOptimistic,
    PageShared, Pager,
};
pub use quarantine::Quarantine;
pub use record::Edit;
pub(crate) use record::{Malformed, Reader};
pub use recovery::Recovery;
pub use verify::{Finding, Verification};

/// A page id: the page's index in the data file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pid(pub u64);

impl Pid {
    /// The first page id that is not a file header copy.
    pub const FIRST_DATA: Pid = Pid(2);
}

/// The place of a record in the primary's log: the epoch, which counts promotions, and the LSN
/// within that epoch's history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    pub epoch: u32,
    pub lsn: Lsn,
}

/// A byte position in the redo log.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Lsn(pub u64);
