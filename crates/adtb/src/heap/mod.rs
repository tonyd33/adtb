//! Hold one heap per constructor: pages, slots, versions, large values and the visibility map.

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

#[cfg(test)]
mod claims_tests;
#[cfg(test)]
mod heap_tests;
#[cfg(test)]
mod heap_threads_tests;
#[cfg(test)]
mod tests;

pub use chain::{successor, walk};
#[cfg(test)]
pub use dir::max_fanout;
#[cfg(test)]
pub use handle::Pruned;
pub use handle::{Heap, HeapNo, Link, Version, fits};
pub use large::LargeStore;
pub use named::{OrphanEntry, refuse_named};
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

impl From<Tid> for WriteTarget {
    fn from(tid: Tid) -> Self {
        Self {
            tid,
            expected: None,
        }
    }
}
