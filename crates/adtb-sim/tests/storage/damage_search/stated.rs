//! What damage to a page marks, by the structure holding the page, read from the storage design's
//! mark table.

use adtb::testing::{Holder, Quarantine};
pub use adtb_testkit::marks::{Class, Held, PHASES, Phase, Stated};

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

/// Returns what damage of `class` to a page of `held` marks when mapped during `phase`.
///
/// Panic when the table states no mark for it.
pub fn stated(class: Class, held: Held, phase: Phase) -> Stated {
    adtb_testkit::marks::stated(class, held, phase)
        .unwrap_or_else(|| panic!("the mark table states no row for {class:?} {held:?} {phase:?}"))
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
