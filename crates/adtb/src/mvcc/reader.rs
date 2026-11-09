use crate::runtime::Runtime;
use crate::storage::{PageError, Pager};

use super::conflict::later;
use super::{Probe, Snapshot, Status, Transactions, Write, Writers, Xid};

/// The latest commit log status of a reserved id.
pub trait Statuses {
    type Error;

    /// Returns the status of `x`, an id already reserved.
    fn status(&self, x: Xid) -> Result<Status, Self::Error>;
}

/// The statuses the commit log of a set of transactions records.
#[derive(Debug)]
pub struct Logged<R: Runtime> {
    txns: Transactions<R>,
    pager: Pager<R>,
}

impl<R: Runtime> Statuses for Logged<R> {
    type Error = PageError;

    fn status(&self, x: Xid) -> Result<Status, PageError> {
        self.txns.status(&self.pager, x)
    }
}

#[cfg(test)]
impl<E, F: Fn(Xid) -> Result<Status, E>> Statuses for F {
    type Error = E;

    fn status(&self, x: Xid) -> Result<Status, E> {
        self(x)
    }
}

/// What a read of one version found: whether it is visible, and the writer its visibility
/// turned on, which [`Reader::before`] resolves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seen {
    visible: bool,
    on: Count,
}

impl Seen {
    pub fn visible(&self) -> bool {
        self.visible
    }
}

/// How a reader counts one writer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Count {
    Committed,
    Aborted,
    /// Looked up and neither counted as committed nor aborted.
    Concurrent(Xid),
    /// Left out by the snapshot without a lookup: running when it was taken, or begun after.
    Excluded(Xid),
}

/// The reader of versions at one snapshot, as the transaction `me` when it is given, through the
/// statuses of `S`.
pub struct Reader<S> {
    statuses: S,
    snapshot: Snapshot,
    me: Option<Xid>,
}

impl<S> std::fmt::Debug for Reader<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reader")
            .field("snapshot", &self.snapshot)
            .field("me", &self.me)
            .finish_non_exhaustive()
    }
}

impl<R: Runtime> Transactions<R> {
    /// Returns the reader of versions at `snapshot` as `me`, through this commit log.
    pub fn reader(
        &self,
        pager: &Pager<R>,
        snapshot: Snapshot,
        me: Option<Xid>,
    ) -> Reader<Logged<R>> {
        Reader {
            statuses: Logged {
                txns: self.clone(),
                pager: pager.clone(),
            },
            snapshot,
            me,
        }
    }
}

impl<S: Statuses> Reader<S> {
    #[cfg(test)]
    pub(super) fn over(statuses: S, snapshot: Snapshot, me: Option<Xid>) -> Reader<S> {
        Reader {
            statuses,
            snapshot,
            me,
        }
    }

    /// Returns whether the version `writers` wrote is visible. The reader sees its own writes,
    /// whatever their command.
    pub fn sees(&self, writers: &Writers) -> Result<Seen, S::Error> {
        let on = self.count(writers.inserter())?;
        if on != Count::Committed {
            return Ok(Seen { visible: false, on });
        }
        let Some(deleter) = writers.deleter() else {
            return Ok(Seen { visible: true, on });
        };
        let on = self.count(deleter)?;
        Ok(Seen {
            visible: on != Count::Committed,
            on,
        })
    }

    /// Returns the writer the read `seen` is before: the inserter of a version the reader
    /// misses, or the deleter of one it sees, when that writer neither counts as committed nor
    /// aborted.
    pub fn before(&self, seen: Seen) -> Result<Option<Xid>, S::Error> {
        Ok(match seen.on {
            Count::Committed | Count::Aborted => None,
            Count::Concurrent(x) => Some(x),
            Count::Excluded(x) => (self.statuses.status(x)? != Status::Aborted).then_some(x),
        })
    }

    /// Returns whether the snapshot counts the writer `x` as committed, consulting the commit
    /// log only for an id below its `xmax` and not in progress.
    pub fn counts_committed(&self, x: Xid) -> Result<bool, S::Error> {
        self.snapshot
            .counts_committed(x, |x| self.statuses.status(x))
    }

    fn count(&self, x: Xid) -> Result<Count, S::Error> {
        if Some(x) == self.me {
            return Ok(Count::Committed);
        }
        if self.snapshot.excludes(x) {
            return Ok(Count::Excluded(x));
        }
        Ok(match self.statuses.status(x)? {
            Status::Committed => Count::Committed,
            Status::Aborted => Count::Aborted,
            Status::InProgress => Count::Concurrent(x),
        })
    }

    /// Returns what an insert by `me`, reading at this reader's snapshot, does about the version
    /// `writers` wrote, found under the key it inserts into a unique index.
    ///
    /// A version `me` inserted and has not deleted is a conflict; whether it came from the same
    /// statement is the caller's, from its command ids.
    pub fn probe_unique(&self, me: Xid, writers: &Writers) -> Result<Probe, S::Error> {
        let status = |x| self.statuses.status(x);
        let (xmin, xmax) = (writers.inserter(), writers.deleter());
        let inserter = if xmin == me {
            Status::Committed
        } else {
            status(xmin)?
        };
        if inserter == Status::Aborted || xmax == Some(me) {
            return Ok(Probe::Skip);
        }
        let in_snapshot = |x: Xid| Ok::<bool, S::Error>(x == me || self.counts_committed(x)?);
        if let Some(xmax) = xmax {
            match status(xmax)? {
                Status::Committed if in_snapshot(xmax)? => return Ok(Probe::Skip),
                Status::Committed => return Ok(Probe::Fail(later(xmax))),
                Status::InProgress if inserter == Status::InProgress => {
                    return Ok(Probe::Wait(xmin));
                }
                Status::InProgress if in_snapshot(xmin)? => return Ok(Probe::Wait(xmax)),
                Status::InProgress => return Ok(Probe::Fail(later(xmin))),
                Status::Aborted => {}
            }
        }
        Ok(match inserter {
            Status::InProgress => Probe::Wait(xmin),
            _ if in_snapshot(xmin)? => Probe::Conflict,
            _ => Probe::Fail(later(xmin)),
        })
    }

    /// Returns what a write by `me` does about the version it found visible at its snapshot,
    /// whose writers are now `writers`.
    pub fn check_write(&self, me: Xid, writers: &Writers) -> Result<Write, S::Error> {
        let Some(xmax) = writers.deleter().filter(|&x| x != me) else {
            return Ok(Write::Proceed);
        };
        Ok(match self.statuses.status(xmax)? {
            Status::Aborted => Write::Proceed,
            Status::InProgress => Write::Wait(xmax),
            Status::Committed => Write::Fail(later(xmax)),
        })
    }
}
