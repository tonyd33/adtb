//! What damage to a page marks, by the structure holding the page, as the storage design states
//! it.

use adtb::testing::{Holder, Quarantine};

/// The structure holding a page, as the storage design names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Held {
    /// A heap, current or history, or an index or history key index, of a relation of no
    /// engine module.
    Relation,
    /// A heap or index of a relation of an engine module.
    EngineRelation,
    LargeStore,
    CatalogRoot,
    /// The heap of the catalog's rows, which also holds the kept rows.
    CatalogRows,
    Counters,
    CommitLog,
    /// The tree recording when each transaction kept in history committed.
    CommitTimes,
    Bitmap,
    MarksPage,
    DamageList,
    LsnMap,
    /// A page of a tree a swap replaced that no sweep has freed.
    Retired,
    /// A page the allocation bitmap calls free.
    Free,
    /// An allocated page no complete walk of the structures reaches.
    Unreached,
}

impl Held {
    pub const ALL: [Held; 15] = [
        Held::Relation,
        Held::EngineRelation,
        Held::LargeStore,
        Held::CatalogRoot,
        Held::CatalogRows,
        Held::Counters,
        Held::CommitLog,
        Held::CommitTimes,
        Held::Bitmap,
        Held::MarksPage,
        Held::DamageList,
        Held::LsnMap,
        Held::Retired,
        Held::Free,
        Held::Unreached,
    ];
}

/// When a damage record is mapped: by an open, or by the end of a call or a commit once the
/// database runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    Opening,
    Running,
}

pub const PHASES: [Phase; 2] = [Phase::Opening, Phase::Running];

/// A page's holder, with the heap or index it names for a relation's page.
pub type Holding = (Held, Option<Quarantine>);

/// Returns the holding of an object whose holder the testing surface answers as `holder`.
///
/// Precondition: `holder` is a structure's, not `Log`, which names no object.
pub fn holding(holder: Holder) -> Holding {
    match holder {
        Holder::Relation {
            object,
            engine_module: false,
        } => (Held::Relation, Some(object)),
        Holder::Relation {
            engine_module: true,
            ..
        } => (Held::EngineRelation, None),
        Holder::LargeStore => (Held::LargeStore, None),
        Holder::CatalogRows => (Held::CatalogRows, None),
        Holder::CatalogRoot => (Held::CatalogRoot, None),
        Holder::CommitLog => (Held::CommitLog, None),
        Holder::Counters => (Held::Counters, None),
        Holder::CommitTimes => (Held::CommitTimes, None),
        Holder::Bitmap => (Held::Bitmap, None),
        Holder::MarksPage => (Held::MarksPage, None),
        Holder::DamageList => (Held::DamageList, None),
        Holder::LsnMap => (Held::LsnMap, None),
        Holder::Retired => (Held::Retired, None),
        Holder::Free => (Held::Free, None),
        Holder::Unreached => (Held::Unreached, None),
        Holder::Log => panic!("an object's holder is a structure: {holder:?}"),
    }
}

/// A kind of damage the storage design states one mark for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    /// A page failing its checksum or its log position.
    Damaged,
    /// A commit log status page whose lost status the retained log settled.
    Settled,
    /// A page the device cannot read.
    Unread,
    /// An LSN map page the open's recovery cannot read, leaving the expected LSNs unknown.
    Unarmed,
}

/// What a page's damage marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stated {
    Nothing,
    /// The heap or index holding the page.
    Own,
    Database,
    /// The database, for the open that met the damage only.
    Open,
}

/// Returns what damage of `class` to a page of `held` marks when mapped during `phase`.
pub fn stated(class: Class, held: Held, phase: Phase) -> Stated {
    match class {
        Class::Unarmed => return Stated::Open,
        Class::Unread if held == Held::DamageList && phase == Phase::Opening => {
            return Stated::Open;
        }
        Class::Settled | Class::Unread => return Stated::Nothing,
        Class::Damaged => {}
    }
    match held {
        Held::Relation => Stated::Own,
        Held::EngineRelation
        | Held::LargeStore
        | Held::CatalogRoot
        | Held::Counters
        | Held::CommitLog
        | Held::CommitTimes
        | Held::Bitmap
        | Held::MarksPage
        | Held::DamageList
        | Held::LsnMap => Stated::Database,
        Held::CatalogRows if phase == Phase::Opening => Stated::Database,
        Held::Unreached if phase == Phase::Running => Stated::Database,
        Held::CatalogRows | Held::Unreached | Held::Retired | Held::Free => Stated::Nothing,
    }
}

/// Returns the mark the file keeps for damage of `class` to a page of `holding` mapped during
/// `phase`.
pub fn durable(class: Class, holding: Holding, phase: Phase) -> Option<Quarantine> {
    match stated(class, holding.0, phase) {
        Stated::Own => Some(
            holding
                .1
                .expect("a relation's page names its heap or index"),
        ),
        Stated::Database => Some(Quarantine::Database),
        Stated::Nothing | Stated::Open => None,
    }
}
