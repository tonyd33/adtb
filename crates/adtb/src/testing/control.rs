use std::sync::Arc;

use crate::api::{DynRows, Error};
use crate::exec;
use crate::key::{Entry, Value};
use crate::mvcc::Xid;
use crate::runtime::Runtime;
use crate::storage::{
    ArchiveStatus, BackupError, BackupRun, Envelope, Finding, Manifest, PageError, PageExclusive,
    Pid, Verification,
};
use crate::write::{self, Found, HoldWait, VerifyScope};

use super::{Failed, Object, Testing};

/// The vacuum lock, held until dropped.
#[derive(Debug)]
pub struct VacuumHold<'a, R: Runtime> {
    _hold: write::VacuumHold<'a, R>,
}

/// The exclusive latch of one page, held until dropped.
pub struct PageHold<'a, R: Runtime> {
    _latch: PageExclusive<'a, R>,
}

impl<R: Runtime> std::fmt::Debug for PageHold<'_, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PageHold")
    }
}

impl<'a, R: Runtime> Testing<'a, R> {
    /// Write the log, then every changed page.
    pub fn flush(self) {
        let pager = self.db.tested_pager();
        pager.flush_log();
        pager.flush_pages(usize::MAX);
    }

    /// Returns whether the allocation bitmap calls `page` free.
    pub fn is_free(self, page: Pid) -> Result<bool, PageError> {
        self.db.tested_pager().is_free(page)
    }

    /// Returns every page the allocation bitmap calls allocated.
    pub fn allocated(self) -> Result<Vec<Pid>, PageError> {
        self.db.tested_pager().allocated()
    }

    /// Returns the pages of trees a swap replaced that no sweep has freed.
    pub fn retired(self) -> Vec<Pid> {
        self.db.tested_pager().retired()
    }

    /// Write the log.
    pub fn flush_log(self) {
        self.db.tested_pager().flush_log();
    }

    /// Write at most `max` changed pages whose changes are durable in the log. Returns how many
    /// it wrote.
    pub fn flush_pages(self, max: usize) -> usize {
        self.db.tested_pager().flush_pages(max)
    }

    /// Remove the log segments no recovery or archive needs. Returns how many it removed.
    pub fn recycle_log(self) -> usize {
        self.db.tested_pager().recycle_log()
    }

    /// Take a checkpoint.
    pub fn checkpoint(self) {
        self.db.tested_pager().checkpoint();
    }

    /// Returns the vacuum lock once no other holds it, holding off vacuum, the background
    /// worker's included, and every rebuild.
    pub fn hold_vacuum(self) -> VacuumHold<'a, R> {
        VacuumHold {
            _hold: self.db.tested_db().holding_off_vacuum(),
        }
    }

    /// Returns the exclusive latch of `page`, which every reader and writer of it waits on.
    pub fn hold_page(self, page: Pid) -> Result<PageHold<'a, R>, PageError> {
        Ok(PageHold {
            _latch: self.db.tested_pager().exclusive(page)?,
        })
    }

    /// Begin a transaction and abort it, writing nothing. Returns the id it took.
    pub fn take_xid(self) -> Result<Xid, Failed> {
        let (pager, txns) = (self.db.tested_pager(), self.db.tested_db().txns());
        let x = txns.begin(pager).map_err(Failed::Txn)?;
        txns.abort(pager, x).map_err(Failed::Txn)?;
        Ok(x)
    }

    /// Call `hook` with each phase of every later rebuild as it starts, on the job's thread.
    pub fn on_reindex_phase(self, hook: impl Fn(&str) + Send + Sync + 'static) {
        self.db.set_phase_hook(Arc::new(hook));
    }

    /// Returns the log archive's status as its last pass left it, `None` with no archive set.
    pub fn archive_status(self) -> Option<ArchiveStatus> {
        self.db.archive_state()
    }

    /// Run `verify` over the whole database with the checks of the allocation bitmap, the heaps
    /// and the large values that hold only with no writer, holding off vacuum for the whole run.
    /// Returns the rows it wrote.
    ///
    /// Precondition: no other call runs on this database while it runs.
    pub fn verify_holding(self) -> Result<DynRows, Error> {
        self.db.verify_holding()
    }

    /// Returns what storage's check of the file's pages and the retained log finds.
    pub fn verify_pages(self) -> Verification {
        self.db.tested_pager().verify()
    }

    /// Back the database up to `target` under `envelope`, calling `visit` with each page as it
    /// is first read, as storage's page copy does.
    pub fn backup(
        self,
        target: &str,
        envelope: Envelope,
        visit: &mut dyn FnMut(Pid, &mut [u8]) -> bool,
    ) -> Result<Manifest, BackupError> {
        self.db.tested_pager().backup(&mut BackupRun {
            target,
            envelope,
            visit,
            name: &|q| format!("{q:?}"),
        })
    }

    /// Returns what a check of `page` against the retained log finds, as storage reports it.
    pub fn check_page(self, page: Pid) -> Vec<Finding> {
        let pager = self.db.tested_pager();
        pager.check_page(page, &pager.read_log(true))
    }

    /// Returns what the background scrub's check of `page` finds, while writers run.
    pub fn scrub_page(self, page: Pid) -> Vec<Found> {
        let w = self.db.tested_db();
        w.scrub_page(page, &w.scrub_log())
    }

    /// Run the check of the indexes of the relation index `index` belongs to, at a snapshot
    /// taken before `began` is called, holding off vacuum across each index. Returns what it
    /// found.
    pub fn check_indexes(self, index: Object, began: impl FnOnce()) -> Result<Vec<Found>, Failed> {
        let w = self.db.tested_db();
        let Object::Index(id) = index else {
            return Err(Failed::Absent(index));
        };
        let rel = w.relation_of_index(id).ok_or(Failed::Absent(index))?;
        let snapshot = w.begin_verify().map_err(Failed::Write)?;
        began();
        let mut found = Vec::new();
        let scope = VerifyScope::Relation(rel);
        w.check_indexes(snapshot.txn(), scope, HoldWait::unbounded(), &mut found)
            .map_err(Failed::Write)?;
        Ok(found)
    }

    /// Probe key index `index` from its relation's heaps, as a read of a relation whose key
    /// index is marked does, for the entries of key prefix `key`, at the snapshot of a run of
    /// `verify`, with a grant of `grant` bytes, the call cancelled when `cancelled` is set.
    pub fn probe_heaps(
        self,
        index: Object,
        key: &[Value],
        grant: usize,
        cancelled: bool,
    ) -> Result<Result<(), exec::Error>, Failed> {
        let w = self.db.tested_db();
        let Object::Index(id) = index else {
            return Err(Failed::NotKey(index));
        };
        let routes = w.key_routes(id).ok_or(Failed::NotKey(index))?;
        let snapshot = w.begin_verify().map_err(Failed::Write)?;
        let store = w.store_at(snapshot.txn());
        let grant = exec::Budget::new(grant)
            .admit(grant)
            .expect("a budget admits its whole limit");
        let check = || {
            if cancelled {
                Err(exec::Error::Cancelled)
            } else {
                Ok(())
            }
        };
        let bounds = exec::ProbeBounds {
            grant: &grant,
            check: &check,
        };
        Ok(store.probe(id, key, &routes, bounds).map(drop))
    }

    /// Run the check a read makes of the version `entry` of `index` names in heap `heap`
    /// before it refuses the index as orphaned, the index's naming of `entry` read when the
    /// check asks.
    pub fn refuse_named(self, heap: Object, index: Object, entry: &Entry) -> Result<(), Failed> {
        let pager = self.db.tested_pager();
        let heap = self.heap_object(heap)?;
        let ix = self.index_object(index)?;
        let named = || Ok::<_, Failed>(ix.probe(pager, &entry.key)?.contains(entry));
        crate::heap::refuse_named(pager, (&heap, entry.tid), ix.tree().owner(), named)
    }
}
