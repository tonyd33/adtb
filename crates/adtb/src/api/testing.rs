use std::sync::Arc;

use crate::exec;
use crate::runtime::{Runtime, ThreadId};
use crate::storage::{self, Pager, Pid};
use crate::testing::Site;
use crate::write;

use super::{Database, Engine};

impl<R: Runtime> Database<R> {
    /// Returns the surface tests observe this database through.
    pub fn testing(&self) -> crate::testing::Testing<'_, R> {
        crate::testing::Testing::of(self)
    }

    /// Returns the engine for a test to observe, the calling thread left unmarked.
    pub(super) fn tested(&self) -> &Engine<R> {
        self.engine.enter().detach().0
    }

    pub(crate) fn tested_runtime(&self) -> &R {
        &self.tested().rt
    }

    pub(crate) fn tested_pager(&self) -> &Pager<R> {
        &self.tested().pager
    }

    pub(crate) fn tested_db(&self) -> &write::Database<R> {
        &self.tested().db
    }

    pub(crate) fn tested_catalog(&self) -> &crate::catalog::Catalog {
        &self.tested().catalog
    }

    /// Returns the store the statements of `txn` read, `None` once it has ended.
    ///
    /// Precondition: `txn` is a transaction of this database, or this panics.
    pub(crate) fn tested_store(
        &self,
        txn: &super::AdHocTransaction<'_, R>,
    ) -> Option<Arc<exec::Store<R>>> {
        assert!(
            txn.engine.ptr_eq(&self.engine),
            "the transaction is one of this database"
        );
        Some(self.tested().db.store(txn.tested_txn()?))
    }

    /// Returns the reindex job threads not yet joined.
    pub(crate) fn reindex_threads(&self) -> Vec<ThreadId> {
        self.tested()
            .reindexing
            .threads
            .lock(&self.tested().rt)
            .clone()
    }

    /// Returns what the mend pass of this process did, `None` until it has run.
    pub(crate) fn mended(&self) -> Option<crate::verify::Mended> {
        self.tested().background.mended()
    }

    /// Returns the next page of the background scrub's pass as last saved in the file: the page,
    /// `None` between passes, and `None` of the outer option when nothing has been saved.
    pub(crate) fn saved_scrub_page(&self) -> Option<Option<Pid>> {
        Some(self.tested().verifier.cursor()?.next)
    }

    /// Returns the rows of `sys::verify_findings` the engine holds, which a read of a database
    /// quarantined whole cannot show.
    pub(crate) fn held_findings(&self) -> usize {
        self.tested().verifier.held_findings()
    }

    pub(crate) fn refuse_kept_store(&self) {
        self.tested().damage_kept_store();
    }

    /// Panic at `site` with `message` the next time the engine passes it.
    pub(crate) fn panic_at(&self, site: Site, message: String) {
        match site {
            Site::BackgroundRead => self.tested().background.fail(message),
            Site::KeptWrite => {
                *self.tested().verifier.fail_kept.lock(&self.tested().rt) = Some(message);
            }
        }
    }

    pub(crate) fn archive_state(&self) -> Option<storage::ArchiveStatus> {
        self.tested().archiver.as_ref().map(|a| a.status())
    }
}

impl<R: Runtime> Engine<R> {
    /// Panic, while the engine is still open, if a transaction ended by being dropped since a
    /// test last took the count.
    ///
    /// Precondition: the calling thread is closing the engine and has not yet refused calls.
    pub(super) fn assert_none_dropped(&self) {
        if *self.open.read(&self.rt) {
            assert_eq!(
                0,
                self.db.take_ends_by_drop(),
                "every transaction ended before the close was committed or aborted, none dropped"
            );
        }
    }
}
