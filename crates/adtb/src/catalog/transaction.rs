//! The one write transaction of the catalog: its id, its snapshot hold, the structures it makes,
//! its mark on the catalog root, and its commit or abandon.

use std::sync::Arc;

use crate::faults;
use crate::heap::Heap;
use crate::mvcc::{Begun, Committed, Scope, Transactions};
use crate::runtime::Runtime;
use crate::storage::{Pager, Pid};

use super::root::{self, Changing, Root};
use super::{Catalog, Error, Migrating, update_root};

/// A catalog transaction under way.
pub(super) struct Change<'a, R: Runtime> {
    begun: Begun,
    m: Migrating<'a, R>,
    mark: Mark,
}

/// The change a transaction recorded on the catalog root page, which an open after a crash
/// frees the pages of.
enum Mark {
    Unmarked,
    Began(root::Begun),
    /// A change the root recorded before the transaction began, which it ends as its own.
    Joined,
}

/// A committed catalog transaction whose mark is not yet ended.
#[must_use = "end it with `Catalog::end_landed`"]
pub(super) struct Landed<'a> {
    committed: Committed<'a>,
    mark: Mark,
}

impl<'a, R: Runtime> Change<'a, R> {
    /// Begin a transaction that reads the catalog at a snapshot holding `scope`.
    ///
    /// Fail as [`Transactions::begin`] and [`Transactions::take`] do, abandoning the
    /// transaction when the hold fails.
    pub(super) fn begin(
        pager: &'a Pager<R>,
        txns: &'a Transactions<R>,
        scope: Scope,
    ) -> Result<Change<'a, R>, Error> {
        let begun = txns.begin(pager)?;
        let hold = match txns.take(scope) {
            Ok(hold) => hold,
            Err(e) => {
                txns.abandon_failed(pager, begun)?;
                return Err(e.into());
            }
        };
        let m = Migrating {
            pager,
            txns,
            x: begun.xid(),
            hold,
            made: Default::default(),
            top: Default::default(),
        };
        Ok(Change {
            begun,
            m,
            mark: Mark::Unmarked,
        })
    }

    /// Record this transaction's schema change on the catalog root page `root`, or join the
    /// change it records, so an open after a crash frees every page the transaction allocated.
    ///
    /// Fail with the page's failure, abandoning the transaction.
    pub(super) fn mark(mut self, root: Pid) -> Result<Change<'a, R>, Error> {
        let kind = Changing::Schema(self.m.x);
        match root::begin_or_join(self.m.pager, root, kind) {
            Ok(Some(began)) => self.mark = Mark::Began(began),
            Ok(None) => self.mark = Mark::Joined,
            Err(e) => return Err(self.abandon(e.into())),
        }
        Ok(self)
    }

    /// Returns the writes of this transaction, read at its snapshot.
    pub(super) fn writes(&self) -> &Migrating<'a, R> {
        &self.m
    }

    /// Raise the root page's `next_heap` of `catalog` to `next_heap`, which the catalog this
    /// transaction builds holds, before it commits.
    pub(super) fn reserve(&self, catalog: &Catalog, next_heap: u32) -> Result<(), Error> {
        if next_heap > catalog.next_heap {
            update_root(self.m.pager, catalog.root, |r| {
                r.next_heap = r.next_heap.max(next_heap);
            })?;
        }
        Ok(())
    }

    /// Release the snapshot and commit, changing the catalog root page `root` through `also` in
    /// the log batch of the commit record. Returns the commit, its mark still recorded.
    ///
    /// Fail as [`Transactions::commit`] does, the transaction aborted and its mark kept.
    pub(super) fn commit_naming(
        self,
        root: Pid,
        also: impl FnOnce(&mut Root),
    ) -> Result<Landed<'a>, Error> {
        let Change { begun, m, mark } = self;
        let (pager, txns) = (m.pager, m.txns);
        drop(m);
        let committed = txns.commit_changing(pager, begun, root, |p| {
            let mut named = Root::read(p);
            also(&mut named);
            named.write(p);
        })?;
        Ok(Landed { committed, mark })
    }

    /// Release the snapshot and commit, changing the page `pid` through `also` in the log batch
    /// of the commit record, as [`Transactions::commit_changing`] does.
    pub(super) fn commit_changing(
        self,
        pid: Pid,
        also: impl FnOnce(&mut [u8]),
    ) -> Result<Committed<'a>, Error> {
        let Change { begun, m, mark: _ } = self;
        let (pager, txns) = (m.pager, m.txns);
        drop(m);
        Ok(txns.commit_changing(pager, begun, pid, also)?)
    }

    /// Release the snapshot and abandon the transaction whole after its work failed with `e`,
    /// keeping any mark for the next open. Returns `e`, or the failure of the abort record in its
    /// place.
    pub(super) fn abandon(self, e: Error) -> Error {
        let Change { begun, m, mark: _ } = self;
        let (pager, txns) = (m.pager, m.txns);
        drop(m);
        match txns.abandon_failed(pager, begun) {
            Ok(()) => e,
            Err(unrecorded) => unrecorded.into(),
        }
    }

    /// Abandon the transaction whole after its work failed with `e`, then raise `next_heap` of
    /// `catalog` past every heap id it took, free every page no structure of `catalog` reaches,
    /// and end its mark. Returns `e`, or the failure of the abort record in its place.
    ///
    /// Postcondition: the mark stays on the root page when the raise or the free fails, or the
    /// free leaves a version not counted off, and the next open frees what is left.
    pub(super) fn fail(self, catalog: &mut Catalog, e: Error) -> Error {
        let made = self.m.made.take();
        let top = self.m.top.get();
        let Change { begun, m, mark } = self;
        let (pager, txns) = (m.pager, m.txns);
        drop(m);
        if let Err(unrecorded) = txns.abandon_failed(pager, begun) {
            return unrecorded.into();
        }
        if top > catalog.next_heap {
            if update_root(pager, catalog.root, |r| r.next_heap = top).is_err() {
                return e;
            }
            catalog.next_heap = top;
        }
        let ended = match mark {
            Mark::Unmarked => return e,
            Mark::Began(began) => Some(began),
            Mark::Joined => None,
        };
        let freed = catalog.free_leaked(pager, txns, &made);
        if freed.is_ok_and(|f| f.counted)
            && let Ok(lsn) = root::end_freed(pager, catalog.root, ended)
        {
            pager.wait_durable(lsn);
        }
        e
    }
}

impl Catalog {
    /// Free every page a schema change `landed` left unreached, counting the versions of the
    /// heaps it took out of the catalog, `gone`, off their large values; then end its mark and
    /// wait until the end is durable. Leave the mark on the root for the next open when the free
    /// fails or leaves a version not counted off.
    #[track_caller]
    pub(super) fn end_landed<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        landed: Landed<'_>,
        gone: &[Arc<Heap>],
    ) {
        let Landed { committed, mark } = landed;
        let ended = match mark {
            Mark::Unmarked => {
                committed.done();
                return;
            }
            Mark::Began(began) => Some(began),
            Mark::Joined => None,
        };
        let freed = committed.then(faults::Step::FreeLeaked, || {
            self.free_leaked(pager, txns, gone)
        });
        if !freed.is_some_and(|f| f.counted) {
            return;
        }
        let ended = committed.then(faults::Step::EndChange, || {
            root::end_freed(pager, self.root, ended)
        });
        if let Some(lsn) = ended {
            pager.wait_durable(lsn);
        }
    }
}
