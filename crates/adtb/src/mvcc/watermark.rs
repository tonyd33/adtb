use std::collections::{BTreeMap, BTreeSet};

use super::Xid;

/// The heaps a snapshot holds back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// A reader whose reads are unknown, such as an interactive transaction.
    Every,
    /// A reader that can read only these heaps, by heap id.
    Heaps(BTreeSet<u32>),
}

impl Scope {
    fn covers(&self, heap: u32) -> bool {
        match self {
            Scope::Every => true,
            Scope::Heaps(hs) => hs.contains(&heap),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SnapshotId(u64);

/// The snapshots and replica reports that bound reclamation, heap by heap.
#[derive(Debug, Default)]
pub struct Holds {
    issued: u64,
    snapshots: BTreeMap<SnapshotId, (Xid, Scope)>,
    replicas: BTreeMap<u64, BTreeMap<u32, Xid>>,
}

impl Holds {
    /// Hold back every heap in `scope` at `xmin` until the returned id is released.
    pub fn hold(&mut self, xmin: Xid, scope: Scope) -> SnapshotId {
        self.issued += 1;
        let id = SnapshotId(self.issued);
        self.snapshots.insert(id, (xmin, scope));
        id
    }

    /// Panics if `id` is not held.
    pub fn release(&mut self, id: SnapshotId) {
        self.snapshots
            .remove(&id)
            .unwrap_or_else(|| panic!("{id:?} released twice"));
    }

    /// Replace what `replica` last reported with `oldest`, the oldest snapshot it holds per heap.
    pub fn report_replica(&mut self, replica: u64, oldest: BTreeMap<u32, Xid>) {
        self.replicas.insert(replica, oldest);
    }

    pub fn forget_replica(&mut self, replica: u64) {
        self.replicas.remove(&replica);
    }

    /// Returns the watermark of `heap` when no reader holds it below `next`, the next id to assign.
    pub fn watermark(&self, heap: u32, next: Xid) -> Xid {
        let local = self
            .snapshots
            .values()
            .filter(|(_, scope)| scope.covers(heap))
            .map(|(xmin, _)| *xmin);
        let remote = self.replicas.values().filter_map(|r| r.get(&heap).copied());
        local.chain(remote).fold(next, Xid::min)
    }
}
