use super::{Fault, Holder, Mark, Phase, mark};
use crate::storage::{Damage, Lsn, Quarantine};

const HEAP: Quarantine = Quarantine::Heap(3);
const INDEX: Quarantine = Quarantine::Index(4);

const fn relation(object: Quarantine, engine_module: bool) -> Holder {
    Holder::Relation {
        object,
        engine_module,
    }
}

const H0: Holder = relation(HEAP, false);
const H1: Holder = relation(HEAP, true);
const I0: Holder = relation(INDEX, false);
const I1: Holder = relation(INDEX, true);

const OPEN: Phase = Phase::Opening;
const RUN: Phase = Phase::Running;

const NO: Mark = Mark::Nothing;
const DB: Mark = Mark::Database;
const HELD: Mark = Mark::Held;
const OH: Mark = Mark::Object(HEAP);
const OI: Mark = Mark::Object(INDEX);

/// A kind of fault the storage design states one mark for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum C {
    /// A page failing its checksum or its log position.
    Damaged,
    Unread,
    Settled,
    Full,
    Unarmed,
}

impl C {
    const ALL: [C; 5] = [C::Damaged, C::Unread, C::Settled, C::Full, C::Unarmed];

    fn faults(self) -> Vec<Fault> {
        let (expected, found) = (Lsn(9), Lsn(5));
        match self {
            C::Damaged => vec![
                Fault::Page(Damage::Checksum),
                Fault::Page(Damage::Stale { expected, found }),
                Fault::Page(Damage::LogUnproven { at: expected }),
            ],
            C::Unread => vec![Fault::Page(Damage::Unreadable)],
            C::Settled => vec![Fault::Page(Damage::Settled { expected, found })],
            C::Full => vec![Fault::Full],
            C::Unarmed => vec![Fault::Unarmed],
        }
    }
}

const HOLDERS: [Holder; 19] = [
    H0,
    H1,
    I0,
    I1,
    Holder::LargeStore,
    Holder::CatalogRows,
    Holder::CatalogRoot,
    Holder::CommitLog,
    Holder::Counters,
    Holder::CommitTimes,
    Holder::Bitmap,
    Holder::MarksPage,
    Holder::DamageList,
    Holder::LsnMap,
    Holder::Log,
    Holder::Retired,
    Holder::Free,
    Holder::Unreached,
    Holder::Met,
];

/// The marks storage.md states, one row per fault kind, holder and phase.
const STATED: [(C, Holder, Phase, Mark); 188] = [
    (C::Damaged, H0, OPEN, OH),
    (C::Damaged, H0, RUN, OH),
    (C::Damaged, H1, OPEN, DB),
    (C::Damaged, H1, RUN, DB),
    (C::Damaged, I0, OPEN, OI),
    (C::Damaged, I0, RUN, OI),
    (C::Damaged, I1, OPEN, DB),
    (C::Damaged, I1, RUN, DB),
    (C::Damaged, Holder::LargeStore, OPEN, DB),
    (C::Damaged, Holder::LargeStore, RUN, DB),
    (C::Damaged, Holder::CatalogRows, OPEN, DB),
    (C::Damaged, Holder::CatalogRows, RUN, NO),
    (C::Damaged, Holder::CatalogRoot, OPEN, DB),
    (C::Damaged, Holder::CatalogRoot, RUN, DB),
    (C::Damaged, Holder::CommitLog, OPEN, DB),
    (C::Damaged, Holder::CommitLog, RUN, DB),
    (C::Damaged, Holder::Counters, OPEN, DB),
    (C::Damaged, Holder::Counters, RUN, DB),
    (C::Damaged, Holder::CommitTimes, OPEN, DB),
    (C::Damaged, Holder::CommitTimes, RUN, DB),
    (C::Damaged, Holder::Bitmap, OPEN, DB),
    (C::Damaged, Holder::Bitmap, RUN, DB),
    (C::Damaged, Holder::MarksPage, OPEN, DB),
    (C::Damaged, Holder::MarksPage, RUN, DB),
    (C::Damaged, Holder::DamageList, OPEN, DB),
    (C::Damaged, Holder::DamageList, RUN, DB),
    (C::Damaged, Holder::LsnMap, OPEN, DB),
    (C::Damaged, Holder::LsnMap, RUN, DB),
    (C::Damaged, Holder::Log, OPEN, NO),
    (C::Damaged, Holder::Log, RUN, NO),
    (C::Damaged, Holder::Retired, OPEN, NO),
    (C::Damaged, Holder::Retired, RUN, NO),
    (C::Damaged, Holder::Free, OPEN, NO),
    (C::Damaged, Holder::Free, RUN, NO),
    (C::Damaged, Holder::Unreached, OPEN, NO),
    (C::Damaged, Holder::Unreached, RUN, DB),
    (C::Damaged, Holder::Met, OPEN, DB),
    (C::Damaged, Holder::Met, RUN, DB),
    (C::Unread, H0, OPEN, NO),
    (C::Unread, H0, RUN, NO),
    (C::Unread, H1, OPEN, NO),
    (C::Unread, H1, RUN, NO),
    (C::Unread, I0, OPEN, NO),
    (C::Unread, I0, RUN, NO),
    (C::Unread, I1, OPEN, NO),
    (C::Unread, I1, RUN, NO),
    (C::Unread, Holder::LargeStore, OPEN, NO),
    (C::Unread, Holder::LargeStore, RUN, NO),
    (C::Unread, Holder::CatalogRows, OPEN, NO),
    (C::Unread, Holder::CatalogRows, RUN, NO),
    (C::Unread, Holder::CatalogRoot, OPEN, NO),
    (C::Unread, Holder::CatalogRoot, RUN, NO),
    (C::Unread, Holder::CommitLog, OPEN, NO),
    (C::Unread, Holder::CommitLog, RUN, NO),
    (C::Unread, Holder::Counters, OPEN, NO),
    (C::Unread, Holder::Counters, RUN, NO),
    (C::Unread, Holder::CommitTimes, OPEN, NO),
    (C::Unread, Holder::CommitTimes, RUN, NO),
    (C::Unread, Holder::Bitmap, OPEN, NO),
    (C::Unread, Holder::Bitmap, RUN, NO),
    (C::Unread, Holder::MarksPage, OPEN, NO),
    (C::Unread, Holder::MarksPage, RUN, NO),
    (C::Unread, Holder::DamageList, OPEN, HELD),
    (C::Unread, Holder::DamageList, RUN, HELD),
    (C::Unread, Holder::LsnMap, OPEN, NO),
    (C::Unread, Holder::LsnMap, RUN, NO),
    (C::Unread, Holder::Log, OPEN, NO),
    (C::Unread, Holder::Log, RUN, NO),
    (C::Unread, Holder::Retired, OPEN, NO),
    (C::Unread, Holder::Retired, RUN, NO),
    (C::Unread, Holder::Free, OPEN, NO),
    (C::Unread, Holder::Free, RUN, NO),
    (C::Unread, Holder::Unreached, OPEN, NO),
    (C::Unread, Holder::Unreached, RUN, NO),
    (C::Settled, H0, OPEN, NO),
    (C::Settled, H0, RUN, NO),
    (C::Settled, H1, OPEN, NO),
    (C::Settled, H1, RUN, NO),
    (C::Settled, I0, OPEN, NO),
    (C::Settled, I0, RUN, NO),
    (C::Settled, I1, OPEN, NO),
    (C::Settled, I1, RUN, NO),
    (C::Settled, Holder::LargeStore, OPEN, NO),
    (C::Settled, Holder::LargeStore, RUN, NO),
    (C::Settled, Holder::CatalogRows, OPEN, NO),
    (C::Settled, Holder::CatalogRows, RUN, NO),
    (C::Settled, Holder::CatalogRoot, OPEN, NO),
    (C::Settled, Holder::CatalogRoot, RUN, NO),
    (C::Settled, Holder::CommitLog, OPEN, NO),
    (C::Settled, Holder::CommitLog, RUN, NO),
    (C::Settled, Holder::Counters, OPEN, NO),
    (C::Settled, Holder::Counters, RUN, NO),
    (C::Settled, Holder::CommitTimes, OPEN, NO),
    (C::Settled, Holder::CommitTimes, RUN, NO),
    (C::Settled, Holder::Bitmap, OPEN, NO),
    (C::Settled, Holder::Bitmap, RUN, NO),
    (C::Settled, Holder::MarksPage, OPEN, NO),
    (C::Settled, Holder::MarksPage, RUN, NO),
    (C::Settled, Holder::DamageList, OPEN, NO),
    (C::Settled, Holder::DamageList, RUN, NO),
    (C::Settled, Holder::LsnMap, OPEN, NO),
    (C::Settled, Holder::LsnMap, RUN, NO),
    (C::Settled, Holder::Log, OPEN, NO),
    (C::Settled, Holder::Log, RUN, NO),
    (C::Settled, Holder::Retired, OPEN, NO),
    (C::Settled, Holder::Retired, RUN, NO),
    (C::Settled, Holder::Free, OPEN, NO),
    (C::Settled, Holder::Free, RUN, NO),
    (C::Settled, Holder::Unreached, OPEN, NO),
    (C::Settled, Holder::Unreached, RUN, NO),
    (C::Settled, Holder::Met, OPEN, NO),
    (C::Settled, Holder::Met, RUN, NO),
    (C::Full, H0, OPEN, DB),
    (C::Full, H0, RUN, DB),
    (C::Full, H1, OPEN, DB),
    (C::Full, H1, RUN, DB),
    (C::Full, I0, OPEN, DB),
    (C::Full, I0, RUN, DB),
    (C::Full, I1, OPEN, DB),
    (C::Full, I1, RUN, DB),
    (C::Full, Holder::LargeStore, OPEN, DB),
    (C::Full, Holder::LargeStore, RUN, DB),
    (C::Full, Holder::CatalogRows, OPEN, DB),
    (C::Full, Holder::CatalogRows, RUN, DB),
    (C::Full, Holder::CatalogRoot, OPEN, DB),
    (C::Full, Holder::CatalogRoot, RUN, DB),
    (C::Full, Holder::CommitLog, OPEN, DB),
    (C::Full, Holder::CommitLog, RUN, DB),
    (C::Full, Holder::Counters, OPEN, DB),
    (C::Full, Holder::Counters, RUN, DB),
    (C::Full, Holder::CommitTimes, OPEN, DB),
    (C::Full, Holder::CommitTimes, RUN, DB),
    (C::Full, Holder::Bitmap, OPEN, DB),
    (C::Full, Holder::Bitmap, RUN, DB),
    (C::Full, Holder::MarksPage, OPEN, DB),
    (C::Full, Holder::MarksPage, RUN, DB),
    (C::Full, Holder::DamageList, OPEN, DB),
    (C::Full, Holder::DamageList, RUN, DB),
    (C::Full, Holder::LsnMap, OPEN, DB),
    (C::Full, Holder::LsnMap, RUN, DB),
    (C::Full, Holder::Log, OPEN, DB),
    (C::Full, Holder::Log, RUN, DB),
    (C::Full, Holder::Retired, OPEN, DB),
    (C::Full, Holder::Retired, RUN, DB),
    (C::Full, Holder::Free, OPEN, DB),
    (C::Full, Holder::Free, RUN, DB),
    (C::Full, Holder::Unreached, OPEN, DB),
    (C::Full, Holder::Unreached, RUN, DB),
    (C::Full, Holder::Met, OPEN, DB),
    (C::Full, Holder::Met, RUN, DB),
    (C::Unarmed, H0, OPEN, HELD),
    (C::Unarmed, H0, RUN, HELD),
    (C::Unarmed, H1, OPEN, HELD),
    (C::Unarmed, H1, RUN, HELD),
    (C::Unarmed, I0, OPEN, HELD),
    (C::Unarmed, I0, RUN, HELD),
    (C::Unarmed, I1, OPEN, HELD),
    (C::Unarmed, I1, RUN, HELD),
    (C::Unarmed, Holder::LargeStore, OPEN, HELD),
    (C::Unarmed, Holder::LargeStore, RUN, HELD),
    (C::Unarmed, Holder::CatalogRows, OPEN, HELD),
    (C::Unarmed, Holder::CatalogRows, RUN, HELD),
    (C::Unarmed, Holder::CatalogRoot, OPEN, HELD),
    (C::Unarmed, Holder::CatalogRoot, RUN, HELD),
    (C::Unarmed, Holder::CommitLog, OPEN, HELD),
    (C::Unarmed, Holder::CommitLog, RUN, HELD),
    (C::Unarmed, Holder::Counters, OPEN, HELD),
    (C::Unarmed, Holder::Counters, RUN, HELD),
    (C::Unarmed, Holder::CommitTimes, OPEN, HELD),
    (C::Unarmed, Holder::CommitTimes, RUN, HELD),
    (C::Unarmed, Holder::Bitmap, OPEN, HELD),
    (C::Unarmed, Holder::Bitmap, RUN, HELD),
    (C::Unarmed, Holder::MarksPage, OPEN, HELD),
    (C::Unarmed, Holder::MarksPage, RUN, HELD),
    (C::Unarmed, Holder::DamageList, OPEN, HELD),
    (C::Unarmed, Holder::DamageList, RUN, HELD),
    (C::Unarmed, Holder::LsnMap, OPEN, HELD),
    (C::Unarmed, Holder::LsnMap, RUN, HELD),
    (C::Unarmed, Holder::Log, OPEN, HELD),
    (C::Unarmed, Holder::Log, RUN, HELD),
    (C::Unarmed, Holder::Retired, OPEN, HELD),
    (C::Unarmed, Holder::Retired, RUN, HELD),
    (C::Unarmed, Holder::Free, OPEN, HELD),
    (C::Unarmed, Holder::Free, RUN, HELD),
    (C::Unarmed, Holder::Unreached, OPEN, HELD),
    (C::Unarmed, Holder::Unreached, RUN, HELD),
    (C::Unarmed, Holder::Met, OPEN, HELD),
    (C::Unarmed, Holder::Met, RUN, HELD),
];

/// The rows whose mark today differs from the one stated, with today's mark. Rows may only be
/// removed from it.
const DIFFERS_TODAY: [(C, Holder, Phase, Mark); 20] = [
    (C::Unread, H0, OPEN, OH),
    (C::Unread, H0, RUN, OH),
    (C::Unread, H1, OPEN, DB),
    (C::Unread, H1, RUN, DB),
    (C::Unread, I0, OPEN, OI),
    (C::Unread, I0, RUN, OI),
    (C::Unread, I1, OPEN, DB),
    (C::Unread, I1, RUN, DB),
    (C::Unread, Holder::LargeStore, OPEN, DB),
    (C::Unread, Holder::LargeStore, RUN, DB),
    (C::Unread, Holder::CatalogRows, OPEN, DB),
    (C::Unread, Holder::CatalogRoot, OPEN, DB),
    (C::Unread, Holder::CatalogRoot, RUN, DB),
    (C::Unread, Holder::CommitLog, OPEN, DB),
    (C::Unread, Holder::CommitLog, RUN, DB),
    (C::Unread, Holder::Counters, OPEN, DB),
    (C::Unread, Holder::Counters, RUN, DB),
    (C::Unread, Holder::Bitmap, OPEN, DB),
    (C::Unread, Holder::Bitmap, RUN, DB),
    (C::Unread, Holder::Unreached, RUN, DB),
];

/// Returns false for an unreadable page as met, which no mapping meets.
fn reachable(class: C, holder: Holder) -> bool {
    (class, holder) != (C::Unread, Holder::Met)
}

/// Returns the index of the one row of `rows` for `class`, `holder` and `phase`.
fn row_of(
    rows: &[(C, Holder, Phase, Mark)],
    class: C,
    holder: Holder,
    phase: Phase,
) -> Option<usize> {
    let mut found = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| (r.0, r.1, r.2) == (class, holder, phase))
        .map(|(i, _)| i);
    let first = found.next();
    assert_eq!(
        None,
        found.next(),
        "two rows for {class:?} {holder:?} {phase:?}"
    );
    first
}

#[test]
fn every_fault_and_holder_marks_as_storage_md_states() {
    let mut visits = [0; STATED.len()];
    let mut differs_visits = [0; DIFFERS_TODAY.len()];
    let mut differ = Vec::new();
    for class in C::ALL {
        for holder in HOLDERS.into_iter().filter(|h| reachable(class, *h)) {
            for phase in [OPEN, RUN] {
                let at = row_of(&STATED, class, holder, phase);
                let at = at.unwrap_or_else(|| panic!("no row for {class:?} {holder:?} {phase:?}"));
                visits[at] += 1;
                let stated = STATED[at].3;
                let today = row_of(&DIFFERS_TODAY, class, holder, phase);
                if let Some(t) = today {
                    differs_visits[t] += 1;
                    let row = DIFFERS_TODAY[t];
                    assert_ne!(
                        stated, row.3,
                        "{row:?} now matches what is stated: remove it"
                    );
                }
                let want = today.map_or(stated, |t| DIFFERS_TODAY[t].3);
                for fault in class.faults() {
                    let got = mark(fault, holder, phase);
                    if want != got {
                        differ.push(format!(
                            "{fault:?} {holder:?} {phase:?}: want {want:?}, marks {got:?}"
                        ));
                    }
                }
            }
        }
    }
    assert_eq!([1; STATED.len()], visits, "each stated row is visited once");
    assert_eq!(
        [1; DIFFERS_TODAY.len()],
        differs_visits,
        "each differing row is visited once"
    );
    assert!(differ.is_empty(), "{}", differ.join("\n"));
}
