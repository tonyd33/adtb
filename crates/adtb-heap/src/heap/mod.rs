//! Hold one heap per constructor: pages, slots, versions, large values and the visibility map.

mod blame;
mod chain;
mod dir;
mod fixed;
mod handle;
mod large;
mod named;
mod page;
mod prune;
mod rooted;
mod row;
#[cfg(any(test, feature = "testing"))]
mod scribble;
mod slotted;
mod stage;
mod undo;
mod version;
mod vm;
mod write;

#[cfg(test)]
mod claims_tests;
#[cfg(test)]
mod heap_tests;
#[cfg(test)]
mod heap_threads_tests;
#[cfg(test)]
mod row_damage_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod unreserved_tests;

pub use chain::{successor, walk};
#[cfg(test)]
pub use dir::max_fanout;
#[cfg(any(test, feature = "testing"))]
pub use handle::Laid;
#[cfg(any(test, feature = "testing"))]
pub use handle::Pruned;
pub use handle::{Heap, HeapNo, Link, Oversize, RowLimits, Slots, Version};
pub use large::LargeStore;
#[cfg(any(test, feature = "testing"))]
pub use named::before_next_refusal;
pub use named::{OrphanEntry, refuse_named, vacant};
#[cfg(any(test, feature = "testing"))]
pub use page::Slot;
#[cfg(any(test, feature = "testing"))]
pub use row::Layout;
#[cfg(any(test, feature = "testing"))]
pub use row::Pointer;
pub use row::put_uvarint;
pub use row::{CtorType, FieldType};
#[cfg(any(test, feature = "testing"))]
pub use scribble::{Edit, scribble};
#[cfg(any(test, feature = "testing"))]
pub use undo::Tested;
pub use undo::{Abandons, UncheckedWriter, Undo, Undoable};
#[cfg(test)]
pub use version::HEADER_LEN;
pub use version::{Flags, Header};

use crate::key::Tid;
use crate::mvcc::{Conflict, SerialId, Serializer};
use crate::steps::Change;
use crate::storage::PageError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Page(PageError),
    /// The version a write targets changed between its write check and its exclusive latch.
    Changed(Conflict),
    /// A version of `len` bytes is longer than the `limit` a page holds, even with its large
    /// values out of line.
    TooLarge {
        len: usize,
        limit: usize,
    },
    /// The heap or store was opened at page 0, with no directory to take a page.
    NoDirectory,
}

impl From<PageError> for Error {
    fn from(e: PageError) -> Error {
        Error::Page(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Page(e) => write!(f, "{e}"),
            Error::Changed(_) => {
                f.write_str("the version a write targets changed before its exclusive latch")
            }
            Error::TooLarge { len, limit } => write!(
                f,
                "a version of {len} bytes is longer than the {limit} a page holds"
            ),
            Error::NoDirectory => f.write_str("the heap has no directory"),
        }
    }
}

/// The failure of a refusal judged again under the page's exclusive latch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    Page(PageError),
    /// The page no longer holds what was judged of it, and the caller cannot read it again: a
    /// status lookup's version or chunk naming the writer is gone, or a write's stage was
    /// abandoned to latch the page.
    Gone,
}

impl From<PageError> for Refused {
    fn from(e: PageError) -> Refused {
        Refused::Page(e)
    }
}

impl Refused {
    /// Returns this refusal as the failure of a write to heap `heap`: a gone version as a write
    /// whose version changed.
    pub fn of_heap(self, heap: HeapNo) -> Error {
        match self {
            Refused::Page(e) => Error::Page(e),
            Refused::Gone => Error::Changed(Conflict::changed(heap.0, Vec::new())),
        }
    }
}

/// A version a delete, update or retag gave its deleter, the change in its page.
#[must_use = "check the write against the SIREAD locks, or say why not with `unchecked`"]
#[derive(Debug, PartialEq, Eq)]
pub struct Superseded {
    heap: HeapNo,
    tid: Tid,
    change: Change,
}

impl Superseded {
    pub fn change(&self) -> Change {
        self.change
    }

    /// Record the write by `id` of this version against the SIREAD locks on it.
    pub fn check(self, ser: &mut Serializer, id: SerialId) -> Result<(), Conflict> {
        ser.write_at(id, self.heap.0, self.tid)
    }

    /// Drop the token without a SIREAD check, for `why`.
    pub fn unchecked(self, _why: &impl UncheckedWriter) {}

    pub fn heap(&self) -> HeapNo {
        self.heap
    }

    pub fn tid(&self) -> Tid {
        self.tid
    }
}

/// A version to change, optionally requiring its header to remain unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteTarget {
    pub tid: Tid,
    expected: Option<Header>,
}

impl WriteTarget {
    /// Require the version at `tid` to retain `header` until its exclusive latch is held.
    pub fn checked(tid: Tid, header: Header) -> Self {
        Self {
            tid,
            expected: Some(header),
        }
    }

    /// Fail as changed, a write of heap `heap`, unless the version holds the header required.
    fn check(self, heap: HeapNo, current: Option<Header>) -> Result<(), Error> {
        if self
            .expected
            .is_some_and(|expected| Some(expected) != current)
        {
            let deleters = current.and_then(|h| h.writers.deleter());
            Err(Error::Changed(Conflict::changed(
                heap.0,
                deleters.into_iter().collect(),
            )))
        } else {
            Ok(())
        }
    }
}

#[cfg(any(test, feature = "testing"))]
impl From<Tid> for WriteTarget {
    fn from(tid: Tid) -> Self {
        Self {
            tid,
            expected: None,
        }
    }
}
