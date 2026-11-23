//! Why a page or a database cannot be opened or handed out.

use std::fmt;

use crate::runtime::IoError;

use super::Lsn;
use super::block::{LogFinding, LogProblem};
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
    /// A version on the page holds a row that does not decode in its format.
    Row(Undecoded),
    /// An index entry on the page names a constructor outside its index's set.
    Stray,
}

/// Why a stored row does not decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Undecoded {
    /// A sum's stored tag names none of its constructors.
    Tag,
    /// The row ends before a value it holds, or a field's offsets lie outside the row.
    Cut,
    /// A field holds bytes past its value.
    Trailing,
    /// Stored text is not UTF-8.
    Text,
    /// An out-of-line value's pointer is not 16 bytes or names no chunk.
    Pointer,
    /// A stored `Date`, `Time` or `Decimal` lies outside its type's domain.
    Domain,
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
    /// `create` took the log directory, then found a page of the new file damaged.
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

impl fmt::Display for PageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "page {} {}", self.pid.0, self.damage)
    }
}

impl fmt::Display for Damage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Damage::Checksum => f.write_str("fails its checksum"),
            Damage::Unreadable => f.write_str("is unreadable"),
            Damage::Stale { expected, found } => write!(
                f,
                "is stale: it holds LSN {}, not the {} the log expects",
                found.0, expected.0
            ),
            Damage::LogUnproven { at } => write!(
                f,
                "holds ids in progress that the log, damaged at LSN {}, cannot settle",
                at.0
            ),
            Damage::Settled { expected, found } => write!(
                f,
                "lost its write at LSN {} and was settled from the log at LSN {}",
                expected.0, found.0
            ),
            Damage::Shape(shape) => write!(f, "is out of shape: {shape}"),
        }
    }
}

impl fmt::Display for Shape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Shape::Blank => f.write_str("it is blank"),
            Shape::Kind => f.write_str("it is of another kind"),
            Shape::Owner => f.write_str("it names another owner"),
            Shape::Frame => f.write_str("its contents contradict themselves or the page naming it"),
            Shape::Format => f.write_str("its format byte names no format"),
            Shape::Row(why) => write!(f, "a row on it does not decode: {why}"),
            Shape::Stray => f.write_str("an index entry on it names a constructor outside its set"),
        }
    }
}

impl fmt::Display for Undecoded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Undecoded::Tag => "a stored tag names no constructor",
            Undecoded::Cut => "it ends before a value it holds",
            Undecoded::Trailing => "a field holds bytes past its value",
            Undecoded::Text => "stored text is not UTF-8",
            Undecoded::Pointer => "an out-of-line pointer names no chunk",
            Undecoded::Domain => "a stored value lies outside its type's domain",
        })
    }
}

impl fmt::Display for LogFinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let at = self.lsn.0;
        match self.problem {
            LogProblem::Damaged => write!(f, "the log block at LSN {at} is damaged"),
            LogProblem::Stale => write!(f, "the log block at LSN {at} is of an older batch"),
            LogProblem::Gap => write!(f, "the log continues past a gap at LSN {at}"),
            LogProblem::Unreadable => write!(f, "the log block at LSN {at} is unreadable"),
            LogProblem::Malformed => write!(f, "the log batch at LSN {at} does not decode"),
            LogProblem::Foreign => {
                write!(
                    f,
                    "the log block at LSN {at} was written by another file's log"
                )
            }
            LogProblem::Unlogged { pid, page } => write!(
                f,
                "page {} holds LSN {}, which no record the log retains past LSN {at} wrote",
                pid.0, page.0
            ),
        }
    }
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::NotFound => f.write_str("the file does not exist"),
            OpenError::Exists => f.write_str("the file a create would make is not empty"),
            OpenError::LogPresent => {
                f.write_str("the log directory holds a log that no file here wrote")
            }
            OpenError::Reserved => {
                f.write_str("the reserved directory of a create holds what no create left")
            }
            OpenError::Locked => f.write_str("another open holds the file"),
            OpenError::Unformatted(e) => write!(f, "the new file is damaged: {e}"),
            OpenError::SegmentSize(n) => write!(
                f,
                "segment size {n} is not a positive multiple of the log block"
            ),
            OpenError::NoHeader => f.write_str("neither copy of the file header verifies"),
            OpenError::Io(e) => write!(f, "the data file does not open: {}", io_text(*e)),
            OpenError::OtherLayout(layout) => write!(
                f,
                "the file header has layout {layout}, not one this engine reads"
            ),
            OpenError::Log(finding) => write!(f, "{finding}"),
            OpenError::Page(e) => write!(f, "{e}"),
        }
    }
}

/// Returns what the device answered with `e`, for a person.
pub fn io_text(e: IoError) -> &'static str {
    match e {
        IoError::NotFound => "no such file",
        IoError::Exists => "the name is taken",
        IoError::NotADirectory => "a component of the path is not a directory",
        IoError::Full => "the device is full",
        IoError::Failed => "the device refused the operation",
    }
}
