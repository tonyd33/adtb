use super::{Fault, Holder, Kind, Mark, Phase, mark};
use crate::storage::{Damage, Lsn, Quarantine, Shape};
use adtb_testkit::marks::{self, Class, Held, ROWS, Stated};

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

const CLASSES: [Class; 7] = adtb_testkit::every![Class;
    Class::Damaged,
    Class::Unproven,
    Class::Unread,
    Class::Settled,
    Class::Full,
    Class::Unarmed,
    Class::Finding,
];

fn faults(class: Class) -> Vec<Fault> {
    let (expected, found) = (Lsn(9), Lsn(5));
    match class {
        Class::Damaged => [
            Damage::Checksum,
            Damage::Stale { expected, found },
            Damage::Shape(Shape::Blank),
            Damage::Shape(Shape::Kind),
            Damage::Shape(Shape::Owner),
            Damage::Shape(Shape::Frame),
            Damage::Shape(Shape::Format),
        ]
        .map(Fault::Page)
        .to_vec(),
        Class::Unproven => vec![Fault::Page(Damage::LogUnproven { at: expected })],
        Class::Unread => vec![Fault::Page(Damage::Unreadable)],
        Class::Settled => vec![Fault::Page(Damage::Settled { expected, found })],
        Class::Full => vec![Fault::Full],
        Class::Unarmed => vec![Fault::Unarmed],
        Class::Finding => vec![Fault::Finding],
    }
}

/// Returns the holders of a page of `held`: a heap and an index for a relation's.
fn holders(held: Held) -> Vec<Holder> {
    match held {
        Held::Relation => vec![H0, I0],
        Held::EngineRelation => vec![H1, I1],
        Held::LargeStore => vec![Holder::LargeStore],
        Held::CatalogRoot => vec![Holder::CatalogRoot],
        Held::CatalogRows => vec![Holder::CatalogRows],
        Held::Counters => vec![Holder::Counters],
        Held::CommitLog => vec![Holder::CommitLog],
        Held::CommitTimes => vec![Holder::CommitTimes],
        Held::Bitmap => vec![Holder::Bitmap],
        Held::MarksPage => vec![Holder::MarksPage],
        Held::DamageList => vec![Holder::DamageList],
        Held::LsnMap => vec![Holder::LsnMap],
        Held::Log => vec![Holder::Log],
        Held::Retired => vec![Holder::Retired],
        Held::Free => vec![Holder::Free],
        Held::Unreached => vec![Holder::Unreached],
    }
}

fn held(holder: Holder) -> Held {
    match holder {
        Holder::Relation {
            engine_module: false,
            ..
        } => Held::Relation,
        Holder::Relation {
            engine_module: true,
            ..
        } => Held::EngineRelation,
        Holder::LargeStore => Held::LargeStore,
        Holder::CatalogRows => Held::CatalogRows,
        Holder::CatalogRoot => Held::CatalogRoot,
        Holder::CommitLog => Held::CommitLog,
        Holder::Counters => Held::Counters,
        Holder::CommitTimes => Held::CommitTimes,
        Holder::Bitmap => Held::Bitmap,
        Holder::MarksPage => Held::MarksPage,
        Holder::DamageList => Held::DamageList,
        Holder::LsnMap => Held::LsnMap,
        Holder::Log => Held::Log,
        Holder::Retired => Held::Retired,
        Holder::Free => Held::Free,
        Holder::Unreached => Held::Unreached,
    }
}

fn phase(phase: Phase) -> marks::Phase {
    match phase {
        Phase::Opening => marks::Phase::Opening,
        Phase::Running => marks::Phase::Running,
    }
}

/// Returns the mark `stated` means for a fault on a page of `holder`.
fn meant(stated: Stated, holder: Holder) -> Mark {
    match (stated, holder) {
        (Stated::Nothing, _) => Mark::NOTHING,
        (Stated::Own, Holder::Relation { object, .. }) => Mark(Kind::Object(object)),
        (Stated::Own, _) => panic!("only a relation's page has its own object: {holder:?}"),
        (Stated::Database, _) => Mark::DATABASE,
        (Stated::Open, _) => Mark::HELD,
    }
}

/// Returns false for a finding on anything but an index, which no finding names, and for an
/// unproven status page anywhere but on the commit log at open.
fn reachable(class: Class, holder: Holder, phase: Phase) -> bool {
    match class {
        Class::Finding => [I0, I1].contains(&holder),
        Class::Unproven => holder == Holder::CommitLog && phase == Phase::Opening,
        _ => true,
    }
}

#[test]
fn every_fault_and_holder_marks_as_storage_md_states() {
    let mut visits = [0; ROWS.len()];
    let mut differ = Vec::new();
    for class in CLASSES {
        for holder in Held::ALL.into_iter().flat_map(holders) {
            for p in [Phase::Opening, Phase::Running]
                .into_iter()
                .filter(|p| reachable(class, holder, *p))
            {
                let at = marks::row(class, held(holder), phase(p));
                let at = at.unwrap_or_else(|| panic!("no row for {class:?} {holder:?} {p:?}"));
                visits[at] += 1;
                let want = meant(ROWS[at].3, holder);
                for fault in faults(class) {
                    let got = mark(fault, holder, p);
                    if want != got {
                        differ.push(format!(
                            "{fault:?} {holder:?} {p:?}: want {want:?}, marks {got:?}"
                        ));
                    }
                }
            }
        }
    }
    let unvisited: Vec<usize> = (0..ROWS.len()).filter(|&i| visits[i] == 0).collect();
    assert_eq!(Vec::<usize>::new(), unvisited, "each stated row is visited");
    assert!(differ.is_empty(), "{}", differ.join("\n"));
}
