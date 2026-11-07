//! The search for damage a consumer misses: generated sequences over every kind of page that
//! holds data or evidence, each fault at each write-out, then every consumer of the file.

mod case;
mod consumers;
mod generate;
mod judge;
mod world;

use std::collections::{BTreeMap, BTreeSet};

use adtb_testkit::profile::{self, every_case};
use case::{FAULTS, Fault, Injection, Point};
use consumers::{CONSUMERS, Consumer};
use judge::Violation;
use world::{KINDS, Kind};

/// How much of the search a run covers.
struct Tier {
    /// The most sequences a shard plans.
    sequences: u64,
    /// The cases of each kind a shard runs.
    per_kind: usize,
}

/// Returns the tier the nextest profile asks: the per-change tier's eight cases of each kind, 32
/// under `deep`, or one in a run of another profile.
fn tier() -> Tier {
    let (sequences, per_kind) = if every_case() {
        (160, 32)
    } else if profile::is("damage") {
        (60, 8)
    } else {
        (40, 1)
    };
    Tier {
        sequences,
        per_kind,
    }
}

/// Returns the kinds a fault can hit: an earlier incarnation only of a kind the engine's code
/// frees, and a tear of no damage list or marks page. Each holds what a sequence records inside
/// the device's first atomic unit, so a tear of it lands whole or is a lost write.
fn kinds(fault: Fault) -> Vec<Kind> {
    KINDS
        .into_iter()
        .filter(|k| fault != Fault::LostEarlier || judge::freed(*k))
        .filter(|k| fault != Fault::Torn || !matches!(k, Kind::DamageList | Kind::Marks))
        .collect()
}

/// A failure main shows: the cell, the user relation the page belongs to, the shape of its
/// violation, its ticket and the case that shows it, with the second fault of a pair.
struct Known {
    fault: Fault,
    kind: Kind,
    owner: Option<world::Rel>,
    consumer: Consumer,
    what: &'static str,
    ticket: &'static str,
    seed: u64,
    at: usize,
    /// The second fault of a pair and the kind of page it hits.
    then: Option<(Point, Kind)>,
}

const BLANK_OPEN: &str = "freed-lost-write-fails-open";
const SETTLED_MARKS: &str = "settled-status-marks-database";
const UNREAD_MARKS: &str = "unread-walk-marks-database";
const UNDER_MARK: &str = "migrate-under-mark-silent";
const BESIDE: &str = "walk-damage-marks-database";
const APPLIED_UNDER_MARK: &str = "migrate-applies-under-mark";

const KNOWN: &[Known] = &[
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::CommitLog,
        owner: None,
        consumer: Consumer::Verify,
        what: "the database is marked for a settled status page",
        ticket: SETTLED_MARKS,
        seed: 3,
        at: 637,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::CommitLog,
        owner: None,
        consumer: Consumer::Read,
        what: "refused the database with a Database mark for a settled status page",
        ticket: SETTLED_MARKS,
        seed: 3,
        at: 637,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::CommitLog,
        owner: None,
        consumer: Consumer::Read,
        what: "by key: refused the database with a Database mark for a settled status page",
        ticket: SETTLED_MARKS,
        seed: 3,
        at: 637,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::CommitLog,
        owner: None,
        consumer: Consumer::Verify,
        what: "the database is marked for a settled status page",
        ticket: SETTLED_MARKS,
        seed: 35,
        at: 641,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::CommitLog,
        owner: None,
        consumer: Consumer::Read,
        what: "refused the database with a Database mark for a settled status page",
        ticket: SETTLED_MARKS,
        seed: 35,
        at: 641,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::CommitLog,
        owner: None,
        consumer: Consumer::Read,
        what: "by key: refused the database with a Database mark for a settled status page",
        ticket: SETTLED_MARKS,
        seed: 35,
        at: 641,
        then: None,
    },
    Known {
        fault: Fault::UnreadableLasting,
        kind: Kind::Heap,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Verify,
        what: "an unreadable page marks the database",
        ticket: UNREAD_MARKS,
        seed: 5,
        at: 620,
        then: None,
    },
    Known {
        fault: Fault::UnreadableLasting,
        kind: Kind::Heap,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Read,
        what: "refused the database with a Database mark for an unreadable page",
        ticket: UNREAD_MARKS,
        seed: 5,
        at: 620,
        then: None,
    },
    Known {
        fault: Fault::UnreadableLasting,
        kind: Kind::Heap,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Read,
        what: "by key: refused the database with a Database mark for an unreadable page",
        ticket: UNREAD_MARKS,
        seed: 5,
        at: 620,
        then: None,
    },
    Known {
        fault: Fault::LostEarlier,
        kind: Kind::Heap,
        owner: Some(world::Rel::Tags),
        consumer: Consumer::Settle,
        what: "the open fails",
        ticket: BLANK_OPEN,
        seed: 6,
        at: 684,
        then: None,
    },
    Known {
        fault: Fault::LostEarlier,
        kind: Kind::Directory,
        owner: Some(world::Rel::Tags),
        consumer: Consumer::Settle,
        what: "the open fails",
        ticket: BLANK_OPEN,
        seed: 3,
        at: 641,
        then: None,
    },
    Known {
        fault: Fault::LostEarlier,
        kind: Kind::Directory,
        owner: Some(world::Rel::Tags),
        consumer: Consumer::Verify,
        what: "verify names an intact page",
        ticket: BLANK_OPEN,
        seed: 3,
        at: 644,
        then: Some((
            Point {
                at: 645,
                injection: Injection::Torn,
            },
            Kind::Index,
        )),
    },
    Known {
        fault: Fault::LostEarlier,
        kind: Kind::Directory,
        owner: Some(world::Rel::Tags),
        consumer: Consumer::Scrub,
        what: "a scrub names an intact page",
        ticket: BLANK_OPEN,
        seed: 3,
        at: 644,
        then: Some((
            Point {
                at: 645,
                injection: Injection::Torn,
            },
            Kind::Index,
        )),
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Bitmap,
        owner: None,
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 3,
        at: 603,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::CommitLog,
        owner: None,
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 3,
        at: 629,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::DamageList,
        owner: None,
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 27,
        at: 637,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Directory,
        owner: Some(world::Rel::Docs),
        consumer: Consumer::Read,
        what: "refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 5,
        at: 613,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Directory,
        owner: Some(world::Rel::Docs),
        consumer: Consumer::Read,
        what: "by key: refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 5,
        at: 613,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Directory,
        owner: Some(world::Rel::Docs),
        consumer: Consumer::Verify,
        what: "the database is marked for pages that map to their relation",
        ticket: BESIDE,
        seed: 5,
        at: 613,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Engine,
        owner: None,
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 3,
        at: 604,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Index,
        owner: Some(world::Rel::Docs),
        consumer: Consumer::Read,
        what: "refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 5,
        at: 616,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Index,
        owner: Some(world::Rel::Docs),
        consumer: Consumer::Read,
        what: "by key: refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 5,
        at: 616,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Index,
        owner: Some(world::Rel::Docs),
        consumer: Consumer::Verify,
        what: "the database is marked for pages that map to their relation",
        ticket: BESIDE,
        seed: 5,
        at: 616,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Large,
        owner: None,
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 3,
        at: 617,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::LsnMap,
        owner: None,
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 3,
        at: 627,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Marks,
        owner: None,
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 26,
        at: 635,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::VisibilityMap,
        owner: Some(world::Rel::Docs),
        consumer: Consumer::Read,
        what: "refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 16,
        at: 685,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::VisibilityMap,
        owner: Some(world::Rel::Docs),
        consumer: Consumer::Read,
        what: "by key: refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 16,
        at: 685,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::VisibilityMap,
        owner: Some(world::Rel::Docs),
        consumer: Consumer::Verify,
        what: "the database is marked for pages that map to their relation",
        ticket: BESIDE,
        seed: 16,
        at: 685,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Bitmap,
        owner: None,
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 26,
        at: 604,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::CommitLog,
        owner: None,
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 26,
        at: 607,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::DamageList,
        owner: None,
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 26,
        at: 627,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Directory,
        owner: Some(world::Rel::Docs),
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 26,
        at: 613,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Directory,
        owner: Some(world::Rel::Docs),
        consumer: Consumer::Read,
        what: "refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 26,
        at: 613,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Directory,
        owner: Some(world::Rel::Docs),
        consumer: Consumer::Read,
        what: "by key: refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 26,
        at: 613,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Directory,
        owner: Some(world::Rel::Docs),
        consumer: Consumer::Verify,
        what: "the database is marked for pages that map to their relation",
        ticket: BESIDE,
        seed: 26,
        at: 613,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Engine,
        owner: None,
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 19,
        at: 604,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Index,
        owner: Some(world::Rel::Docs),
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 26,
        at: 616,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Index,
        owner: Some(world::Rel::Docs),
        consumer: Consumer::Read,
        what: "refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 26,
        at: 616,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Index,
        owner: Some(world::Rel::Docs),
        consumer: Consumer::Read,
        what: "by key: refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 26,
        at: 616,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Index,
        owner: Some(world::Rel::Docs),
        consumer: Consumer::Verify,
        what: "the database is marked for pages that map to their relation",
        ticket: BESIDE,
        seed: 26,
        at: 616,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Large,
        owner: None,
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 19,
        at: 603,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::LsnMap,
        owner: None,
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 29,
        at: 635,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::VisibilityMap,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Read,
        what: "refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 13,
        at: 630,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::VisibilityMap,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Read,
        what: "by key: refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 13,
        at: 630,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::VisibilityMap,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Verify,
        what: "the database is marked for pages that map to their relation",
        ticket: BESIDE,
        seed: 13,
        at: 630,
        then: None,
    },
    Known {
        fault: Fault::UnreadableLasting,
        kind: Kind::DamageList,
        owner: None,
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 26,
        at: 627,
        then: None,
    },
    Known {
        fault: Fault::UnreadableLasting,
        kind: Kind::LsnMap,
        owner: None,
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 3,
        at: 627,
        then: None,
    },
    Known {
        fault: Fault::UnreadableOnce,
        kind: Kind::LsnMap,
        owner: None,
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 3,
        at: 627,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Directory,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Read,
        what: "by key: refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 2,
        at: 636,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Directory,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Read,
        what: "refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 2,
        at: 636,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Directory,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Verify,
        what: "the database is marked for pages that map to their relation",
        ticket: BESIDE,
        seed: 2,
        at: 636,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Directory,
        owner: Some(world::Rel::Scraps),
        consumer: Consumer::Read,
        what: "by key: refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 2,
        at: 614,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Directory,
        owner: Some(world::Rel::Scraps),
        consumer: Consumer::Read,
        what: "refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 2,
        at: 614,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Directory,
        owner: Some(world::Rel::Scraps),
        consumer: Consumer::Verify,
        what: "the database is marked for pages that map to their relation",
        ticket: BESIDE,
        seed: 2,
        at: 614,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Index,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Read,
        what: "by key: refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 2,
        at: 638,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Index,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Read,
        what: "refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 2,
        at: 638,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Index,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Verify,
        what: "the database is marked for pages that map to their relation",
        ticket: BESIDE,
        seed: 2,
        at: 638,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Index,
        owner: Some(world::Rel::Scraps),
        consumer: Consumer::Read,
        what: "by key: refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 2,
        at: 617,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Index,
        owner: Some(world::Rel::Scraps),
        consumer: Consumer::Read,
        what: "refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 2,
        at: 617,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Index,
        owner: Some(world::Rel::Scraps),
        consumer: Consumer::Verify,
        what: "the database is marked for pages that map to their relation",
        ticket: BESIDE,
        seed: 2,
        at: 617,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Marks,
        owner: None,
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 29,
        at: 647,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Directory,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Read,
        what: "by key: refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 5,
        at: 637,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Directory,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Read,
        what: "refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 5,
        at: 637,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Directory,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Verify,
        what: "the database is marked for pages that map to their relation",
        ticket: BESIDE,
        seed: 5,
        at: 637,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Directory,
        owner: Some(world::Rel::Scraps),
        consumer: Consumer::Read,
        what: "by key: refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 5,
        at: 614,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Directory,
        owner: Some(world::Rel::Scraps),
        consumer: Consumer::Read,
        what: "refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 5,
        at: 614,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Directory,
        owner: Some(world::Rel::Scraps),
        consumer: Consumer::Verify,
        what: "the database is marked for pages that map to their relation",
        ticket: BESIDE,
        seed: 5,
        at: 614,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Index,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Read,
        what: "by key: refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 5,
        at: 639,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Index,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Read,
        what: "refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 5,
        at: 639,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Index,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Verify,
        what: "the database is marked for pages that map to their relation",
        ticket: BESIDE,
        seed: 5,
        at: 639,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Index,
        owner: Some(world::Rel::Scraps),
        consumer: Consumer::Read,
        what: "by key: refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 5,
        at: 617,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Index,
        owner: Some(world::Rel::Scraps),
        consumer: Consumer::Read,
        what: "refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 5,
        at: 617,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Index,
        owner: Some(world::Rel::Scraps),
        consumer: Consumer::Verify,
        what: "the database is marked for pages that map to their relation",
        ticket: BESIDE,
        seed: 5,
        at: 617,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::VisibilityMap,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Read,
        what: "by key: refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 16,
        at: 708,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::VisibilityMap,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Read,
        what: "refused the database with a Database mark the faults map to their relation",
        ticket: BESIDE,
        seed: 16,
        at: 708,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::VisibilityMap,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Verify,
        what: "the database is marked for pages that map to their relation",
        ticket: BESIDE,
        seed: 16,
        at: 708,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Directory,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 26,
        at: 628,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Directory,
        owner: Some(world::Rel::Scraps),
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 26,
        at: 614,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Index,
        owner: Some(world::Rel::Users),
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 26,
        at: 630,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Index,
        owner: Some(world::Rel::Scraps),
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 26,
        at: 617,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Bitmap,
        owner: None,
        consumer: Consumer::Catalog,
        what: "applies a migration under a whole-database mark",
        ticket: APPLIED_UNDER_MARK,
        seed: 3,
        at: 653,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Engine,
        owner: None,
        consumer: Consumer::Catalog,
        what: "applies a migration under a whole-database mark",
        ticket: APPLIED_UNDER_MARK,
        seed: 3,
        at: 605,
        then: None,
    },
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Large,
        owner: None,
        consumer: Consumer::Catalog,
        what: "applies a migration under a whole-database mark",
        ticket: APPLIED_UNDER_MARK,
        seed: 3,
        at: 617,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::Bitmap,
        owner: None,
        consumer: Consumer::Catalog,
        what: "applies a migration under a whole-database mark",
        ticket: APPLIED_UNDER_MARK,
        seed: 51,
        at: 647,
        then: None,
    },
];

/// The cells whose every case ends at a failed open, which no consumer after it can reach, with
/// the rule that makes the open fail.
const AT_OPEN: &[(Fault, Kind, &str)] = &[
    (
        Fault::UnreadableLasting,
        Kind::CommitLog,
        "A failed open that found no damaged page, or an unreadable one, still fails.",
    ),
    (
        Fault::UnreadableLasting,
        Kind::Marks,
        "An I/O failure reading the marks' page at open fails the open and marks nothing.",
    ),
];

/// The cells of a kind the engine's code frees that no sequence reaches, `None` for every
/// consumer, with why.
const UNREACHED: &[(Fault, Kind, Option<Consumer>, &str)] = &[
    (
        Fault::LostEarlier,
        Kind::CommitLog,
        None,
        "truncation frees only a status page wholly below its bound, and one page holds the \
         status of 32 thousand ids while a sequence assigns some thousands",
    ),
    (
        Fault::LostEarlier,
        Kind::Engine,
        None,
        "no sequence frees a page of a relation of an engine module",
    ),
    (
        Fault::LostEarlier,
        Kind::Index,
        Some(Consumer::Read),
        "the only index pages a sequence frees are those of tags, which the drop that frees them \
         leaves no read of",
    ),
];

/// Returns why the cell of `fault`, `kind` and `consumer` is never reached, `None` for a cell
/// the search reaches.
fn unreached(fault: Fault, kind: Kind, consumer: Consumer) -> Option<&'static str> {
    UNREACHED
        .iter()
        .find(|(f, k, c, _)| *f == fault && *k == kind && c.is_none_or(|c| c == consumer))
        .map(|(_, _, _, why)| *why)
}

/// Returns the rule a cell of `fault` and `kind` ends at the open by, `None` for a cell its
/// consumers must reach.
fn at_open(fault: Fault, kind: Kind) -> Option<&'static str> {
    AT_OPEN
        .iter()
        .find(|(f, k, _)| *f == fault && *k == kind)
        .map(|(_, _, rule)| *rule)
}

/// Returns up to `most` of `items`, spread from first to last.
fn spread(items: &[usize], most: usize) -> Vec<usize> {
    if items.len() <= most {
        return items.to_vec();
    }
    (0..most)
        .map(|i| items[i * (items.len() - 1) / (most - 1).max(1)])
        .collect()
}

/// A violation a case showed, with what reruns it.
#[derive(Debug)]
struct Found {
    kind: Kind,
    owner: Option<world::Rel>,
    violation: Violation,
    seed: u64,
    at: usize,
    ops: Vec<world::Op>,
    failed: BTreeMap<usize, String>,
}

/// Run the shard of `fault`: for every kind it can hit, cases until the tier's count, then fail on
/// any cell never reached or any violation no known failure names. Returns what the shard found.
fn shard(fault: Fault) -> String {
    let tier = tier();
    let found = search(fault, &tier, false);
    report(fault, &found, tier.per_kind)
}

/// Rerun every single-fault known failure of `fault`.
fn reruns(fault: Fault) {
    for k in KNOWN
        .iter()
        .filter(|k| k.fault == fault && k.then.is_none())
    {
        rerun(k);
    }
}

/// Fail unless the write-out of `k`'s case still has its kind, owner and fault and the case
/// still shows its shape.
fn rerun(k: &Known) {
    let fault = k.fault;
    let plan = case::plan(k.seed, generate::sequence(k.seed), false);
    let injection = fault.injection();
    assert_eq!(
        (Some(k.kind), k.owner, fault),
        (
            plan.kind(k.at),
            plan.owner(k.at),
            plan.fault(k.at, injection)
        ),
        "{fault:?} {:?}: seed {} at {} writes another cell; pin {} again",
        k.kind,
        k.seed,
        k.at,
        k.ticket
    );
    let first = Point {
        at: k.at,
        injection,
    };
    let outcome = match k.then {
        Some((then, kind)) => {
            assert_eq!(
                Some(kind),
                plan.kind(then.at),
                "{fault:?} {:?}: seed {} second fault at {} hits another kind; pin {} again",
                k.kind,
                k.seed,
                then.at,
                k.ticket
            );
            case::run_pair(&plan, first, then)
        }
        None => case::run(&plan, first),
    };
    let shows = outcome.is_some_and(|o| {
        o.verdicts
            .violations
            .iter()
            .any(|v| v.consumer == k.consumer && v.what == k.what)
    });
    assert!(
        shows,
        "{fault:?} {:?} {:?}: seed {} at {} no longer shows {:?} of {}; drop the expectation",
        k.kind, k.consumer, k.seed, k.at, k.what, k.ticket
    );
}

/// What a search of one fault found.
struct Searched {
    kinds: Vec<Kind>,
    cases: BTreeMap<Kind, usize>,
    endings: BTreeMap<judge::Ending, usize>,
    reached: BTreeSet<(Kind, Consumer)>,
    /// The cases of each kind whose consumers' open failed.
    failed_opens: BTreeMap<Kind, usize>,
    found: Vec<Found>,
    /// The cases a vacuum on a second thread ran during a write of.
    overlapped: usize,
}

/// Returns the cases of `fault` the tier asks, with a vacuum beside each write when
/// `concurrent`.
fn search(fault: Fault, tier: &Tier, concurrent: bool) -> Searched {
    let kinds = kinds(fault);
    let mut s = Searched {
        kinds: kinds.clone(),
        cases: BTreeMap::new(),
        endings: BTreeMap::new(),
        reached: BTreeSet::new(),
        failed_opens: BTreeMap::new(),
        found: Vec::new(),
        overlapped: 0,
    };
    let done = |kind: Kind, s: &Searched| {
        let counted = s.cases.get(&kind).copied().unwrap_or(0);
        let open = at_open(fault, kind).is_some();
        counted >= tier.per_kind
            && CONSUMERS.iter().all(|c| {
                s.reached.contains(&(kind, *c))
                    || (open && *c != Consumer::Settle)
                    || unreached(fault, kind, *c).is_some()
            })
            && (!concurrent || s.overlapped >= tier.per_kind)
    };
    let steered = generate::STEERED..generate::STEERED + tier.sequences;
    for seed in (1..=tier.sequences).chain(steered) {
        if kinds.iter().all(|k| done(*k, &s)) {
            break;
        }
        let plan = case::plan(seed, generate::sequence(seed), concurrent);
        for &kind in &kinds {
            let injection = fault.injection();
            let candidates: Vec<usize> = (0..plan.clean.trace.writes.len())
                .filter(|&at| plan.kind(at) == Some(kind) && plan.fault(at, injection) == fault)
                .collect();
            let tries = tier.per_kind * 3 + 2;
            for at in spread(&candidates, tries) {
                if done(kind, &s) {
                    break;
                }
                let Some(outcome) = case::run(&plan, Point { at, injection }) else {
                    continue;
                };
                *s.cases.entry(kind).or_default() += 1;
                *s.endings.entry(outcome.verdicts.ending).or_default() += 1;
                if !outcome.opened {
                    *s.failed_opens.entry(kind).or_default() += 1;
                }
                if outcome.overlaps > 0 {
                    s.overlapped += 1;
                }
                s.reached
                    .extend(outcome.verdicts.reached.iter().map(|c| (kind, *c)));
                for violation in outcome.verdicts.violations {
                    s.found.push(Found {
                        kind,
                        owner: plan.owner(at),
                        violation,
                        seed,
                        at,
                        ops: plan.ops.clone(),
                        failed: outcome.failed.clone(),
                    });
                }
            }
        }
    }
    s
}

/// A violation's text, its consumer, and the kind and owner of the page its fault hit.
type Shape = (Kind, Option<world::Rel>, Consumer, String);

/// Returns what `s` found, failing on any kind with fewer than `per_kind` cases, any cell its
/// consumer never reached, any cell that ends at the open by its rule but had a case whose open
/// did not fail, or any violation no known failure names.
fn report(fault: Fault, s: &Searched, per_kind: usize) -> String {
    let mut shapes: BTreeMap<Shape, (usize, &Found)> = BTreeMap::new();
    for f in &s.found {
        let key = (
            f.kind,
            f.owner,
            f.violation.consumer,
            f.violation.what.clone(),
        );
        let entry = shapes.entry(key).or_insert((0, f));
        entry.0 += 1;
        if f.ops.len() < entry.1.ops.len() {
            entry.1 = f;
        }
    }
    let mut text = format!(
        "{fault:?}: cases {:?}, endings {:?}, failed opens {:?}\n",
        s.cases, s.endings, s.failed_opens
    );
    for ((kind, owner, consumer, what), (n, f)) in &shapes {
        let ticket = known(fault, *kind, *owner, *consumer, what).map_or("unknown", |k| k.ticket);
        text += &format!(
            "{fault:?} {kind:?} {consumer:?}: {what} x{n} [{ticket}]; owner {:?}; seed {} at {}: \
             {:?}; {}; failed {:?}\n",
            f.owner, f.seed, f.at, f.ops, f.violation.detail, f.failed
        );
    }
    let whole = |k: Kind| {
        UNREACHED
            .iter()
            .any(|(f, kind, c, _)| *f == fault && *kind == k && c.is_none())
    };
    let short: Vec<(Kind, usize)> = s
        .kinds
        .iter()
        .filter(|k| !whole(**k))
        .map(|k| (*k, s.cases.get(k).copied().unwrap_or(0)))
        .filter(|(_, n)| *n < per_kind)
        .collect();
    assert_eq!(
        Vec::<(Kind, usize)>::new(),
        short,
        "{fault:?}: kinds with fewer than {per_kind} cases\n{text}"
    );
    let cells = || s.kinds.iter().flat_map(|k| CONSUMERS.map(|c| (*k, c)));
    let missed: Vec<(Kind, Consumer)> = cells()
        .filter(|(k, c)| at_open(fault, *k).is_none() || *c == Consumer::Settle)
        .filter(|(k, c)| unreached(fault, *k, *c).is_none())
        .filter(|cell| !s.reached.contains(cell))
        .collect();
    assert_eq!(
        Vec::<(Kind, Consumer)>::new(),
        missed,
        "{fault:?}: cells never reached\n{text}"
    );
    let came: Vec<(Kind, Consumer)> = cells()
        .filter(|(k, c)| unreached(fault, *k, *c).is_some() && s.reached.contains(&(*k, *c)))
        .collect();
    assert_eq!(
        Vec::<(Kind, Consumer)>::new(),
        came,
        "{fault:?}: cells listed as never reached were reached; drop their entries\n{text}"
    );
    for k in s.kinds.iter().filter(|k| at_open(fault, **k).is_some()) {
        let cases = s.cases.get(k).copied().unwrap_or(0);
        let past: Vec<_> = CONSUMERS
            .iter()
            .filter(|c| **c != Consumer::Settle && s.reached.contains(&(*k, **c)))
            .collect();
        assert_eq!(
            (cases, Vec::<&Consumer>::new()),
            (s.failed_opens.get(k).copied().unwrap_or(0), past),
            "{fault:?} {k:?}: a case got past the open {}\n{text}",
            at_open(fault, *k).unwrap_or_default()
        );
    }
    let unknown: Vec<_> = shapes
        .keys()
        .filter(|(kind, owner, consumer, what)| {
            known(fault, *kind, *owner, *consumer, what).is_none()
        })
        .collect();
    assert_eq!(
        Vec::<&Shape>::new(),
        unknown,
        "{fault:?}: violations\n{text}"
    );
    text
}

/// Returns the known failure whose cell and shape match exactly.
fn known(
    fault: Fault,
    kind: Kind,
    owner: Option<world::Rel>,
    consumer: Consumer,
    what: &str,
) -> Option<&'static Known> {
    KNOWN.iter().find(|k| {
        k.then.is_none()
            && k.fault == fault
            && k.kind == kind
            && k.owner == owner
            && k.consumer == consumer
            && k.what == what
    })
}

#[test]
fn a_pinned_known_failure_still_shows() {
    let cheapest = KNOWN
        .iter()
        .find(|k| k.ticket == SETTLED_MARKS && k.consumer == Consumer::Verify)
        .expect("the settled status page's entry is known");
    rerun(cheapest);
}

#[test]
fn a_lost_write_of_an_earlier_incarnation_judges_nothing_of_the_next() {
    eprintln!("{}", shard(Fault::LostEarlier));
    reruns(Fault::LostEarlier);
}

#[test]
fn a_lost_write_of_a_live_page_is_reported_and_marked() {
    eprintln!("{}", shard(Fault::LostCurrent));
    reruns(Fault::LostCurrent);
}

#[test]
fn a_lost_write_whose_log_is_recycled_is_still_reported() {
    eprintln!("{}", shard(Fault::Recycled));
    reruns(Fault::Recycled);
}

#[test]
fn a_torn_write_is_restored_from_its_image() {
    eprintln!("{}", shard(Fault::Torn));
    reruns(Fault::Torn);
}

#[test]
fn a_page_unreadable_once_is_a_storage_failure_then_intact() {
    eprintln!("{}", shard(Fault::UnreadableOnce));
    reruns(Fault::UnreadableOnce);
}

#[test]
fn a_page_unreadable_for_good_is_a_storage_failure_that_marks_nothing() {
    eprintln!("{}", shard(Fault::UnreadableLasting));
    reruns(Fault::UnreadableLasting);
}

/// Returns the sequences and cases of each kind a concurrent or paired tier runs: a few under
/// the per-change tier, more under `deep`.
fn wide() -> (u64, usize) {
    if every_case() { (6, 2) } else { (2, 1) }
}

#[test]
fn a_vacuum_beside_each_write_loses_no_row_and_hides_no_damage() {
    let (sequences, per_kind) = wide();
    let tier = Tier {
        sequences: sequences * 4,
        per_kind,
    };
    let mut unknown = Vec::new();
    let mut overlapped = BTreeMap::new();
    for fault in FAULTS {
        let s = search(fault, &tier, true);
        overlapped.insert(fault, s.overlapped);
        eprintln!(
            "concurrent {fault:?}: cases {:?}, {} with a vacuum during a write",
            s.cases, s.overlapped
        );
        for f in &s.found {
            let v = &f.violation;
            eprintln!(
                "concurrent {fault:?} {:?} {:?}: {}; seed {} at {}: {:?}; {}",
                f.kind, v.consumer, v.what, f.seed, f.at, f.ops, v.detail
            );
            if known(fault, f.kind, f.owner, v.consumer, &v.what).is_none() {
                unknown.push((fault, f.kind, v.consumer, v.what.clone(), f.seed, f.at));
            }
        }
    }
    let short: Vec<_> = overlapped.iter().filter(|(_, n)| **n < per_kind).collect();
    assert_eq!(
        Vec::<(&Fault, &usize)>::new(),
        short,
        "faults with fewer than {per_kind} cases a vacuum overlapped"
    );
    assert_eq!(
        Vec::<(Fault, Kind, Consumer, String, u64, usize)>::new(),
        unknown
    );
}

#[test]
fn two_faults_lose_no_acknowledged_row_and_hide_no_stale_page() {
    let (sequences, per) = wide();
    let seconds = [
        Injection::Lost,
        Injection::Torn,
        Injection::Unreadable { lasting: false },
        Injection::Unreadable { lasting: true },
    ];
    let mut ran: BTreeMap<String, usize> = BTreeMap::new();
    let mut found = Vec::new();
    for seed in 1..=sequences {
        let plan = case::plan(seed, generate::sequence(seed), false);
        let writes = plan.clean.trace.writes.len();
        let counted = |at: &usize| plan.kind(*at).is_some();
        let lost: Vec<usize> = (0..writes).filter(counted).collect();
        for first in spread(&lost, per * 3) {
            for second in seconds {
                let later: Vec<usize> = (first + 1..writes)
                    .filter(counted)
                    .filter(|at| {
                        plan.clean.trace.writes[*at].op != plan.clean.trace.writes[first].op
                    })
                    .collect();
                for at in spread(&later, per * 2) {
                    let one = Point {
                        at: first,
                        injection: Injection::Lost,
                    };
                    let two = Point {
                        at,
                        injection: second,
                    };
                    let Some(paired) = case::run_pair(&plan, one, two) else {
                        continue;
                    };
                    if paired.opened {
                        *ran.entry(format!("{second:?}")).or_default() += 1;
                    }
                    let cells = [one, two].map(|p| {
                        (
                            plan.fault(p.at, p.injection),
                            plan.kind(p.at).expect("a counted write-out has a kind"),
                            plan.owner(p.at),
                        )
                    });
                    for v in paired.verdicts.violations {
                        let pinned = KNOWN.iter().any(|k| {
                            k.seed == seed
                                && k.at == first
                                && k.then
                                    .is_some_and(|(t, _)| t.at == at && t.injection == second)
                                && k.consumer == v.consumer
                                && k.what == v.what
                        });
                        let gated = pinned
                            || cells
                                .iter()
                                .any(|(f, k, o)| known(*f, *k, *o, v.consumer, &v.what).is_some());
                        if !gated {
                            found.push((seed, first, at, second, v));
                        }
                    }
                }
            }
        }
    }
    eprintln!("pairs whose consumers ran: {ran:?}");
    for (seed, first, at, second, v) in &found {
        eprintln!(
            "pair lost at {first} then {second:?} at {at}, seed {seed}: {:?} {}; {}",
            v.consumer, v.what, v.detail
        );
    }
    for k in KNOWN.iter().filter(|k| k.then.is_some()) {
        rerun(k);
    }
    for second in seconds {
        assert!(
            ran.contains_key(&format!("{second:?}")),
            "no pair with {second:?} whose consumers ran"
        );
    }
    assert_eq!(
        Vec::<(u64, usize, usize, Injection, Violation)>::new(),
        found
    );
}
