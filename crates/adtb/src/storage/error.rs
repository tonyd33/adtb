//! Why a page or a database cannot be opened or handed out.

use crate::runtime::IoError;

use super::Lsn;
use super::block::LogFinding;
use super::pager::PageError;

/// Why a page cannot be handed out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Damage {
    /// The page fails its checksum.
    Checksum,
    /// The device refused to read it.
    Unreadable,
    /// The page's LSN is not the one the log says it must be: a lost page write when `found` is
    /// before `expected`, a lost log record when past it.
    Stale { expected: Lsn, found: Lsn },
    /// The page holds ids in progress that the log, damaged at `at`, cannot settle.
    LogUnproven { at: Lsn },
    /// A commit log status page lost the write to `expected` and was settled from the log at
    /// `found`. The page serves again.
    Settled { expected: Lsn, found: Lsn },
    /// The page passes its checksum but is not the page the structure naming it expects.
    Shape(Shape),
}

/// How a page that passes its checksum is out of shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Shape {
    /// Every byte of the page is zero.
    Blank,
    /// The page's kind, or its role within its kind, is not the one expected.
    Kind,
    /// The page names another structure as its owner.
    Owner,
    /// The page's contents contradict themselves or the page naming it: a count, level, entry or
    /// slot out of range, or a version naming a writer no transaction was given.
    Frame,
    /// The page's format byte names no format of its structure.
    Format,
}

impl Damage {
    /// Returns the damage of a page the file holds at LSN `found`, other than a blank one, when the
    /// log holds no record at or past `end`.
    pub(crate) fn past_log_end(found: Lsn, end: Lsn) -> Option<Damage> {
        (found >= end && found != Lsn(0)).then_some(Damage::Stale {
            expected: end,
            found,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenError {
    NotFound,
    /// `create` found a nonempty file.
    Exists,
    /// `create` found the log directory holding what the new file did not write.
    LogPresent,
    /// `create` found its reserved directory holding what no create left.
    Reserved,
    /// Another open holds the lock on a file `create` would remove or rename.
    Locked,
    /// `create` took the log directory, then failed on a page of the new file.
    Unformatted(PageError),
    /// The options' segment size is not a positive multiple of their log block.
    SegmentSize(u64),
    /// Neither copy of the file header verifies.
    NoHeader,
    /// Opening the data file failed.
    Io(IoError),
    /// A whole copy of the file header is written in this layout, which this engine does not read.
    OtherLayout(u32),
    /// The log is damaged where recovery needs it.
    Log(LogFinding),
    /// A page opening needs, an allocation bitmap page or the quarantine marks', is damaged.
    Page(PageError),
}

impl From<PageError> for OpenError {
    fn from(e: PageError) -> OpenError {
        OpenError::Page(e)
    }
}
