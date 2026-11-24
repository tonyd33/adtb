use crate::heap::{self, Edit, Laid};
use crate::mvcc::Xid;
use crate::runtime::Runtime;
use crate::storage::{self as page, PageKind, Pid};

use super::{Failed, Object, Testing};

impl<R: Runtime> Testing<'_, R> {
    /// Returns the first transaction id past the commit log's reservation.
    pub fn unreserved(self) -> Xid {
        self.db.tested_db().txns().commit_log().reserved_tested()
    }

    /// Returns each position of the directory of heap `object` with what its page holds.
    pub fn layout(self, object: Object) -> Result<Vec<Laid>, Failed> {
        Ok(self.heap_object(object)?.layout(self.db.tested_pager())?)
    }

    /// Apply `edit` to `page` as one logged change. Fails as unchanged when the page holds
    /// nothing the edit applies to.
    pub fn scribble(self, page: Pid, edit: Edit) -> Result<(), Failed> {
        if heap::scribble(self.db.tested_pager(), page, edit)? {
            Ok(())
        } else {
            Err(Failed::Unchanged)
        }
    }

    /// Allocate a page of the kind and owner of `like`, its contents zero, that no structure
    /// names. Returns it.
    pub fn leak(self, like: Pid) -> Result<Pid, Failed> {
        let pager = self.db.tested_pager();
        let read = pager.read(like)?;
        let kind = PageKind::from_u16(page::kind(&read)).ok_or(Failed::Unchanged)?;
        let owner = read.owner();
        drop(read);
        Ok(pager.allocate(1, kind, owner)?)
    }
}
