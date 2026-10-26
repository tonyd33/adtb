use crate::runtime::Runtime;
use crate::storage::{FIRST_EDITABLE, PageError, PageExclusive, Pager, Pid, Reservation};

/// Pages being changed together, each held under its exclusive latch from the moment it is
/// staged, written back by one pager update.
///
/// Callers stage pages in the order their structure defines: heap pages in page id order, then
/// large-value store pages in page id order, then directory pages by level, the highest first,
/// then visibility map pages in page id order.
pub(super) struct Stage<'p, R: Runtime> {
    pages: Vec<(PageExclusive<'p, R>, Vec<u8>)>,
}

impl<R: Runtime> std::fmt::Debug for Stage<'_, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let pids: Vec<Pid> = self.pages.iter().map(|(g, _)| g.pid()).collect();
        f.debug_struct("Stage").field("pages", &pids).finish()
    }
}

impl<'p, R: Runtime> Stage<'p, R> {
    pub fn new() -> Stage<'p, R> {
        Stage { pages: Vec::new() }
    }

    fn find(&self, pid: Pid) -> Option<usize> {
        self.pages.iter().position(|(g, _)| g.pid() == pid)
    }

    /// Returns a copy of `pid` as staged, or as the pager holds it when it is not staged. A page
    /// not staged is read under a shared latch released before this returns.
    pub fn read(&self, pager: &'p Pager<R>, pid: Pid) -> Result<Vec<u8>, PageError> {
        match self.find(pid) {
            Some(i) => Ok(self.pages[i].1.clone()),
            None => Ok(pager.read(pid)?.to_vec()),
        }
    }

    pub fn holds(&self, pid: Pid) -> bool {
        self.find(pid).is_some()
    }

    /// Release the latch on `pid` and forget its staged copy.
    ///
    /// Precondition: the staged copy of `pid` is unchanged.
    pub fn unstage(&mut self, pid: Pid) {
        if let Some(i) = self.find(pid) {
            self.pages.remove(i);
        }
    }

    /// Returns the staged copy of `pid`, latching and staging it first if needed.
    pub fn page(&mut self, pager: &'p Pager<R>, pid: Pid) -> Result<&mut Vec<u8>, PageError> {
        let i = match self.find(pid) {
            Some(i) => i,
            None => {
                let g = pager.exclusive(pid)?;
                let bytes = g.to_vec();
                self.pages.push((g, bytes));
                self.pages.len() - 1
            }
        };
        Ok(&mut self.pages[i].1)
    }

    /// Stage `bytes` as the new contents of `pid`, latching it first if needed.
    pub fn put(&mut self, pager: &'p Pager<R>, pid: Pid, bytes: Vec<u8>) -> Result<(), PageError> {
        *self.page(pager, pid)? = bytes;
        Ok(())
    }

    /// Write every staged page back in one pager update, so a crash keeps all of them or none.
    pub fn commit(self, pager: &'p Pager<R>) -> Result<(), PageError> {
        self.commit_with(pager, pager.reserve())
    }

    /// Write every staged page back as [`Stage::commit`] does, allocating and freeing the pages of
    /// `new` in the same update.
    pub fn commit_with(
        mut self,
        pager: &'p Pager<R>,
        new: Reservation<'_, R>,
    ) -> Result<(), PageError> {
        if self.pages.is_empty() {
            return Ok(());
        }
        let copies: Vec<Vec<u8>> = self.pages.iter().map(|(_, b)| b.clone()).collect();
        let mut guards: Vec<&mut PageExclusive<'p, R>> =
            self.pages.iter_mut().map(|(g, _)| g).collect();
        pager.update_all_with(new, &mut guards, |views| {
            for (view, bytes) in views.iter_mut().zip(&copies) {
                view[FIRST_EDITABLE..].copy_from_slice(&bytes[FIRST_EDITABLE..]);
            }
        })?;
        Ok(())
    }
}
