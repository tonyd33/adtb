use crate::runtime::Runtime;
use crate::storage::{FIRST_EDITABLE, PageError, PageExclusive, Pager, Pid, Reservation};

use super::write::Rank;

/// One staged page: its latch, its staged copy, and its rank when it was latched by rank.
struct Held<'p, R: Runtime> {
    guard: PageExclusive<'p, R>,
    bytes: Vec<u8>,
    rank: Option<Rank>,
}

/// Pages being changed together, each held under its exclusive latch from the moment it is
/// staged, written back by one pager update.
pub(super) struct Stage<'p, R: Runtime> {
    pages: Vec<Held<'p, R>>,
    /// The rank of the highest latch an enclosing change holds while this one runs.
    floor: Option<Rank>,
}

impl<R: Runtime> std::fmt::Debug for Stage<'_, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let pids: Vec<Pid> = self.pages.iter().map(|h| h.guard.pid()).collect();
        f.debug_struct("Stage").field("pages", &pids).finish()
    }
}

impl<'p, R: Runtime> Stage<'p, R> {
    pub fn new() -> Stage<'p, R> {
        Stage::above(None)
    }

    /// Returns an empty stage for an update made while latches up to `floor` are held.
    pub fn above(floor: Option<Rank>) -> Stage<'p, R> {
        Stage {
            pages: Vec::new(),
            floor,
        }
    }

    fn find(&self, pid: Pid) -> Option<usize> {
        self.pages.iter().position(|h| h.guard.pid() == pid)
    }

    /// Returns the rank of the highest latch held by this stage or below it.
    pub fn top(&self) -> Option<Rank> {
        self.pages
            .iter()
            .filter_map(|h| h.rank)
            .chain(self.floor)
            .max()
    }

    /// Panic, in debug builds and when `latch-checks` is on, if a latch on `rank`, shared or
    /// exclusive, is not above every latch held.
    fn ordered(&self, rank: Rank) {
        if !cfg!(any(debug_assertions, feature = "latch-checks")) {
            return;
        }
        if let Some(top) = self.top() {
            assert!(
                rank > top,
                "latch {rank:?} taken while holding {top:?} breaks the latch order"
            );
        }
    }

    /// Returns a copy of the page `rank` names as staged, or as the pager holds it when it is not
    /// staged. A page not staged is read under a shared latch released before this returns, and
    /// refused when every byte of it is zero.
    pub fn read(&self, pager: &'p Pager<R>, rank: Rank) -> Result<Vec<u8>, PageError> {
        match self.find(rank.pid()) {
            Some(i) => Ok(self.pages[i].bytes.clone()),
            None => {
                self.ordered(rank);
                Ok(pager.read_formatted(rank.pid())?.to_vec())
            }
        }
    }

    /// Returns a copy of the page `rank` names as [`Stage::read`] does, a page not staged read as
    /// the pager holds it, whatever its bytes.
    pub fn read_any(&self, pager: &'p Pager<R>, rank: Rank) -> Result<Vec<u8>, PageError> {
        match self.find(rank.pid()) {
            Some(i) => Ok(self.pages[i].bytes.clone()),
            None => {
                self.ordered(rank);
                Ok(pager.read(rank.pid())?.to_vec())
            }
        }
    }

    pub fn holds(&self, pid: Pid) -> bool {
        self.find(pid).is_some()
    }

    /// Returns the staged pages latched as heap pages.
    pub fn heap_pages(&self) -> Vec<Pid> {
        self.pages
            .iter()
            .filter_map(|h| match h.rank {
                Some(Rank::Heap(pid)) => Some(pid),
                _ => None,
            })
            .collect()
    }

    /// Panic, in debug builds and when `latch-checks` is on, if the stage holds a directory page.
    pub fn holds_no_directory(&self) {
        if !cfg!(any(debug_assertions, feature = "latch-checks")) {
            return;
        }
        let dir = self.pages.iter().find_map(|h| match h.rank {
            Some(Rank::Dir { pid, .. }) => Some(pid),
            _ => None,
        });
        assert!(dir.is_none(), "the stage holds directory page {dir:?}");
    }

    /// Returns whether the staged copy of `pid` differs from the page as latched.
    pub fn changed(&self, pid: Pid) -> bool {
        self.find(pid).is_some_and(|i| {
            let h = &self.pages[i];
            h.guard[FIRST_EDITABLE..] != h.bytes[FIRST_EDITABLE..]
        })
    }

    /// Release the latch on `pid` and forget its staged copy.
    ///
    /// Precondition: the staged copy of `pid` is unchanged.
    pub fn unstage(&mut self, pid: Pid) {
        if let Some(i) = self.find(pid) {
            self.pages.remove(i);
        }
    }

    /// Returns the staged copy of the page `rank` names, latching and staging it first if needed.
    pub fn page(&mut self, pager: &'p Pager<R>, rank: Rank) -> Result<&mut Vec<u8>, PageError> {
        let pid = rank.pid();
        let i = match self.find(pid) {
            Some(i) => i,
            None => {
                self.ordered(rank);
                let guard = pager.exclusive_formatted(pid)?;
                let bytes = guard.to_vec();
                self.pages.push(Held {
                    guard,
                    bytes,
                    rank: Some(rank),
                });
                self.pages.len() - 1
            }
        };
        Ok(&mut self.pages[i].bytes)
    }

    /// Stage `bytes` as the new contents of the page `rank` names, latching it first if needed.
    pub fn put(
        &mut self,
        pager: &'p Pager<R>,
        rank: Rank,
        bytes: Vec<u8>,
    ) -> Result<(), PageError> {
        match self.find(rank.pid()) {
            Some(i) => self.pages[i].bytes = bytes,
            None => {
                self.ordered(rank);
                let guard = pager.exclusive(rank.pid())?;
                self.pages.push(Held {
                    guard,
                    bytes,
                    rank: Some(rank),
                });
            }
        }
        Ok(())
    }

    /// Stage `bytes` as the new contents of `pid`, which `new` reserves, latching it through `new`.
    /// Its latch takes no rank.
    pub fn put_reserved(&mut self, new: &Reservation<'p, R>, pid: Pid, bytes: Vec<u8>) {
        match self.find(pid) {
            Some(i) => self.pages[i].bytes = bytes,
            None => self.pages.push(Held {
                guard: new.latch(pid),
                bytes,
                rank: None,
            }),
        }
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
        let copies: Vec<Vec<u8>> = self.pages.iter().map(|h| h.bytes.clone()).collect();
        let mut guards: Vec<&mut PageExclusive<'p, R>> =
            self.pages.iter_mut().map(|h| &mut h.guard).collect();
        pager.update_all_with(new, &mut guards, |views| {
            for (view, bytes) in views.iter_mut().zip(&copies) {
                view[FIRST_EDITABLE..].copy_from_slice(&bytes[FIRST_EDITABLE..]);
            }
        })?;
        Ok(())
    }
}
