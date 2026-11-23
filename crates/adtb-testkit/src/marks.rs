//! What damage to a page marks, by the structure holding the page and when it is mapped, as the
//! storage design states it.

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
    Log,
    /// A page of a tree a swap replaced that no sweep has freed.
    Retired,
    /// A page the allocation bitmap calls free.
    Free,
    /// An allocated page no complete walk of the structures reaches.
    Unreached,
}

impl Held {
    pub const ALL: [Held; 16] = crate::every![Held;
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
        Held::Log,
        Held::Retired,
        Held::Free,
        Held::Unreached,
    ];
}

/// When a fault is mapped: by an open, or by the end of a call or a commit once the database
/// runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    Opening,
    Running,
}

pub const PHASES: [Phase; 2] = [Phase::Opening, Phase::Running];

/// A kind of fault the storage design states one mark for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    /// A page failing its checksum or its log position, or out of shape.
    Damaged,
    /// A commit log status page the retained log cannot prove at open.
    Unproven,
    /// A page the device cannot read.
    Unread,
    /// A commit log status page whose lost status the retained log settled.
    Settled,
    /// A damage list that ran out of room.
    Full,
    /// An LSN map page the open's recovery cannot read, leaving the expected LSNs unknown.
    Unarmed,
    /// An `Order`, `Orphan` or `Missing` finding of verify, or an index entry naming an unused
    /// slot, on the index it names.
    Finding,
}

/// What a fault marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stated {
    Nothing,
    /// The heap or index holding the page.
    Own,
    Database,
    /// The database, for the open that met the fault only.
    Open,
}

const BOTH: &[Phase] = &PHASES;
const OPENING: &[Phase] = &[Phase::Opening];
const RUNNING: &[Phase] = &[Phase::Running];

/// One row of the table: a fault of the class on a page of any of the holders, mapped in any of
/// the phases, marks what it states.
pub type Row = (Class, &'static [Held], &'static [Phase], Stated);

/// The marks the storage design states. No two rows share a class, holder and phase.
pub const ROWS: [Row; 15] = [
    (Class::Damaged, &[Held::Relation], BOTH, Stated::Own),
    (
        Class::Damaged,
        &[
            Held::EngineRelation,
            Held::LargeStore,
            Held::CatalogRoot,
            Held::Counters,
            Held::CommitLog,
            Held::CommitTimes,
            Held::Bitmap,
            Held::MarksPage,
            Held::DamageList,
            Held::LsnMap,
        ],
        BOTH,
        Stated::Database,
    ),
    (
        Class::Damaged,
        &[Held::CatalogRows],
        OPENING,
        Stated::Database,
    ),
    (
        Class::Damaged,
        &[Held::CatalogRows],
        RUNNING,
        Stated::Nothing,
    ),
    (Class::Damaged, &[Held::Unreached], OPENING, Stated::Nothing),
    (
        Class::Damaged,
        &[Held::Unreached],
        RUNNING,
        Stated::Database,
    ),
    (
        Class::Damaged,
        &[Held::Log, Held::Retired, Held::Free],
        BOTH,
        Stated::Nothing,
    ),
    (Class::Unproven, &[Held::CommitLog], OPENING, Stated::Open),
    (Class::Unread, &[Held::DamageList], OPENING, Stated::Open),
    (Class::Unread, &[Held::DamageList], RUNNING, Stated::Nothing),
    (
        Class::Unread,
        &[
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
            Held::LsnMap,
            Held::Log,
            Held::Retired,
            Held::Free,
            Held::Unreached,
        ],
        BOTH,
        Stated::Nothing,
    ),
    (Class::Settled, &Held::ALL, BOTH, Stated::Nothing),
    (Class::Full, &Held::ALL, BOTH, Stated::Database),
    (Class::Unarmed, &Held::ALL, BOTH, Stated::Open),
    (
        Class::Finding,
        &[Held::Relation, Held::EngineRelation],
        BOTH,
        Stated::Own,
    ),
];

/// Returns the index in [`ROWS`] of the row for a fault of `class` on a page of `held` mapped
/// during `phase`, `None` when the design states no mark for it.
///
/// Panic if two rows state one.
pub fn row(class: Class, held: Held, phase: Phase) -> Option<usize> {
    let mut found = ROWS
        .iter()
        .enumerate()
        .filter(|(_, r)| r.0 == class && r.1.contains(&held) && r.2.contains(&phase))
        .map(|(i, _)| i);
    let first = found.next();
    assert_eq!(
        None,
        found.next(),
        "two rows for {class:?} {held:?} {phase:?}"
    );
    first
}

/// Returns what a fault of `class` on a page of `held` marks when mapped during `phase`, `None`
/// when the design states no mark for it.
pub fn stated(class: Class, held: Held, phase: Phase) -> Option<Stated> {
    row(class, held, phase).map(|i| ROWS[i].3)
}
