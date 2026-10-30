mod dir;
mod fixed;
mod handle;
mod large;
mod page;
mod prune;
mod rooted;
mod row;
mod slotted;
mod stage;
mod version;
mod vm;

#[cfg(test)]
mod tests;

pub use dir::max_fanout;
pub use handle::{Heap, HeapNo, Judge, Link, Pruned, Updated, Upgrade, Version, fits, walk};
pub use large::{Chunk, ChunkCheck, LargeStore};
pub use prune::Fate;
pub(crate) use row::put_uvarint;
pub use row::{CtorType, FIXED_SUM_PAYLOAD, FieldType, Layout, POINTER_LEN, Pointer, Stored};
pub use version::{Flags, HEADER_LEN, Header, header_at, tid_bits, tid_from_bits};

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
