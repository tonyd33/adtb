use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

/// A commit as the client saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fate {
    /// Submitted with no answer yet; after a crash it may or may not be committed.
    InDoubt,
    Acknowledged,
    Aborted,
}

/// The client's record of every commit it attempted. It survives crashes of the simulated process.
#[derive(Clone, Debug, Default)]
pub struct Ledger {
    fates: Arc<Mutex<BTreeMap<u64, Fate>>>,
}

impl Ledger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that commit `id` was submitted.
    ///
    /// Panics if `id` was submitted before.
    pub fn submit(&self, id: u64) {
        let previous = self.fates.lock().unwrap().insert(id, Fate::InDoubt);
        assert!(previous.is_none(), "commit {id} submitted twice");
    }

    pub fn acknowledge(&self, id: u64) {
        self.resolve(id, Fate::Acknowledged);
    }

    pub fn abort(&self, id: u64) {
        self.resolve(id, Fate::Aborted);
    }

    fn resolve(&self, id: u64, fate: Fate) {
        let mut fates = self.fates.lock().unwrap();
        let slot = fates
            .get_mut(&id)
            .unwrap_or_else(|| panic!("commit {id} resolved before it was submitted"));
        assert_eq!(Fate::InDoubt, *slot, "commit {id} resolved twice");
        *slot = fate;
    }

    pub fn fate(&self, id: u64) -> Option<Fate> {
        self.fates.lock().unwrap().get(&id).copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FindingKind {
    Structure,
    CrossReference,
    Logic,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    pub kind: FindingKind,
    pub at: String,
}

/// A database reopened after a run, as the post-run checks see it.
pub trait Recovered {
    /// Returns the id of every commit the database reports as committed.
    fn committed(&self) -> BTreeSet<u64>;

    /// Returns what a full `verify` finds.
    fn verify(&self) -> Vec<Finding>;
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Violation {
    /// An acknowledged commit is missing.
    Lost(u64),
    /// A commit that was aborted, or never submitted, is reported as committed.
    Phantom(u64),
    Found(Finding),
}

/// Returns every way `db` breaks the guarantees the ledger records: acknowledged commits present,
/// nothing reported committed that was not submitted and left unaborted, and a clean `verify`.
pub fn check(ledger: &Ledger, db: &impl Recovered) -> Vec<Violation> {
    let fates = ledger.fates.lock().unwrap();
    let committed = db.committed();
    let mut out = Vec::new();
    for (&id, &fate) in fates.iter() {
        if fate == Fate::Acknowledged && !committed.contains(&id) {
            out.push(Violation::Lost(id));
        }
    }
    for &id in &committed {
        if !matches!(fates.get(&id), Some(Fate::Acknowledged | Fate::InDoubt)) {
            out.push(Violation::Phantom(id));
        }
    }
    out.extend(db.verify().into_iter().map(Violation::Found));
    out.sort();
    out
}
