//! The one write transaction of the catalog: its id, its snapshot hold, the structures it makes,
//! its mark on the catalog root, and its commit or abandon.

use std::sync::Arc;

use crate::faults;
use crate::heap::Heap;
use crate::mvcc::{Begun, Committed, Scope, Transactions};
use crate::runtime::Runtime;
use crate::storage::{Lsn, Pager, Pid};

use super::root::{self, Changing, Root};
use super::{Broken, Catalog, Error, Migrating, update_root};

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

    /// Release the snapshot and commit. Returns the commit, its mark still recorded.
    ///
    /// Fail as [`Transactions::commit`] does, the transaction aborted and its mark kept.
    pub(super) fn commit(self) -> Result<Landed<'a>, Error> {
        let Change { begun, m, mark } = self;
        let (pager, txns) = (m.pager, m.txns);
        drop(m);
        let committed = txns.commit(pager, begun)?;
        Ok(Landed { committed, mark })
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
    /// Mark the catalog as rebuilding indexes, so an open after a crash frees the pages no
    /// structure names. Returns the change begun, or `None`, marking nothing, when the root page
    /// already records a change in progress, which an open after a crash frees as well.
    ///
    /// Precondition: the database is open, so no schema change is running.
    pub fn begin_reindexing<R: Runtime>(
        &self,
        pager: &Pager<R>,
    ) -> Result<Option<root::Begun>, Error> {
        Ok(root::begin_or_join(pager, self.root, Changing::Reindexing)?)
    }

    /// End the change [`Catalog::begin_reindexing`] began.
    ///
    /// Fail with [`Broken::ChangeMoved`], writing nothing, when the root page records another
    /// change in progress.
    pub fn end_reindexing<R: Runtime>(
        &self,
        pager: &Pager<R>,
        began: root::Begun,
    ) -> Result<(), Error> {
        root::end(pager, began, |_| {})?;
        Ok(())
    }

    /// Mark the catalog whose root page is `root` as changing, before the write path allocates
    /// its page of generated counters, so an open after a crash frees that page until
    /// [`Catalog::name_sequences`] names it.
    ///
    /// Fail with [`root::Unbegun::Kept`], marking nothing, when the root page records a change in
    /// progress.
    #[cfg(any(test, feature = "testing"))]
    pub fn naming_sequences<R: Runtime>(
        pager: &Pager<R>,
        root: Pid,
    ) -> Result<root::Begun, root::Unbegun> {
        root::begin(pager, root, Changing::NamingSequences)
    }

    /// Name `pid` as the write path's page of generated counters in the catalog whose root page
    /// is `root`, ending `naming` in the same write. With no `naming`, the change in progress
    /// stays for the next open to free. Returns once the naming is durable.
    ///
    /// Fail with [`Broken::ChangeMoved`], naming nothing, when the root page records another
    /// change in progress than `naming`.
    pub fn name_sequences<R: Runtime>(
        pager: &Pager<R>,
        root: Pid,
        pid: Pid,
        naming: Option<root::Begun>,
    ) -> Result<(), Error> {
        let lsn = match naming {
            Some(begun) => root::end(pager, begun, |r| r.sequences = pid)?,
            None => update_root(pager, root, |r| r.sequences = pid)?,
        };
        pager.wait_durable(lsn);
        Ok(())
    }

    /// Returns the page of the database's id counters. When the root page names none, allocate
    /// one, advance the counter of each relation whose single key field is generated by
    /// `sequence` past every key the relation holds, and name the page, durably. A change in
    /// progress the root page already records stays there for the next open to free.
    ///
    /// Precondition: no `write::Database` holds the page of id counters.
    pub fn counters<R: Runtime>(&mut self, pager: &Pager<R>) -> Result<Pid, Error> {
        if let Some(pid) = self.sequences() {
            return Ok(pid);
        }
        let naming = root::begin_or_join(pager, self.root, Changing::NamingSequences)?;
        let pid = self.counter_page(pager)?;
        Catalog::name_sequences(pager, self.root, pid, naming)?;
        self.sequences = pid;
        Ok(pid)
    }

    /// Allocate a page of id counters whose counter of each relation with a single key field
    /// generated by `sequence` is past every key the relation holds. Returns the page, named by
    /// nothing.
    pub(super) fn counter_page<R: Runtime>(&self, pager: &Pager<R>) -> Result<Pid, Error> {
        let ids = &self.schema.ids();
        crate::write::counter_page(pager, &self.schema, |r| {
            ids.key_id(r).map(|k| &self.indexes[&k.raw()].1)
        })
        .map_err(|e| match e {
            crate::write::Error::Exec(crate::exec::Error::Page(e)) => Error::Page(e),
            other => Error::from(Broken::KeyCounter(other)),
        })
    }

    /// Free every page a schema change `landed` left unreached, counting the versions of the
    /// heaps it took out of the catalog, `gone`, off their large values; then end its mark and
    /// wait until the end is durable. Leave the mark on the root for the next open when the free
    /// fails or leaves a version not counted off. Returns the LSN the log is durable past.
    #[track_caller]
    pub(super) fn end_landed<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        landed: Landed<'_>,
        gone: &[Arc<Heap>],
    ) -> Lsn {
        let Landed { committed, mark } = landed;
        let ended = match mark {
            Mark::Unmarked => return committed.done(),
            Mark::Began(began) => Some(began),
            Mark::Joined => None,
        };
        let freed = committed.then(faults::Step::FreeLeaked, || {
            self.free_leaked(pager, txns, gone)
        });
        if !freed.is_some_and(|f| f.counted) {
            return committed.done();
        }
        let ended = committed.then(faults::Step::EndChange, || {
            root::end_freed(pager, self.root, ended)
        });
        if let Some(lsn) = ended {
            pager.wait_durable(lsn);
        }
        committed.done()
    }
}
