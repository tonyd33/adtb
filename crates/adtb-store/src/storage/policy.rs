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
}

/// What was detected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// A page refused with `Damage`.
    Page(Damage),
    /// The damage list overflowed: every record maps alike.
    Full,
    /// Recovery could not prove the expected LSNs of the open: a page LSN map page out of shape,
    /// or a gap in the retained log.
    Unarmed,
    /// An `Order`, `Orphan` or `Missing` finding of verify, or an index entry naming an unused
    /// slot, on the index it names.
    Finding,
}

/// What a fault marks, as [`mark`] decides it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Mark(Kind);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Nothing,
    /// One heap or index.
    Object(Quarantine),
    /// The database, durably.
    Database,
    /// The database, held for this open only and never written.
    Held,
}

impl Mark {
    const NOTHING: Mark = Mark(Kind::Nothing);
    const DATABASE: Mark = Mark(Kind::Database);
    const HELD: Mark = Mark(Kind::Held);

    /// Returns a durable mark of `object` that no fault decided.
    #[cfg(any(test, feature = "testing"))]
    pub(in crate::storage) fn planted(object: Quarantine) -> Mark {
        Mark(Kind::Object(object))
    }

    pub fn is_nothing(self) -> bool {
        self == Mark::NOTHING
    }

    /// Returns the object to mark durably.
    pub fn durable(self) -> Option<Quarantine> {
        match self.0 {
            Kind::Object(object) => Some(object),
            Kind::Database => Some(Quarantine::Database),
            Kind::Nothing | Kind::Held => None,
        }
    }

    /// Returns whether this mark holds the database for an open only.
    pub fn open_only(self) -> bool {
        self == Mark::HELD
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
        Fault::Full => return Mark::DATABASE,
        Fault::Unarmed => return Mark::HELD,
        Fault::Finding => return found(holder),
        Fault::Page(Damage::Settled { .. }) => return Mark::NOTHING,
        Fault::Page(damage) => damage,
    };
    match holder {
        _ if damage == Damage::Unreadable => Mark::NOTHING,
        Holder::Relation {
            object,
            engine_module: false,
        } => Mark(Kind::Object(object)),
        Holder::CommitLog if matches!(damage, Damage::LogUnproven { .. }) => match phase {
            Phase::Opening => Mark::HELD,
            Phase::Running => Mark::DATABASE,
        },
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
        | Holder::LsnMap => Mark::DATABASE,
        Holder::CatalogRows => match phase {
            Phase::Opening => Mark::DATABASE,
            Phase::Running => Mark::NOTHING,
        },
        Holder::Unreached => match phase {
            Phase::Opening => Mark::NOTHING,
            Phase::Running => Mark::DATABASE,
        },
        Holder::Log | Holder::Retired | Holder::Free => Mark::NOTHING,
    }
}

/// Returns what a finding on `holder` marks.
fn found(holder: Holder) -> Mark {
    match holder {
        Holder::Relation {
            object: object @ Quarantine::Index(_),
            ..
        } => Mark(Kind::Object(object)),
        _ => Mark::NOTHING,
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
        .filter(|m| *m != Mark::NOTHING)
        .collect();
    if marks.contains(&Mark::DATABASE) {
        BTreeSet::from([Mark::DATABASE])
    } else {
        marks
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
