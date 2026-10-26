use std::collections::BTreeMap;

use adtb_isolation::{Aborted, Database};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Commit only when nothing committed a write since the transaction began.
    Optimistic,
    /// Snapshot reads; commit only when no key the transaction appended to took a commit since
    /// it began.
    Snapshot,
    ReadCommitted,
    /// Appends are visible to every transaction at once, and undone on abort.
    Uncommitted,
}

/// An in-memory database of lists with one isolation level.
#[derive(Debug)]
pub struct Memory {
    mode: Mode,
    committed: BTreeMap<i64, Vec<i64>>,
    live: BTreeMap<i64, Vec<(i64, u64)>>,
    writes: u64,
    written: BTreeMap<i64, u64>,
    next: u64,
}

#[derive(Debug)]
pub struct MemoryTxn {
    id: u64,
    began: u64,
    snapshot: BTreeMap<i64, Vec<i64>>,
    appends: Vec<(i64, i64)>,
}

impl Memory {
    pub fn new(mode: Mode) -> Memory {
        Memory {
            mode,
            committed: BTreeMap::new(),
            live: BTreeMap::new(),
            writes: 0,
            written: BTreeMap::new(),
            next: 0,
        }
    }
}

impl Database for Memory {
    type Txn = MemoryTxn;

    fn begin(&mut self) -> MemoryTxn {
        self.next += 1;
        MemoryTxn {
            id: self.next,
            began: self.writes,
            snapshot: self.committed.clone(),
            appends: Vec::new(),
        }
    }

    fn append(&mut self, txn: &mut MemoryTxn, key: i64, elem: i64) -> Result<(), Aborted> {
        if self.mode == Mode::Uncommitted {
            self.live.entry(key).or_default().push((elem, txn.id));
        }
        txn.appends.push((key, elem));
        Ok(())
    }

    fn read(&mut self, txn: &mut MemoryTxn, key: i64) -> Result<Vec<i64>, Aborted> {
        let base = match self.mode {
            Mode::Uncommitted => {
                let live = self.live.get(&key).into_iter().flatten();
                return Ok(live.map(|&(e, _)| e).collect());
            }
            Mode::ReadCommitted => &self.committed,
            Mode::Optimistic | Mode::Snapshot => &txn.snapshot,
        };
        let mut list = base.get(&key).cloned().unwrap_or_default();
        list.extend(txn.appends.iter().filter(|a| a.0 == key).map(|a| a.1));
        Ok(list)
    }

    fn commit(&mut self, txn: MemoryTxn) -> Result<(), Aborted> {
        if txn.appends.is_empty() {
            return Ok(());
        }
        let conflict = match self.mode {
            Mode::Optimistic => self.writes != txn.began,
            Mode::Snapshot => txn
                .appends
                .iter()
                .any(|(key, _)| self.written.get(key).is_some_and(|&w| w > txn.began)),
            Mode::ReadCommitted | Mode::Uncommitted => false,
        };
        if conflict {
            return Err(Aborted);
        }
        self.writes += 1;
        for (key, elem) in txn.appends {
            self.committed.entry(key).or_default().push(elem);
            self.written.insert(key, self.writes);
        }
        Ok(())
    }

    fn abort(&mut self, txn: MemoryTxn) {
        for list in self.live.values_mut() {
            list.retain(|&(_, t)| t != txn.id);
        }
    }
}
