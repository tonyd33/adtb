//! What a detected fault marks.

use std::collections::BTreeSet;

use super::{Damage, Quarantine};

/// The structure holding a damaged page, as the mapping finds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Holder {
    /// A heap of a constructor, current or history, or an index or history key index of the
    /// relation: `object` names the heap or index, and `engine_module` whether the relation is of
    /// an engine module.
    Relation {
        object: Quarantine,
        engine_module: bool,
    },
    /// The large-value store every heap shares.
    LargeStore,
    /// The heap of the catalog's rows, which also holds the kept rows.
    CatalogRows,
    CatalogRoot,
    CommitLog,
    /// A page of the catalog's counters.
    Counters,
    /// A page of the tree recording when each transaction kept in history committed.
    CommitTimes,
    Bitmap,
    MarksPage,
    DamageList,
    LsnMap,
    Log,
    /// A page of a tree a swap replaced that no sweep has freed.
    Retired,
    /// A page the allocation bitmap calls free.
    Free,
    /// An allocated page no complete walk of the structures reaches.
    Unreached,
    /// A page whose damage a commit's walks met, with no record when the mapping began.
    Met,
}

/// What was detected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// A page refused with `Damage`.
    Page(Damage),
    /// The damage list overflowed: every record maps alike.
    Full,
    /// Recovery could not read the retained log or a page LSN map page whole, so the expected
    /// LSNs of the open are unknown.
    Unarmed,
    /// An `Order`, `Orphan` or `Missing` finding of verify, or an index entry naming an unused
    /// slot, on the index it names.
    Finding,
}

/// What a fault marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mark {
    Nothing,
    /// One heap or index.
    Object(Quarantine),
    /// The database, durably.
    Database,
    /// The database for this open only, writing no mark from then on.
    Held,
}

impl Mark {
    /// Returns the object to mark durably.
    pub fn durable(self) -> Option<Quarantine> {
        match self {
            Mark::Object(object) => Some(object),
            Mark::Database => Some(Quarantine::Database),
            Mark::Nothing | Mark::Held => None,
        }
    }

    /// Returns whether this mark holds the database for an open only.
    pub fn open_only(self) -> bool {
        self == Mark::Held
    }

    /// Returns the object an open holds marked for itself.
    pub fn held(self) -> Option<Quarantine> {
        match self {
            Mark::Database | Mark::Held => Some(Quarantine::Database),
            Mark::Nothing | Mark::Object(_) => None,
        }
    }
}

/// When a fault is mapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// While the database opens, before the write path exists.
    Opening,
    Running,
}

/// Returns what `fault` on a page of `holder` marks during `phase`.
pub fn mark(fault: Fault, holder: Holder, phase: Phase) -> Mark {
    let damage = match fault {
        Fault::Full => return Mark::Database,
        Fault::Unarmed => return Mark::Held,
        Fault::Finding => return found(holder),
        Fault::Page(Damage::Settled { .. }) => return Mark::Nothing,
        Fault::Page(damage) => damage,
    };
    match holder {
        Holder::Relation {
            object,
            engine_module: false,
        } => Mark::Object(object),
        Holder::DamageList if damage == Damage::Unreadable => Mark::Held,
        Holder::LsnMap | Holder::MarksPage | Holder::CommitTimes
            if damage == Damage::Unreadable =>
        {
            Mark::Nothing
        }
        Holder::Relation {
            engine_module: true,
            ..
        }
        | Holder::LargeStore
        | Holder::CatalogRoot
        | Holder::CommitLog
        | Holder::Counters
        | Holder::CommitTimes
        | Holder::Bitmap
        | Holder::MarksPage
        | Holder::DamageList
        | Holder::LsnMap
        | Holder::Met => Mark::Database,
        Holder::CatalogRows => match phase {
            Phase::Opening => Mark::Database,
            Phase::Running => Mark::Nothing,
        },
        Holder::Unreached => match phase {
            Phase::Opening => Mark::Nothing,
            Phase::Running => Mark::Database,
        },
        Holder::Log | Holder::Retired | Holder::Free => Mark::Nothing,
    }
}

/// Returns what a finding on `holder` marks.
fn found(holder: Holder) -> Mark {
    match holder {
        Holder::Relation {
            object: object @ Quarantine::Index(_),
            ..
        } => Mark::Object(object),
        _ => Mark::Nothing,
    }
}

/// Returns what a page of `holder` failing its checksum marks during `phase`.
pub fn checksum_mark(holder: Holder, phase: Phase) -> Mark {
    mark(Fault::Page(Damage::Checksum), holder, phase)
}

/// Returns the marks of `fault` on a page `holders` hold: the database alone once any holder
/// marks it durably.
pub fn marks(
    fault: Fault,
    holders: impl IntoIterator<Item = Holder>,
    phase: Phase,
) -> BTreeSet<Mark> {
    let marks: BTreeSet<Mark> = holders
        .into_iter()
        .map(|h| mark(fault, h, phase))
        .filter(|m| *m != Mark::Nothing)
        .collect();
    if marks.contains(&Mark::Database) {
        BTreeSet::from([Mark::Database])
    } else {
        marks
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
