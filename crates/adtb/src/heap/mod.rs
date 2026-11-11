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
mod slotted;
mod stage;
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
mod tests;
#[cfg(test)]
mod unreserved_tests;

pub use chain::{successor, walk};
#[cfg(test)]
pub use dir::max_fanout;
#[cfg(test)]
pub use handle::Pruned;
pub use handle::{Heap, HeapNo, Link, Slots, Version, fits};
pub use large::LargeStore;
#[cfg(any(test, feature = "testing"))]
pub(crate) use named::before_next_refusal;
pub use named::{OrphanEntry, refuse_named, vacant};
#[cfg(test)]
pub use row::Pointer;
pub(crate) use row::put_uvarint;
pub use row::{CtorType, FieldType};
#[cfg(test)]
pub use version::HEADER_LEN;
pub use version::{Flags, Header};

use crate::key::Tid;
use crate::storage::PageError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Page(PageError),
    Changed {
        current: Option<Header>,
    },
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

impl From<Refused> for Error {
    fn from(e: Refused) -> Error {
        match e {
            Refused::Page(e) => Error::Page(e),
            Refused::Gone => Error::Changed { current: None },
        }
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

    fn check(self, current: Option<Header>) -> Result<(), Error> {
        if self
            .expected
            .is_some_and(|expected| Some(expected) != current)
        {
            Err(Error::Changed { current })
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
