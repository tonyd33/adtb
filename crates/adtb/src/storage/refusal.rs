use crate::runtime::Runtime;

use super::Pid;
use super::error::{Damage, Shape};
use super::pager::{PageError, PageExclusive, PageShared, Pager};

impl<R: Runtime> Pager<R> {
    /// Returns the page `pid` under a shared latch, as [`Pager::read`] does, refusing it when every
    /// byte of it is zero.
    ///
    /// Preconditions: a structure in the database names `pid`, so the batch that named it formatted
    /// it, and the caller holds no latch on `pid`.
    pub fn read_formatted(&self, pid: Pid) -> Result<PageShared<'_>, PageError> {
        loop {
            let page = self.read(pid)?;
            if !blank(&page) {
                return Ok(page);
            }
            drop(page);
            if let Some(e) = self.refuse_if(pid, |p| blank(p).then_some(Shape::Blank)) {
                return Err(e);
            }
        }
    }

    /// Returns the exclusive latch on `pid`, as [`Pager::exclusive`] does, refusing the page when
    /// every byte of it is zero.
    ///
    /// Preconditions: a structure in the database names `pid`, so the batch that named it formatted
    /// it, and the caller holds no latch on `pid`.
    pub fn exclusive_formatted(&self, pid: Pid) -> Result<PageExclusive<'_, R>, PageError> {
        let page = self.exclusive(pid)?;
        if blank(&page) {
            return Err(page.refuse(Shape::Blank));
        }
        Ok(page)
    }

    /// Take the exclusive latch on `pid` and refuse the page as out of the shape `judge` finds it,
    /// judged under that latch. Returns the refusal, the page's damage when it cannot be latched,
    /// or `None` when `judge` finds the page in shape: it changed since the caller judged it.
    ///
    /// Preconditions: the caller holds no latch on `pid`, and neither the log lock nor the meta
    /// lock; `judge` takes no latch.
    pub fn refuse_if(
        &self,
        pid: Pid,
        judge: impl FnOnce(&PageExclusive<'_, R>) -> Option<Shape>,
    ) -> Option<PageError> {
        let page = match self.exclusive(pid) {
            Ok(page) => page,
            Err(e) => return Some(e),
        };
        let shape = judge(&page)?;
        Some(page.refuse(shape))
    }

    /// Refuse `pid` from now on, dropping its memory. Memory holding changes the file lacks is
    /// written first, so the next force of the data file drops its dirty entry; during recovery
    /// or on a file quarantined whole it is dropped unwritten and its dirty entry stays until a
    /// free of the page is durable, so recovery replays them. Record the damage unless it is
    /// [`Damage::Unreadable`], durable on return once recovery has ended and unless the file is
    /// quarantined whole.
    ///
    /// Preconditions: the caller holds no latch on `pid`, and neither the log lock nor the meta
    /// lock.
    pub(super) fn damage(&self, pid: Pid, damage: Damage) -> PageError {
        self.refuse_latched(self.hold(pid), damage)
    }

    /// Refuse `pid` as [`Pager::damage`] does, for a test planting `damage`.
    #[cfg(any(test, feature = "testing"))]
    pub fn plant_damage(&self, pid: Pid, damage: Damage) -> PageError {
        self.damage(pid, damage)
    }
}

fn blank(page: &[u8]) -> bool {
    page.iter().all(|&b| b == 0)
}
