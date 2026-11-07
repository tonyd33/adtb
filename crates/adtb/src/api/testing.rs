use crate::runtime::Runtime;
use crate::storage::{Pager, Pid};
use crate::write;

use super::Database;

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

    pub(crate) fn tested_write_path(&self) -> &write::Database<R> {
        &self.engine.db
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
}
