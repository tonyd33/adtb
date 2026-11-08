use std::sync::Arc;

use crate::exec;
use crate::runtime::Runtime;
use crate::storage::{self, Pager, Pid};
use crate::testing::Site;
use crate::write;

use super::Database;
use super::reindex::PhaseHook;

impl<R: Runtime> Database<R> {
    /// Returns the surface tests observe this database through.
    pub fn testing(&self) -> crate::testing::Testing<'_, R> {
        crate::testing::Testing::of(self)
    }

    pub(crate) fn tested_runtime(&self) -> &R {
        &self.engine.rt
    }

    pub(crate) fn tested_pager(&self) -> &Pager<R> {
        &self.engine.pager
    }

    pub(crate) fn tested_db(&self) -> &write::Database<R> {
        &self.engine.db
    }

    /// Returns the store the statements of `txn` read, `None` once it has ended.
    ///
    /// Precondition: `txn` is a transaction of this database, or this panics.
    pub(crate) fn tested_store(
        &self,
        txn: &super::AdHocTransaction<'_, R>,
    ) -> Option<Arc<exec::Store<R>>> {
        assert!(
            std::ptr::eq(txn.engine, &*self.engine),
            "the transaction is one of this database"
        );
        Some(self.engine.db.store(txn.tested_txn()?))
    }

    /// Returns how many reindex job threads are not yet joined.
    pub(crate) fn reindex_threads(&self) -> usize {
        self.engine.reindexing.threads.lock(&self.engine.rt).len()
    }

    /// Returns the next page of the background scrub's pass as last saved in the file: the page,
    /// `None` between passes, and `None` of the outer option when nothing has been saved.
    pub(crate) fn saved_scrub_page(&self) -> Option<Option<Pid>> {
        Some(self.engine.verifier.cursor()?.next)
    }

    /// Returns the rows of `sys::verify_findings` the engine holds, which a read of a database
    /// quarantined whole cannot show.
    pub(crate) fn held_findings(&self) -> usize {
        self.engine.verifier.held_findings()
    }

    pub(crate) fn refuse_kept_store(&self) {
        self.engine.damage_kept_store();
    }

    /// Panic at `site` with `message` the next time the engine passes it.
    pub(crate) fn panic_at(&self, site: Site, message: String) {
        match site {
            Site::BackgroundRead => self.engine.background.fail(message),
            Site::KeptWrite => {
                *self.engine.verifier.fail_kept.lock(&self.engine.rt) = Some(message);
            }
        }
    }

    pub(crate) fn set_phase_hook(&self, hook: PhaseHook) {
        *self.engine.reindexing.hook.lock(&self.engine.rt) = Some(hook);
    }

    pub(crate) fn archive_state(&self) -> Option<storage::ArchiveStatus> {
        self.engine.archiver.as_ref().map(|a| a.status())
    }
}
