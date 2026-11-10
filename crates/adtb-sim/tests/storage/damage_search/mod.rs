//! The search for damage a consumer misses: generated sequences over every kind of page that
//! holds data or evidence, each fault at each write-out, then every consumer of the file.

mod case;
mod consumers;
mod generate;
mod judge;
mod stated;
mod world;

use std::collections::{BTreeMap, BTreeSet};

use adtb_testkit::profile::{self, assert_reached, every_case};
use case::{FAULTS, Fault, Injection, Point};
use consumers::{CONSUMERS, Consumer};
use judge::{Row, Violation};
use stated::{Class, Held, PHASES, Phase};
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
const UNDER_MARK: &str = "migrate-under-mark-silent";
const APPLIED_UNDER_MARK: &str = "migrate-applies-under-mark";

/// The failures main shows. When a change moves the write-outs of the generated cases, run
/// [`nearest_pins`] for the nearest write-out that shows each entry whose pin no longer does.
const KNOWN: &[Known] = &[
    Known {
        fault: Fault::LostEarlier,
        kind: Kind::Heap,
        owner: Some(world::Rel::Tags),
        consumer: Consumer::Settle,
        what: "the open fails",
        ticket: BLANK_OPEN,
        seed: 6,
        at: 690,
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
        at: 648,
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
        at: 648,
        then: Some((
            Point {
                at: 649,
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
        at: 648,
        then: Some((
            Point {
                at: 649,
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
        at: 604,
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
        at: 634,
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
        at: 645,
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
        at: 605,
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
        at: 622,
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
        at: 632,
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
        at: 640,
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
        at: 605,
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
        at: 609,
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
        at: 632,
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
        kind: Kind::Large,
        owner: None,
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 26,
        at: 626,
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
        at: 640,
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
        at: 632,
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
        at: 632,
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
        at: 632,
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
        at: 652,
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
        at: 659,
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
        at: 606,
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
        at: 622,
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
        at: 651,
        then: None,
    },
    Known {
        fault: Fault::LostEarlier,
        kind: Kind::Large,
        owner: None,
        consumer: Consumer::Catalog,
        what: "a migration under a whole-database mark returns Ok",
        ticket: UNDER_MARK,
        seed: 58,
        at: 623,
        then: None,
    },
    Known {
        fault: Fault::LostEarlier,
        kind: Kind::Large,
        owner: None,
        consumer: Consumer::Catalog,
        what: "applies a migration under a whole-database mark",
        ticket: APPLIED_UNDER_MARK,
        seed: 58,
        at: 623,
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

/// Returns the classes of damage the table is asked of for a page `fault` hits: none for a tear,
/// which lands whole or is a lost write.
fn classes(fault: Fault) -> &'static [Class] {
    match fault {
        Fault::LostEarlier | Fault::LostCurrent | Fault::Recycled => {
            &[Class::Damaged, Class::Settled]
        }
        Fault::UnreadableOnce | Fault::UnreadableLasting => &[Class::Unread, Class::Unarmed],
        Fault::Torn => &[],
    }
}

const AT_THE_OPEN: &str = "at the damage tier the consumers' open records every stale page of \
                           the structure, so its mapping is surely the open's";

/// Why no case of a fault asks a row of the stated table.
#[derive(Clone, Copy, Debug)]
enum Unasked {
    /// No case can: the rule is in the reason.
    Never(&'static str),
    /// No case at the damage tier did: a deeper tier may.
    AtTier(&'static str),
}

impl Unasked {
    fn reason(self) -> &'static str {
        match self {
            Unasked::Never(why) | Unasked::AtTier(why) => why,
        }
    }
}

/// Returns why no case of `fault` asks `row`, `None` for a row its cases must ask.
fn unasked(fault: Fault, row: Row) -> Option<Unasked> {
    use Phase::{Opening, Running};
    let (class, held, phase) = row;
    match (class, held) {
        (Class::Settled, h) if h != Held::CommitLog => {
            return Some(Unasked::Never("only a commit log status page settles"));
        }
        (Class::Unarmed, h) if h != Held::LsnMap => {
            return Some(Unasked::Never(
                "only an LSN map page the open's recovery cannot read leaves it unarmed",
            ));
        }
        (_, Held::Retired) => {
            return Some(Unasked::Never("no operation of a sequence swaps a tree"));
        }
        (_, Held::Free) => {
            return Some(Unasked::Never(
                "the search judges a page by the holder its write-out named, never as free",
            ));
        }
        (_, Held::Unreached) => {
            return Some(Unasked::Never(
                "the search classifies every page by a listed holder, and a page it cannot \
                 classify fails the case",
            ));
        }
        _ => {}
    }
    let lost = matches!(fault, Fault::LostCurrent | Fault::Recycled);
    match (fault, class, held, phase) {
        (Fault::LostEarlier, _, Held::CommitLog, _) => Some(Unasked::Never(
            "truncation frees only a status page wholly below its bound, and one page holds the \
             status of 32 thousand ids while a sequence assigns some thousands",
        )),
        (
            Fault::LostEarlier,
            _,
            Held::EngineRelation
            | Held::CatalogRoot
            | Held::CatalogRows
            | Held::Counters
            | Held::CommitTimes,
            _,
        ) => Some(Unasked::Never(
            "no sequence frees a page of the catalog or of an engine structure",
        )),
        (
            Fault::LostEarlier,
            _,
            Held::Bitmap | Held::MarksPage | Held::DamageList | Held::LsnMap,
            _,
        ) => Some(Unasked::Never(
            "a lost write of an earlier incarnation hits only a kind the engine's code frees",
        )),
        (Fault::LostEarlier, _, Held::Relation | Held::LargeStore, Running) => {
            Some(Unasked::AtTier(
                "at the damage tier only the consumers' open meets a stale page of an earlier \
             incarnation",
            ))
        }
        (
            _,
            Class::Damaged,
            Held::CatalogRoot
            | Held::CatalogRows
            | Held::Counters
            | Held::CommitLog
            | Held::CommitTimes
            | Held::Bitmap
            | Held::MarksPage
            | Held::DamageList
            | Held::LsnMap,
            Running,
        ) if lost => Some(Unasked::AtTier(AT_THE_OPEN)),
        (_, Class::Damaged, Held::EngineRelation, Running) if lost => {
            Some(Unasked::AtTier(AT_THE_OPEN))
        }
        (_, Class::Settled, Held::CommitLog, _) if lost => Some(Unasked::AtTier(
            "at the damage tier no case settles a lost status page from the log",
        )),
        (Fault::UnreadableOnce, Class::Unarmed, Held::LsnMap, _) => Some(Unasked::Never(
            "whether the open met the page is observed only for a failure that lasts",
        )),
        (Fault::UnreadableOnce, Class::Unread, _, Opening) => Some(Unasked::AtTier(
            "at the damage tier a read failure that does not last leaves no record the open holds",
        )),
        (Fault::UnreadableLasting, Class::Unarmed, Held::LsnMap, Running) => Some(Unasked::Never(
            "an LSN map page unreadable for good is met by the open's recovery",
        )),
        (Fault::UnreadableLasting, Class::Unread, Held::LsnMap, _) => Some(Unasked::Never(
            "the open's recovery reads every LSN map page, so it meets one unreadable for good",
        )),
        (
            Fault::UnreadableLasting,
            Class::Unread,
            Held::CatalogRoot
            | Held::CatalogRows
            | Held::Counters
            | Held::CommitLog
            | Held::CommitTimes
            | Held::MarksPage,
            _,
        ) => Some(Unasked::AtTier(
            "at the damage tier no case with this page unreadable for good observes marks after \
             an open that read it",
        )),
        (Fault::UnreadableLasting, Class::Unread, Held::EngineRelation, Opening)
        | (Fault::UnreadableLasting, Class::Unread, Held::Bitmap | Held::DamageList, Running) => {
            Some(Unasked::AtTier(
                "at the damage tier no case asks this phase of the page unreadable for good",
            ))
        }
        _ => None,
    }
}

/// Returns every row of the stated table for the classes of `fault`.
fn rows_of(fault: Fault) -> impl Iterator<Item = Row> {
    classes(fault)
        .iter()
        .flat_map(|c| Held::ALL.map(|h| (*c, h)))
        .flat_map(|(c, h)| PHASES.map(|p| (c, h, p)))
}

/// Returns the rows the cases of `fault` must ask that `s` has not asked.
fn rows_due(fault: Fault, s: &Searched) -> Vec<Row> {
    rows_of(fault)
        .filter(|r| unasked(fault, *r).is_none())
        .filter(|r| !s.rows.contains(r))
        .collect()
}

/// Fail unless the cases of `fault` asked every row of the stated table for its classes but
/// those [`unasked`] lists, and none it lists as never asked.
fn assert_rows_asked(fault: Fault, s: &Searched, text: &str) {
    let missed = rows_due(fault, s);
    assert_eq!(
        Vec::<Row>::new(),
        missed,
        "{fault:?}: rows of the stated table no case asked\n{text}"
    );
    let came: Vec<Row> = rows_of(fault)
        .filter(|r| matches!(unasked(fault, *r), Some(Unasked::Never(_))))
        .filter(|r| s.rows.contains(r))
        .collect();
    assert_eq!(
        Vec::<Row>::new(),
        came,
        "{fault:?}: rows no case can ask were asked; drop their entries\n{text}"
    );
}

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
    if fault == Fault::UnreadableLasting {
        assert_reached(
            "a lasting unreadable page failing reads after a restart",
            found.failed_after_restart,
            1,
        );
    }
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
    /// The rows of the stated table the cases asked.
    rows: BTreeSet<Row>,
    /// The cases of each kind run on a write-out picked for a row still to ask, apart from
    /// `cases`.
    steered: BTreeMap<Kind, usize>,
    /// The write-outs picked for each holder of a row still to ask.
    steer_tries: BTreeMap<Held, usize>,
    /// The cases of each kind whose consumers' open failed.
    failed_opens: BTreeMap<Kind, usize>,
    found: Vec<Found>,
    /// The cases a vacuum on a second thread ran during a write of.
    overlapped: usize,
    /// The cases whose planned read failures struck again after a restart.
    failed_after_restart: usize,
}

impl Searched {
    /// Record what the case of `kind` at write-out `at` of `plan`, of sequence `seed`, reached and
    /// found, ending in `outcome`.
    fn tally(
        &mut self,
        kind: Kind,
        plan: &case::Plan,
        seed: u64,
        at: usize,
        outcome: case::Outcome,
    ) {
        *self.endings.entry(outcome.verdicts.ending).or_default() += 1;
        if !outcome.opened {
            *self.failed_opens.entry(kind).or_default() += 1;
        }
        if outcome.overlaps > 0 {
            self.overlapped += 1;
        }
        if outcome.failed_after_restart {
            self.failed_after_restart += 1;
        }
        self.reached
            .extend(outcome.verdicts.reached.iter().map(|c| (kind, *c)));
        self.rows.extend(outcome.verdicts.rows.iter().copied());
        for violation in outcome.verdicts.violations {
            self.found.push(Found {
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

/// Returns, for each holder of the rows `s` has still to ask and has picked fewer than `most`
/// write-outs for, the first write-out of `plan` a fault of `fault` hits on a page of that
/// holder, with the holder and the page's kind.
fn steer(fault: Fault, plan: &case::Plan, s: &Searched, most: usize) -> Vec<(Held, Kind, usize)> {
    let due: BTreeSet<Held> = rows_due(fault, s)
        .iter()
        .map(|r| r.1)
        .filter(|h| s.steer_tries.get(h).copied().unwrap_or(0) < most)
        .collect();
    let ids = &plan.observed.ids;
    let mut out: BTreeMap<Held, (Kind, usize)> = BTreeMap::new();
    for (at, w) in plan.clean.trace.writes.iter().enumerate() {
        let Some(kind) = plan.kind(at).filter(|k| s.kinds.contains(k)) else {
            continue;
        };
        let Some((held, _)) = ids.holding(w.kind, w.owner, w.pid) else {
            continue;
        };
        if due.contains(&held) && plan.fault(at, fault.injection()) == fault {
            out.entry(held).or_insert((kind, at));
        }
    }
    out.into_iter().map(|(h, (k, at))| (h, k, at)).collect()
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
        rows: BTreeSet::new(),
        steered: BTreeMap::new(),
        steer_tries: BTreeMap::new(),
        failed_opens: BTreeMap::new(),
        found: Vec::new(),
        overlapped: 0,
        failed_after_restart: 0,
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
        let late = seed > tier.sequences;
        if kinds.iter().all(|k| done(*k, &s)) && (late || rows_due(fault, &s).is_empty()) {
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
                if let Some(outcome) = case::run(&plan, Point { at, injection }) {
                    *s.cases.entry(kind).or_default() += 1;
                    s.tally(kind, &plan, seed, at, outcome);
                }
            }
        }
        if !concurrent && !late {
            for (held, kind, at) in steer(fault, &plan, &s, tier.per_kind) {
                *s.steer_tries.entry(held).or_default() += 1;
                let injection = fault.injection();
                if let Some(outcome) = case::run(&plan, Point { at, injection }) {
                    *s.steered.entry(kind).or_default() += 1;
                    s.tally(kind, &plan, seed, at, outcome);
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
        "{fault:?}: cases {:?}, steered {:?}, endings {:?}, failed opens {:?}, stated rows asked {} \
         of {}\n",
        s.cases,
        s.steered,
        s.endings,
        s.failed_opens,
        s.rows.len(),
        rows_of(fault).count()
    );
    text += &format!("{fault:?} asked {:?}\n", s.rows);
    for row in rows_of(fault) {
        if let Some(u) = unasked(fault, row) {
            text += &format!("{fault:?} unasked {row:?}: {}\n", u.reason());
        }
    }
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
        let cases = s.cases.get(k).copied().unwrap_or(0) + s.steered.get(k).copied().unwrap_or(0);
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
    assert_rows_asked(fault, s, &text);
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
    let pinned = KNOWN
        .iter()
        .find(|k| k.ticket == BLANK_OPEN && k.consumer == Consumer::Settle)
        .expect("the failed open's entry is known");
    rerun(pinned);
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

/// Returns why the case `k` pins no longer shows `k`, or `None` when it does.
fn unpinned(k: &Known) -> Option<String> {
    let plan = case::plan(k.seed, generate::sequence(k.seed), false);
    let writes = plan.clean.trace.writes.len();
    let then = k.then.map(|(t, _)| t);
    if k.at >= writes {
        return Some("names a write-out past the last".into());
    }
    if plan.kind(k.at) != Some(k.kind) {
        return Some(format!("writes another cell: kind {:?}", plan.kind(k.at)));
    }
    let cell = (plan.owner(k.at), plan.fault(k.at, k.fault.injection()));
    if cell != (k.owner, k.fault) {
        return Some(format!("writes another cell: {cell:?}"));
    }
    if let Some((t, kind)) = k.then
        && (t.at >= writes || plan.kind(t.at) != Some(kind))
    {
        return Some("second fault hits another kind".into());
    }
    (!shows(k, &plan, k.at, then)).then(|| "no longer shows".into())
}

/// Returns whether the case of `plan` faulted at `at`, and at `then` for a pair, shows `k`'s
/// cell and shape.
fn shows(k: &Known, plan: &case::Plan, at: usize, then: Option<Point>) -> bool {
    let injection = k.fault.injection();
    if plan.kind(at) != Some(k.kind)
        || (plan.owner(at), plan.fault(at, injection)) != (k.owner, k.fault)
    {
        return false;
    }
    let first = Point { at, injection };
    let outcome = match then {
        Some(t) => case::run_pair(plan, first, t),
        None => case::run(plan, first),
    };
    outcome.is_some_and(|o| {
        o.verdicts
            .violations
            .iter()
            .any(|v| v.consumer == k.consumer && v.what == k.what)
    })
}

/// Returns the nearest write-out to `k`'s pin, in its seed first, whose case shows `k`, with the
/// second fault's write-out for a pair.
fn nearest(k: &Known) -> Option<(u64, usize, Option<usize>)> {
    let steered = generate::STEERED..generate::STEERED + 60;
    for seed in std::iter::once(k.seed).chain(1..=60).chain(steered) {
        let plan = case::plan(seed, generate::sequence(seed), false);
        let writes = plan.clean.trace.writes.len();
        let mut ats: Vec<usize> = (0..writes)
            .filter(|&at| plan.kind(at) == Some(k.kind))
            .collect();
        ats.sort_by_key(|at| at.abs_diff(k.at));
        for &at in ats.iter().take(12) {
            let Some((t, kind)) = k.then else {
                if shows(k, &plan, at, None) {
                    return Some((seed, at, None));
                }
                continue;
            };
            let mut seconds: Vec<usize> = (0..writes)
                .filter(|&x| plan.kind(x) == Some(kind))
                .collect();
            seconds.sort_by_key(|x| x.abs_diff(t.at));
            for &second in seconds.iter().take(6) {
                let then = Point {
                    at: second,
                    injection: t.injection,
                };
                if shows(k, &plan, at, Some(then)) {
                    return Some((seed, at, Some(second)));
                }
            }
        }
    }
    None
}

/// Print each known failure its pin no longer shows, why, and the nearest write-out that shows
/// it. Run it by hand under the damage profile:
/// `cargo nextest run -p adtb-sim --test storage --profile damage --run-ignored only --no-capture -E 'test(nearest_pins)'`.
#[test]
#[ignore = "a search for new pins, run by hand after a change moves the write-outs"]
fn nearest_pins() {
    for (i, k) in KNOWN.iter().enumerate() {
        let Some(why) = unpinned(k) else {
            continue;
        };
        eprintln!(
            "nearest_pins: {i} {:?} {:?} {:?} {:?} {:?}: seed {} at {} then {:?}: {why}; nearest {:?}",
            k.fault,
            k.kind,
            k.owner,
            k.consumer,
            k.what,
            k.seed,
            k.at,
            k.then.map(|(t, _)| t.at),
            nearest(k)
        );
    }
}
