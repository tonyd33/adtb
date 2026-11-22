use crate::consumed::Consumed;
use crate::key::Tid;
use crate::mvcc::Xid;

use super::Error;
use super::write::Failed;

/// A change a write made to one version, which taking the write back undoes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Undo {
    /// Delete the version at this TID, which this transaction inserted.
    Kill(Tid, Xid),
    /// Clear the deleter of the version at this TID, which this transaction set.
    Restore(Tid, Xid),
}

impl Undo {
    pub fn tid(self) -> Tid {
        let (Undo::Kill(tid, _) | Undo::Restore(tid, _)) = self;
        tid
    }
}

/// The outcome of a write to heap pages, held until the changes that take it back are recorded.
/// A write that failed after its update wrote the pages holds its failure and its take-back.
#[must_use = "record the write's take-back with `record`, or show it has none with `kept`"]
#[derive(Debug, PartialEq, Eq)]
pub struct Undoable<T> {
    done: Result<T, Error>,
    /// The deleter the write gave the version it superseded.
    old: Option<Undo>,
    /// The version the write inserted.
    new: Option<Undo>,
    /// The guard a drop panics on in a debug build unless `record` or `kept` consumed the write.
    unrecorded: Consumed,
}

/// A writer whose failure abandons its whole transaction, so that it takes none of its heap
/// writes back. Only the catalog's writer, a restore's and a heap test's are.
pub trait Abandons {}

/// The writer of a test of a heap, outside any statement.
#[cfg(any(test, feature = "testing"))]
#[derive(Debug)]
pub struct Tested;

#[cfg(any(test, feature = "testing"))]
impl Abandons for Tested {}

/// A writer whose writes of versions take no SIREAD check. Only the catalog's writer and a heap
/// test's are.
pub trait UncheckedWriter {}

#[cfg(any(test, feature = "testing"))]
impl UncheckedWriter for Tested {}

impl<T> Undoable<T> {
    /// Returns the write that failed with `e` before it changed a page.
    pub(super) fn failed(e: Error) -> Undoable<T> {
        Undoable {
            done: Err(e),
            old: None,
            new: None,
            unrecorded: Consumed::armed(),
        }
    }

    /// Returns the write `committed` reports: `done` with the changes `old` and `new` that take
    /// it back, or the commit's failure with them when the update wrote the pages. Fails with the
    /// commit's failure when the update did not.
    pub(super) fn committed(
        committed: Result<(), Failed>,
        done: T,
        old: Option<Undo>,
        new: Option<Undo>,
    ) -> Result<Undoable<T>, Error> {
        let done = match committed {
            Ok(()) => Ok(done),
            Err(f) if f.written() => Err(f.into()),
            Err(f) => return Err(f.into()),
        };
        Ok(Undoable {
            done,
            old,
            new,
            unrecorded: Consumed::armed(),
        })
    }

    /// Push the changes that take the write back onto `list`, oldest first, the superseded
    /// version's with heap `old` and the inserted version's with heap `new`. Returns the write's
    /// result.
    pub fn record<K: Copy>(self, list: &mut Vec<(K, Undo)>, old: K, new: K) -> Result<T, Error> {
        self.unrecorded.consume();
        list.extend(self.old.map(|u| (old, u)));
        list.extend(self.new.map(|u| (new, u)));
        self.done
    }

    /// Returns the write's result, taking none of its changes back, as `writer` does.
    pub fn kept(self, _writer: &impl Abandons) -> Result<T, Error> {
        self.unrecorded.consume();
        self.done
    }
}
