use super::{Lsn, Pid};

/// The LSN each page had when the pool last evicted it, for a bounded number of pages.
///
/// A page shares its slot with every page of the same residue; the latest eviction keeps the
/// slot, so a page whose slot was taken since is no longer checked.
#[derive(Debug)]
pub(crate) struct EvictedLsns {
    /// One slot per residue: the page id plus one, 0 for empty, and its LSN.
    slots: Vec<(u64, Lsn)>,
}

impl EvictedLsns {
    /// Returns an empty table of at least `capacity` slots.
    pub(crate) fn new(capacity: usize) -> EvictedLsns {
        EvictedLsns {
            slots: vec![(0, Lsn(0)); capacity.max(1).next_power_of_two()],
        }
    }

    fn slot(&self, pid: Pid) -> usize {
        (pid.0 as usize) & (self.slots.len() - 1)
    }

    pub(crate) fn insert(&mut self, pid: Pid, lsn: Lsn) {
        let i = self.slot(pid);
        self.slots[i] = (pid.0 + 1, lsn);
    }

    pub(crate) fn get(&self, pid: Pid) -> Option<Lsn> {
        let (p, lsn) = self.slots[self.slot(pid)];
        (p == pid.0 + 1).then_some(lsn)
    }

    #[cfg(test)]
    pub(crate) fn capacity(&self) -> usize {
        self.slots.len()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.slots.iter().filter(|(p, _)| *p != 0).count()
    }
}
