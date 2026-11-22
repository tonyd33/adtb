use crate::key::Tid;
use crate::mvcc::{Writers, Xid};
use crate::runtime::Runtime;
use crate::steps::Change;
use crate::storage::{PageError, PageExclusive, Pager, Pid, Shape};

use super::super::page::{HeapPage, Owned, Slot};
use super::super::version::{Flags, Header};
use super::super::write::{HeapWrite, Rank};
use super::super::{Error, Superseded, Undo, Undoable, WriteTarget};
use super::Heap;

impl Heap {
    /// Mark the version at `at` deleted by `xid`. Returns the version, with the change that takes
    /// the delete back.
    pub fn delete<R: Runtime>(
        &self,
        pager: &Pager<R>,
        at: impl Into<WriteTarget>,
        xid: Xid,
    ) -> Undoable<Superseded> {
        self.put_deleter(pager, at.into(), xid)
            .unwrap_or_else(Undoable::failed)
    }

    fn put_deleter<R: Runtime>(
        &self,
        pager: &Pager<R>,
        target: WriteTarget,
        xid: Xid,
    ) -> Result<Undoable<Superseded>, Error> {
        let mut write = HeapWrite::new(pager);
        write.latch(&[(self, Pid(target.tid.page()))])?;
        self.supersede(
            write.stage(),
            pager,
            target,
            |w| w.superseded_by(xid),
            None,
            Flags::NONE,
        )?;
        let committed = write.commit_in_place().finish();
        let old = self.superseded(target.tid, Change::Delete);
        Undoable::committed(committed, old, Some(Undo::Restore(target.tid, xid)), None)
    }

    /// Take back `undo`, a change the caller's running transaction made to a version of this
    /// heap: delete the version it inserted, or clear the deleter it set with that version's
    /// `next` and successor flags. Refuse the version's page as out of frame, changing nothing,
    /// when the page holds no version of this heap there or `undo`'s transaction is not its
    /// inserter or deleter as `undo` says.
    ///
    /// Preconditions: a successor the transaction wrote of a version it restores is taken back
    /// first.
    pub fn take_back<R: Runtime>(&self, pager: &Pager<R>, undo: Undo) -> Result<(), Error> {
        let tid = undo.tid();
        let pid = Pid(tid.page());
        let mut write = HeapWrite::new(pager);
        write.latch(&[(self, pid)])?;
        let copy = write.stage().read(pager, Rank::Heap(pid))?;
        let header = self.header_at(&copy, tid).filter(|h| match undo {
            Undo::Kill(_, xid) => h.writers.inserted_by(xid),
            Undo::Restore(_, xid) => h.writers.deleted_by(xid),
        });
        let Some(header) = header else {
            return Err(write.stage().refuse(pid, Shape::Frame).into());
        };
        let target = WriteTarget::checked(tid, header);
        let stage = write.stage();
        match undo {
            Undo::Kill(_, xid) => {
                let deleted = |w: Writers| w.superseded_by(xid);
                self.supersede(stage, pager, target, deleted, None, Flags::NONE)?;
            }
            Undo::Restore(..) => {
                self.supersede(stage, pager, target, Writers::cleared, None, Flags::NONE)?;
            }
        }
        write.commit_in_place().finish()?;
        Ok(())
    }

    /// Refuse the page of `tid` as out of frame when, under its exclusive latch, it holds no
    /// version of this heap at `tid`. Returns `None` when it holds one: the caller reads it again.
    pub fn refuse_absent<R: Runtime>(&self, pager: &Pager<R>, tid: Tid) -> Option<PageError> {
        let judge =
            |p: &PageExclusive<'_, R>| self.header_at(p, tid).is_none().then_some(Shape::Frame);
        pager.refuse_if(Pid(tid.page()), judge)
    }

    /// Returns the header of the version at `tid` on `page`, if it is a page of this heap holding
    /// one there.
    fn header_at(&self, page: &impl Owned, tid: Tid) -> Option<Header> {
        let page = HeapPage::any(page).filter(|p| p.owner() == self.id)?;
        match page.get(usize::from(tid.slot())) {
            Slot::Normal { at, .. } => Some(Header::read(&page[at])),
            _ => None,
        }
    }
}
