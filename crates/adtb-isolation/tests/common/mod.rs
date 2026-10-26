#![expect(
    dead_code,
    reason = "each test binary uses a different part of the shared module"
)]

use std::collections::BTreeMap;

use adtb_isolation::{Aborted, Anomaly, Database, Op, Outcome, Txn};

pub fn append(key: i64, elem: i64) -> Op {
    Op::Append { key, elem }
}

pub fn read(key: i64, list: &[i64]) -> Op {
    Op::Read {
        key,
        list: list.to_vec(),
    }
}

pub fn ok(id: u64, ops: Vec<Op>) -> Txn {
    Txn::new(id, Outcome::Committed, ops)
}

pub fn failed(id: u64, ops: Vec<Op>) -> Txn {
    Txn::new(id, Outcome::Aborted, ops)
}

pub fn in_doubt(id: u64, ops: Vec<Op>) -> Txn {
    Txn::new(id, Outcome::InDoubt, ops)
}

pub fn kind(a: &Anomaly) -> &'static str {
    match a {
        Anomaly::G0 { .. } => "G0",
        Anomaly::G1a { .. } => "G1a",
        Anomaly::G1b { .. } => "G1b",
        Anomaly::G1c { .. } => "G1c",
        Anomaly::GSingle { .. } => "G-single",
        Anomaly::G2 { .. } => "G2",
        Anomaly::Internal { .. } => "internal",
        Anomaly::Garbage { .. } => "garbage",
        Anomaly::Duplicate { .. } => "duplicate",
        Anomaly::IncompatibleOrder { .. } => "incompatible order",
    }
}

pub fn kinds(history: &[Txn]) -> Vec<&'static str> {
    adtb_isolation::check(history)
        .expect("well formed")
        .iter()
        .map(kind)
        .collect()
}

/// SplitMix64.
#[derive(Debug)]
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

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
