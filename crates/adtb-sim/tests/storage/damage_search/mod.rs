//! The search for damage a consumer misses: generated sequences over every kind of page that
//! holds data or evidence, each fault at each write-out, then every consumer of the file.

mod case;
mod consumers;
mod generate;
mod judge;
mod ledger;
mod plans;
mod pool;
mod stated;
mod world;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use adtb_testkit::profile::{assert_reached, every_case};
use case::{FAULTS, FOLDS, Fault, Fold, Injection, Point};
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
    /// The most write-outs a shard steers to the pages of each holder of a row still to ask.
    steers: usize,
    /// Whether a shard runs sequences while a row of the stated table its fault can ask is unasked
    /// or a kind no consumer reaches has fewer cases than `per_kind`, and fails on a row none
    /// asked or, for a lasting unreadable page, when no restart failed its reads again.
    every_row: bool,
}

/// Returns the tier the nextest profile asks: 32 cases of each kind and every row of the stated
/// table under `deep`, else the per-change tier's one case of each kind.
fn tier() -> Tier {
    if every_case() {
        Tier {
            sequences: 160,
            per_kind: 32,
            steers: 32,
            every_row: true,
        }
    } else {
        Tier {
            sequences: 60,
            per_kind: 1,
            steers: 0,
            every_row: false,
        }
    }
}

/// Returns the kinds a fault can hit: an earlier incarnation only of a kind the engine's code
/// frees, and a tear of no page whose writes change only the device's first atomic unit, which a
/// tear lands whole or not at all.
fn kinds(fault: Fault) -> Vec<Kind> {
    KINDS
        .into_iter()
        .filter(|k| fault != Fault::LostEarlier || judge::freed(*k))
        .filter(|k| fault != Fault::Torn || !UNTORN.contains(k))
        .collect()
}

/// The kinds whose every write changes only the first atomic unit of its page.
const UNTORN: [Kind; 5] = [
    Kind::Directory,
    Kind::VisibilityMap,
    Kind::DamageList,
    Kind::Bitmap,
    Kind::Marks,
];

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
    /// The second fault of a pair and the kind of page its write-out writes.
    then: Option<(Point, Kind)>,
}

const BLANK_OPEN: &str = "freed-lost-write-fails-open";
const UNDER_RELATION_MARK: &str = "migrate-under-relation-mark";
const APPLIES_UNDER_MARK: &str = "migrate-applies-under-mark";
/// The failures main shows. When a change moves the write-outs of the generated cases, run
/// [`nearest_pins`] for the nearest write-out that shows each entry whose pin no longer does.
const KNOWN: &[Known] = &[
    Known {
        fault: Fault::LostCurrent,
        kind: Kind::Engine,
        owner: None,
        consumer: Consumer::Catalog,
        what: "applies a migration under a whole-database mark",
        ticket: APPLIES_UNDER_MARK,
        seed: 3,
        at: 638,
        then: None,
    },
    Known {
        fault: Fault::Recycled,
        kind: Kind::VisibilityMap,
        owner: Some(world::Rel::Tags),
        consumer: Consumer::Verify,
        what: "verify reports nothing of a mark",
        ticket: UNDER_RELATION_MARK,
        seed: 114,
        at: 652,
        then: None,
    },
    Known {
        fault: Fault::LostEarlier,
        kind: Kind::Directory,
        owner: Some(world::Rel::Tags),
        consumer: Consumer::Verify,
        what: "verify reports nothing of a mark",
        ticket: UNDER_RELATION_MARK,
        seed: 3,
        at: 650,
        then: None,
    },
    Known {
        fault: Fault::LostEarlier,
        kind: Kind::Index,
        owner: Some(world::Rel::Tags),
        consumer: Consumer::Verify,
        what: "verify reports nothing of a mark",
        ticket: UNDER_RELATION_MARK,
        seed: 3,
        at: 639,
        then: None,
    },
    Known {
        fault: Fault::LostEarlier,
        kind: Kind::VisibilityMap,
        owner: Some(world::Rel::Tags),
        consumer: Consumer::Verify,
        what: "verify reports nothing of a mark",
        ticket: UNDER_RELATION_MARK,
        seed: 3,
        at: 646,
        then: None,
    },
    Known {
        fault: Fault::Flipped,
        kind: Kind::VisibilityMap,
        owner: Some(world::Rel::Tags),
        consumer: Consumer::Verify,
        what: "verify reports nothing of a mark",
        ticket: UNDER_RELATION_MARK,
        seed: 3,
        at: 646,
        then: None,
    },
    Known {
        fault: Fault::Flipped,
        kind: Kind::Engine,
        owner: None,
        consumer: Consumer::Catalog,
        what: "applies a migration under a whole-database mark",
        ticket: APPLIES_UNDER_MARK,
        seed: 6,
        at: 606,
        then: None,
    },
    Known {
        fault: Fault::Flipped,
        kind: Kind::Directory,
        owner: Some(world::Rel::Tags),
        consumer: Consumer::Verify,
        what: "verify reports nothing of a mark",
        ticket: UNDER_RELATION_MARK,
        seed: 3,
        at: 650,
        then: None,
    },
    Known {
        fault: Fault::Flipped,
        kind: Kind::Index,
        owner: Some(world::Rel::Tags),
        consumer: Consumer::Verify,
        what: "verify reports nothing of a mark",
        ticket: UNDER_RELATION_MARK,
        seed: 3,
        at: 639,
        then: None,
    },
    Known {
        fault: Fault::Flipped,
        kind: Kind::CommitLog,
        owner: None,
        consumer: Consumer::Catalog,
        what: "applies a migration under a whole-database mark",
        ticket: APPLIES_UNDER_MARK,
        seed: 6,
        at: 677,
        then: None,
    },
    Known {
        fault: Fault::Flipped,
        kind: Kind::Directory,
        owner: Some(world::Rel::Tags),
        consumer: Consumer::Settle,
        what: "the open fails",
        ticket: BLANK_OPEN,
        seed: 3,
        at: 647,
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
        at: 688,
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
        at: 647,
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
        at: 647,
        then: Some((
            Point {
                at: 651,
                injection: Injection::Torn,
            },
            Kind::Engine,
        )),
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
        Fault::Flipped => &[Class::Damaged],
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
    let lost = matches!(fault, Fault::LostCurrent | Fault::Recycled | Fault::Flipped);
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
        (_, Class::Settled, Held::CommitLog, Running) => Some(Unasked::Never(
            "only the open of the commit log settles a status page",
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
        .filter(|(_, h)| *h != Held::Log)
        .flat_map(|(c, h)| PHASES.map(|p| (c, h, p)))
}

/// Returns the rows the cases of `fault` must ask that `s` has not asked.
fn rows_due(fault: Fault, s: &Searched) -> Vec<Row> {
    rows_of(fault)
        .filter(|r| unasked(fault, *r).is_none())
        .filter(|r| !s.rows.contains_key(r))
        .collect()
}

/// Fail if the cases of `fault` asked a row [`unasked`] lists as never asked, or with `every_row`,
/// unless they asked every row of the stated table for its classes but those it lists.
fn assert_rows_asked(fault: Fault, s: &Searched, every_row: bool, text: &str) {
    if every_row {
        let missed = rows_due(fault, s);
        assert_eq!(
            Vec::<Row>::new(),
            missed,
            "{fault:?}: rows of the stated table no case asked\n{text}"
        );
    }
    let came: Vec<Row> = rows_of(fault)
        .filter(|r| matches!(unasked(fault, *r), Some(Unasked::Never(_))))
        .filter(|r| s.rows.contains_key(r))
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

/// Returns whether no consumer of a page of `kind` is ever reached under `fault`.
fn never_reached(fault: Fault, kind: Kind) -> bool {
    UNREACHED
        .iter()
        .any(|(f, k, c, _)| *f == fault && *k == kind && c.is_none())
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
    let found = search(fault, kinds(fault), &tier, false);
    if fault == Fault::Torn {
        let lacking = |of: fn(&(usize, usize)) -> usize| {
            KINDS
                .into_iter()
                .filter(|k| found.tearable.get(k).map_or(0, of) == 0)
                .collect::<Vec<Kind>>()
        };
        assert_eq!(
            Vec::<Kind>::new(),
            lacking(|n| n.0),
            "Torn: kinds no sequence wrote; write-outs and those a tear splits {:?}",
            found.tearable
        );
        assert_eq!(
            UNTORN.to_vec(),
            lacking(|n| n.1),
            "Torn: kinds no write-out of which a tear splits; write-outs and those a tear splits \
             {:?}",
            found.tearable
        );
    }
    if fault == Fault::UnreadableLasting && tier.every_row {
        assert_reached(
            "a lasting unreadable page failing reads after a restart",
            found.failed_after_restart,
            1,
        );
    }
    let text = report(fault, &found, &tier);
    if fault == Fault::Torn {
        return format!(
            "{text}Torn: write-outs and those a tear splits {:?}\n",
            found.tearable
        );
    }
    text
}

/// Rerun every single-fault known failure of `fault`.
fn reruns(fault: Fault) {
    let known: Vec<&Known> = KNOWN
        .iter()
        .filter(|k| k.fault == fault && k.then.is_none())
        .collect();
    pool::map(&known, |k| named(k), |k| rerun(k));
}

/// Returns the case of `k` by its seed and write-outs.
fn named(k: &Known) -> String {
    let then = k
        .then
        .map(|(t, _)| format!(" then {} {:?}", t.at, t.injection));
    format!(
        "{:?} seed {} at {}{}",
        k.fault,
        k.seed,
        k.at,
        then.unwrap_or_default()
    )
}

/// Fail unless the write-out of `k`'s case still has its kind, owner and fault and the case
/// still shows its shape.
fn rerun(k: &Known) {
    let fault = k.fault;
    let plan = plans::plan(k.seed, false);
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
                (Some(kind), true),
                (plan.kind(then.at), plan.takes(then.at, then.injection)),
                "{fault:?} {:?}: seed {} second fault at {} hits another kind or cannot take; pin {} \
                 again",
                k.kind,
                k.seed,
                then.at,
                k.ticket
            );
            case::run_pair(&plan, first, then)
        }
        None => case::run(&plan, first),
    };
    let shown: BTreeSet<(Consumer, &str)> = outcome
        .iter()
        .flat_map(|o| &o.verdicts.violations)
        .map(|v| (v.consumer, v.what.as_str()))
        .collect();
    if k.then.is_some() {
        let pinned: BTreeSet<(Consumer, &str)> = KNOWN
            .iter()
            .filter(|p| (p.fault, p.seed, p.at, p.then) == (fault, k.seed, k.at, k.then))
            .map(|p| (p.consumer, p.what))
            .collect();
        assert_eq!(
            pinned, shown,
            "{fault:?} {:?}: pair seed {} at {} then {:?} shows other violations than its entries \
             of {}",
            k.kind, k.seed, k.at, k.then, k.ticket
        );
    }
    assert!(
        shown.contains(&(k.consumer, k.what)),
        "{fault:?} {:?} {:?}: seed {} at {} no longer shows {:?} of {}; drop the expectation",
        k.kind,
        k.consumer,
        k.seed,
        k.at,
        k.what,
        k.ticket
    );
}

/// What a search of one fault found.
struct Searched {
    kinds: Vec<Kind>,
    cases: BTreeMap<Kind, usize>,
    endings: BTreeMap<judge::Ending, usize>,
    reached: BTreeSet<(Kind, Consumer)>,
    /// The rows of the stated table the cases asked, with the cases that asked each.
    rows: BTreeMap<Row, usize>,
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
    /// For a tear, the write-outs of each kind in the sequences searched, and those a tear splits.
    tearable: BTreeMap<Kind, (usize, usize)>,
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
        for row in &outcome.verdicts.rows {
            *self.rows.entry(*row).or_default() += 1;
        }
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

/// A sequence of a search: its plan, the write-outs of each kind to fault, the outcomes of those
/// run ahead of the tally, and how many of each kind's were run.
struct Sequence {
    plan: Arc<case::Plan>,
    lists: Vec<(Kind, Vec<usize>)>,
    got: Vec<VecDeque<pool::Ran<Option<case::Outcome>>>>,
    asked: Vec<usize>,
}

/// Whether the search has run enough cases of a kind.
type Done<'a> = &'a dyn Fn(Kind, &Searched) -> bool;

/// The sequences of a search planned ahead of its tally, the first the one it tallies.
struct Window<'a> {
    seeds: pool::Seeds<'a>,
    planned: VecDeque<Sequence>,
    /// The panic of the plan of the seed after the last planned, which plans no further seed.
    failed: Option<pool::Panicked>,
    fault: Fault,
    kinds: Vec<Kind>,
    per_kind: usize,
}

impl Window<'_> {
    /// Returns the first sequence planned. Raises the panic of its plan when planning it panicked.
    fn first(&mut self) -> &mut Sequence {
        if self.planned.is_empty()
            && let Some(panicked) = self.failed.take()
        {
            panicked.raise()
        }
        self.planned.front_mut().expect("the seed is planned")
    }

    /// Run `extra` cases of the first sequence, and, ahead of the tally of `s` at kind `k` of the
    /// first sequence, cases of each kind not `done` up to the cases it lacks, then plan more
    /// seeds while few are planned. Returns the outcomes of `extra`.
    ///
    /// Precondition: when `extra` is empty and a sequence is planned, the first has its kind `k`
    /// still to run.
    fn step(&mut self, s: &Searched, k: usize, done: Done<'_>, extra: &[usize]) -> pool::Outcomes {
        let injection = self.fault.injection();
        let lacking = |j: usize| {
            let had = s.cases.get(&self.kinds[j]).copied().unwrap_or(0);
            let ahead: usize = self.planned.iter().map(|q| q.got[j].len()).sum();
            self.per_kind
                .saturating_sub(had)
                .max(1)
                .saturating_sub(ahead)
        };
        let mut budget: Vec<usize> = (0..self.kinds.len()).map(lacking).collect();
        let mut taken: Vec<(usize, usize, usize)> = Vec::new();
        let mut count = extra.len();
        for (n, q) in self.planned.iter().enumerate() {
            for (j, (kind, ats)) in q.lists.iter().enumerate().skip(if n == 0 { k } else { 0 }) {
                if count >= 4 * pool::WIDTH || done(*kind, s) {
                    continue;
                }
                let first = n == 0 && j == k && q.got[j].is_empty();
                let most = budget[j].max(usize::from(first));
                let end = (q.asked[j] + most).min(ats.len());
                let more = end - q.asked[j];
                budget[j] = budget[j].saturating_sub(more);
                count += more;
                taken.push((n, j, more));
            }
        }
        let mut cases: Vec<(&case::Plan, pool::Case)> = Vec::new();
        if let Some(q) = self.planned.front() {
            cases.extend(
                extra
                    .iter()
                    .map(|&at| (&*q.plan, (Point { at, injection }, None))),
            );
        }
        for &(n, j, more) in &taken {
            let q = &self.planned[n];
            let ats = &q.lists[j].1[q.asked[j]..q.asked[j] + more];
            cases.extend(
                ats.iter()
                    .map(|&at| (&*q.plan, (Point { at, injection }, None))),
            );
        }
        let held = if self.failed.is_some() {
            usize::MAX
        } else {
            self.planned.len()
        };
        let (plans, outcomes) = self.seeds.run(held, &cases);
        let mut outcomes = outcomes.into_iter();
        let extra: Vec<_> = outcomes.by_ref().take(extra.len()).collect();
        for (n, j, more) in taken {
            let q = &mut self.planned[n];
            q.got[j].extend(outcomes.by_ref().take(more));
            q.asked[j] += more;
        }
        for plan in plans {
            match plan {
                _ if self.failed.is_some() => {}
                Ok(plan) => {
                    let sequence = self.sequence(plan);
                    self.planned.push_back(sequence);
                }
                Err(panicked) => self.failed = Some(panicked),
            }
        }
        extra
    }

    /// Returns the sequence of `plan`: for each kind, its write-outs a fault hits, spread.
    fn sequence(&self, plan: Arc<case::Plan>) -> Sequence {
        let injection = self.fault.injection();
        let lists: Vec<(Kind, Vec<usize>)> = self
            .kinds
            .iter()
            .map(|&kind| {
                let candidates: Vec<usize> = (0..plan.writes.len())
                    .filter(|&at| {
                        plan.kind(at) == Some(kind)
                            && plan.fault(at, injection) == self.fault
                            && plan.takes(at, injection)
                    })
                    .collect();
                (kind, spread(&candidates, self.per_kind * 3 + 2))
            })
            .collect();
        Sequence {
            got: lists.iter().map(|_| VecDeque::new()).collect(),
            asked: vec![0; lists.len()],
            lists,
            plan,
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
    let ids = &plan.ids;
    let mut out: BTreeMap<Held, (Kind, usize)> = BTreeMap::new();
    for (at, w) in plan.writes.iter().enumerate() {
        let Some(kind) = plan.kind(at).filter(|k| s.kinds.contains(k)) else {
            continue;
        };
        let Some((held, _)) = ids.holding(w.kind, w.owner, w.pid) else {
            continue;
        };
        let injection = fault.injection();
        if due.contains(&held) && plan.fault(at, injection) == fault && plan.takes(at, injection) {
            out.entry(held).or_insert((kind, at));
        }
    }
    out.into_iter().map(|(h, (k, at))| (h, k, at)).collect()
}

/// Returns the cases of `fault` the tier asks, with a vacuum beside each write when
/// `concurrent`.
fn search(fault: Fault, kinds: Vec<Kind>, tier: &Tier, concurrent: bool) -> Searched {
    let mut s = Searched {
        kinds: kinds.clone(),
        cases: BTreeMap::new(),
        endings: BTreeMap::new(),
        reached: BTreeSet::new(),
        rows: BTreeMap::new(),
        steered: BTreeMap::new(),
        steer_tries: BTreeMap::new(),
        failed_opens: BTreeMap::new(),
        found: Vec::new(),
        overlapped: 0,
        failed_after_restart: 0,
        tearable: BTreeMap::new(),
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
    let seeds: Vec<u64> = (1..=tier.sequences).chain(steered).collect();
    let mut w = Window {
        seeds: pool::Seeds::new(&seeds, concurrent),
        planned: VecDeque::new(),
        failed: None,
        fault,
        kinds: kinds.clone(),
        per_kind: tier.per_kind,
    };
    for &seed in &seeds {
        let late = seed > tier.sequences;
        let walked = |k: &Kind| done(*k, &s) || (!tier.every_row && never_reached(fault, *k));
        if kinds.iter().all(walked) && (late || !tier.every_row || rows_due(fault, &s).is_empty()) {
            break;
        }
        if w.planned.is_empty() {
            w.step(&s, 0, &done, &[]);
        }
        if fault == Fault::Torn {
            let plan = &w.first().plan;
            for at in 0..plan.writes.len() {
                if let Some(kind) = plan.kind(at) {
                    let n = s.tearable.entry(kind).or_default();
                    n.0 += 1;
                    n.1 += usize::from(plan.tear(at).is_some());
                }
            }
        }
        let (mut k, mut i) = (0, 0);
        loop {
            let q = w.first();
            while let Some((kind, ats)) = q.lists.get(k) {
                if i == ats.len() || done(*kind, &s) {
                    (k, i) = (k + 1, 0);
                    continue;
                }
                let Some(outcome) = q.got[k].pop_front() else {
                    break;
                };
                let outcome = outcome.unwrap_or_else(|p| p.raise());
                if let Some(outcome) = outcome {
                    *s.cases.entry(*kind).or_default() += 1;
                    s.tally(*kind, &q.plan, q.plan.seed, ats[i], outcome);
                }
                i += 1;
            }
            if k == q.lists.len() {
                break;
            }
            w.step(&s, k, &done, &[]);
        }
        if !concurrent && !late && tier.steers > 0 {
            let q = w.first();
            let steered = steer(fault, &q.plan, &s, tier.steers);
            let ats: Vec<usize> = steered.iter().map(|(_, _, at)| *at).collect();
            let k = kinds.len();
            let outcomes = w.step(&s, k, &done, &ats);
            let q = w.first();
            for ((held, kind, at), outcome) in steered.into_iter().zip(outcomes) {
                *s.steer_tries.entry(held).or_default() += 1;
                if let Some(outcome) = outcome.unwrap_or_else(|p| p.raise()) {
                    *s.steered.entry(kind).or_default() += 1;
                    s.tally(kind, &q.plan, q.plan.seed, at, outcome);
                }
            }
        }
        w.planned.pop_front();
    }
    s
}

/// A violation's text, its consumer, and the kind and owner of the page its fault hit.
type Shape = (Kind, Option<world::Rel>, Consumer, String);

/// Returns what `s` found, failing on any kind with fewer than `per_kind` cases, any cell its
/// consumer never reached, any cell that ends at the open by its rule but had a case whose open
/// did not fail, or any violation no known failure names.
fn report(fault: Fault, s: &Searched, tier: &Tier) -> String {
    let per_kind = tier.per_kind;
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
    let short: Vec<(Kind, usize)> = s
        .kinds
        .iter()
        .filter(|k| !never_reached(fault, **k))
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
    assert_rows_asked(fault, s, tier.every_row, &text);
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

/// A case the default profile runs: its fault, the kind of the page its write-out writes, its
/// seed and write-out, and the row of the stated table it asks.
type Pinned = (Fault, Kind, u64, usize, Row);

/// The cases the default profile runs: a lost write of each structure whose damage is refused and
/// marked, a settled status page, a flipped byte, an LSN map page unreadable at the open and a
/// lost write past a recycle and past the map's entry of its page.
#[rustfmt::skip]
const PINNED: &[Pinned] = {
    use Class::{Damaged, Settled, Unarmed};
    use Fault::{Flipped, LostCurrent, Recycled, UnreadableLasting};
    use Held::{Bitmap, CatalogRoot, CommitLog, CommitTimes, Counters, LsnMap, MarksPage, Relation};
    use Phase::{Opening, Running};
    &[
        (LostCurrent, Kind::Heap, 5, 641, (Damaged, Relation, Opening)),
        (LostCurrent, Kind::Index, 5, 619, (Damaged, Relation, Opening)),
        (LostCurrent, Kind::Engine, 5, 605, (Damaged, CatalogRoot, Opening)),
        (LostCurrent, Kind::Engine, 5, 614, (Damaged, Counters, Opening)),
        (LostCurrent, Kind::Engine, 5, 657, (Damaged, CommitTimes, Opening)),
        (LostCurrent, Kind::CommitLog, 1001, 639, (Damaged, CommitLog, Opening)),
        (LostCurrent, Kind::CommitLog, 5, 658, (Settled, CommitLog, Opening)),
        (LostCurrent, Kind::LsnMap, 5, 660, (Damaged, LsnMap, Opening)),
        (LostCurrent, Kind::Bitmap, 5, 648, (Damaged, Bitmap, Opening)),
        (LostCurrent, Kind::Marks, 5, 655, (Damaged, MarksPage, Opening)),
        (LostCurrent, Kind::Heap, 1001, 641, (Damaged, Relation, Running)),
        (Recycled, Kind::Heap, 1001, 624, (Damaged, Relation, Running)),
        (Flipped, Kind::Heap, 3, 620, (Damaged, Relation, Opening)),
        (Flipped, Kind::CommitLog, 5, 608, (Damaged, CommitLog, Opening)),
        (UnreadableLasting, Kind::LsnMap, 5, 660, (Unarmed, LsnMap, Opening)),
    ]
};

/// Returns why the case of `plan` with the fault of `pin` at write-out `at` fails `pin`, `None`
/// when it hits the pin's cell, asks its row, shows no violation and every consumer but the
/// catalog meets its page, the open only for a page the open finds and a read only for a page
/// that is neither settled nor an index page's. Verify names a settled page
/// `Settled` and a flipped one `Checksum`, and nothing else of it.
fn pin_fails(plan: &case::Plan, &(fault, kind, _, _, row): &Pinned, at: usize) -> Option<String> {
    let injection = fault.injection();
    let cell = (plan.kind(at), plan.fault(at, injection));
    if cell != (Some(kind), fault) {
        return Some(format!("writes another cell: {cell:?}"));
    }
    let Some(o) = case::run(plan, Point { at, injection }) else {
        return Some("the fault does not take".into());
    };
    let v = &o.verdicts;
    let unmet: Vec<Consumer> = CONSUMERS
        .into_iter()
        .filter(|c| *c != Consumer::Catalog)
        .filter(|c| *c != Consumer::Settle || row.2 == Phase::Opening)
        .filter(|c| *c != Consumer::Read || (row.0 != Class::Settled && kind != Kind::Index))
        .filter(|c| !v.met.contains(c))
        .collect();
    let named = match row.0 {
        Class::Settled => "Settled",
        Class::Damaged if fault == Fault::Flipped => "Checksum",
        _ => "",
    };
    let misnamed = !named.is_empty() && v.named != [named];
    (!v.rows.contains(&row) || !unmet.is_empty() || misnamed || !v.violations.is_empty()).then(
        || {
            format!(
                "asks {:?}, meets no {unmet:?}, verify names it {:?}, shows {:?}",
                v.rows, v.named, v.violations
            )
        },
    )
}

#[test]
fn a_pinned_case_of_each_row_asks_it_and_breaks_no_rule() {
    let seeds: Vec<u64> = PINNED
        .iter()
        .map(|p| p.2)
        .collect::<BTreeSet<u64>>()
        .into_iter()
        .collect();
    let plans: BTreeMap<u64, case::Plan> = seeds
        .iter()
        .copied()
        .zip(pool::map(
            &seeds,
            |s| format!("seed {s}"),
            |&s| plans::plan(s, false),
        ))
        .collect();
    let failed: Vec<String> = pool::map(
        PINNED,
        |(fault, _, seed, at, _)| format!("{fault:?} seed {seed} at {at}"),
        |pin| {
            let (fault, kind, seed, at, row) = *pin;
            let plan = &plans[&seed];
            let why = pin_fails(plan, pin, at)?;
            let nearest = (0..plan.writes.len())
                .filter(|x| plan.kind(*x) == Some(kind))
                .find(|x| pin_fails(plan, pin, *x).is_none());
            Some(format!(
                "{fault:?} {kind:?} {row:?}: seed {seed} at {at} {why}; nearest write-out that \
                 holds {nearest:?}"
            ))
        },
    )
    .into_iter()
    .flatten()
    .collect();
    assert_eq!(Vec::<String>::new(), failed);
}

#[test]
fn a_pinned_known_failure_still_shows() {
    let pinned = KNOWN
        .iter()
        .find(|k| {
            k.ticket == BLANK_OPEN
                && k.fault == Fault::LostEarlier
                && k.kind == Kind::Heap
                && k.consumer == Consumer::Settle
                && (k.seed, k.at) == (6, 688)
        })
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

#[test]
fn a_flipped_byte_of_a_page_is_refused_and_marked() {
    eprintln!("{}", shard(Fault::Flipped));
    reruns(Fault::Flipped);
}

/// Returns the sequences and cases of each kind a concurrent or paired tier runs: a few under
/// the per-change tier, more under `deep`.
fn wide() -> (u64, usize) {
    if every_case() { (6, 2) } else { (2, 1) }
}

/// Search each of the faults `of` picks with a vacuum beside each write, and fail on a fault with
/// fewer cases a vacuum overlapped than the tier asks, or a violation no known failure names.
/// Returns what the searches found.
fn vacuum(of: impl Fn(Fault) -> bool) -> String {
    let (sequences, per_kind) = wide();
    let tier = Tier {
        sequences: sequences * 4,
        per_kind,
        steers: per_kind,
        every_row: every_case(),
    };
    let mut unknown = Vec::new();
    let mut overlapped = BTreeMap::new();
    let mut text = String::new();
    for fault in FAULTS.into_iter().filter(|f| of(*f)) {
        let s = search(fault, kinds(fault), &tier, true);
        overlapped.insert(fault, s.overlapped);
        text += &format!(
            "concurrent {fault:?}: cases {:?}, {} with a vacuum during a write\n",
            s.cases, s.overlapped
        );
        for f in &s.found {
            let v = &f.violation;
            text += &format!(
                "concurrent {fault:?} {:?} {:?}: {}; seed {} at {}: {:?}; {}\n",
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
        "faults with fewer than {per_kind} cases a vacuum overlapped\n{text}"
    );
    assert_eq!(
        Vec::<(Fault, Kind, Consumer, String, u64, usize)>::new(),
        unknown,
        "{text}"
    );
    text
}

fn is_unreadable(fault: Fault) -> bool {
    matches!(fault, Fault::UnreadableOnce | Fault::UnreadableLasting)
}

#[test]
fn a_vacuum_beside_each_write_loses_no_row_and_hides_no_lost_write() {
    eprintln!("{}", vacuum(judge::is_lost));
}

#[test]
fn a_vacuum_beside_each_write_loses_no_row_and_hides_no_unreadable_page() {
    eprintln!("{}", vacuum(is_unreadable));
}

#[test]
fn a_vacuum_beside_each_write_loses_no_row_and_hides_no_other_damage() {
    eprintln!("{}", vacuum(|f| !judge::is_lost(f) && !is_unreadable(f)));
}

/// The second fault of a pair, as the pairs tier counts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Second {
    /// A lost write whose log no recycle passes.
    Lost,
    /// A lost write the sequence recycles the log past.
    Recycled,
    Torn,
    UnreadableOnce,
    UnreadableLasting,
    /// A write to the log lost.
    LogLost,
    /// A block of the log unreadable at the first read after its write.
    LogUnreadable,
    /// The damage list's page unreadable for good.
    ListUnreadable,
    /// The marks page unreadable at the first read after its write-out.
    MarksUnreadable,
    /// A crash at a point of a checkpoint's fold.
    Fold(Fold),
}

/// Returns the points of `second` a pair with a lost write at write-out `first` of `plan` takes:
/// each at a later write-out of another write than `first`'s, and for an unreadable block of the
/// log, one the run with no fault reads after its write.
fn second_points(plan: &case::Plan, first: usize, second: Second) -> Vec<Point> {
    let writes = &plan.writes;
    let other = |at: usize| at > first && writes[at].op != writes[first].op;
    let page = |at: usize| match second {
        Second::Lost | Second::Recycled => {
            let recycled = plan.fault(at, Injection::Lost) == Fault::Recycled;
            (recycled == (second == Second::Recycled)).then_some(Injection::Lost)
        }
        Second::Torn => plan.takes(at, Injection::Torn).then_some(Injection::Torn),
        Second::UnreadableOnce => Some(Injection::Unreadable { lasting: false }),
        Second::UnreadableLasting => Some(Injection::Unreadable { lasting: true }),
        Second::ListUnreadable => (plan.kind(at) == Some(Kind::DamageList))
            .then_some(Injection::Unreadable { lasting: true }),
        Second::MarksUnreadable => {
            (plan.kind(at) == Some(Kind::Marks)).then_some(Injection::Unreadable { lasting: false })
        }
        Second::LogLost | Second::LogUnreadable | Second::Fold(_) => None,
    };
    let log = |write: usize, folds: bool| match second {
        Second::LogLost => Some(Injection::LogLost { write }),
        Second::LogUnreadable => plan.log[write]
            .read_later
            .then_some(Injection::LogUnreadable { write }),
        Second::Fold(point) => folds.then_some(Injection::Crash { point, write }),
        _ => None,
    };
    let pages = (0..writes.len())
        .filter(|at| other(*at) && plan.kind(*at).is_some())
        .filter_map(|at| {
            Some(Point {
                at,
                injection: page(at)?,
            })
        });
    let logged = plan.log.iter().enumerate().filter_map(|(write, w)| {
        let injection = log(write, w.folds)?;
        let at = match injection {
            Injection::Crash {
                point: Fold::List, ..
            } => (w.after..writes.len()).find(|at| Some(writes[*at].pid) == plan.list)?,
            _ => w.after.checked_sub(1)?,
        };
        other(at).then_some(Point { at, injection })
    });
    pages.chain(logged).collect()
}

/// Returns the pairs of `plan` the tier asks of each of `seconds`, in order: a lost write at up to
/// `per` times three write-outs that have a second point, each with up to `per` times two of its
/// second points.
fn pair_cases(plan: &case::Plan, seconds: &[Second], per: usize) -> Vec<(Second, Point, Point)> {
    let writes = plan.writes.len();
    let mut out = Vec::new();
    for &second in seconds {
        let lost: Vec<usize> = (0..writes)
            .filter(|at| plan.kind(*at).is_some())
            .filter(|first| !second_points(plan, *first, second).is_empty())
            .collect();
        for first in spread(&lost, per * 3) {
            let points = second_points(plan, first, second);
            let ats: Vec<usize> = (0..points.len()).collect();
            let one = Point {
                at: first,
                injection: Injection::Lost,
            };
            out.extend(
                spread(&ats, per * 2)
                    .into_iter()
                    .map(|i| (second, one, points[i])),
            );
        }
    }
    out
}

/// Returns the least number of cases of each second fault that reached it: one under the
/// per-change tier, two under `deep`.
fn pairs_floor() -> usize {
    if every_case() { 2 } else { 1 }
}

/// The cases of one second fault: those judged, those whose consumers ran their open, those
/// whose consumers' open succeeded, and those whose planned read failure fired.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Ran {
    judged: usize,
    consumed: usize,
    opened: usize,
    fired: usize,
}

/// Run the pairs of a lost write with each of `seconds` the tier asks, over up to four times the
/// tier's sequences for a second fault still under its floor, judge each on the verdicts and on
/// its file's evidence, and fail on a second fault with fewer cases that reached it than the
/// floor, or a violation no known failure names. Returns what the pairs found.
fn pairs(seconds: &[Second]) -> String {
    let (sequences, per) = wide();
    let mut ran: BTreeMap<Second, Ran> = seconds.iter().map(|s| (*s, Ran::default())).collect();
    let mut found = Vec::new();
    let (mut recorded, mut marked) = (0, 0);
    let floor = pairs_floor();
    let short =
        |ran: &BTreeMap<Second, Ran>, second: Second| reached(second, &ran[&second]) < floor;
    let mut past: BTreeMap<Second, Vec<u64>> = BTreeMap::new();
    let seeds: Vec<u64> = (1..=sequences * 4).collect();
    let mut ahead = pool::Seeds::new(&seeds, false);
    let mut planned: VecDeque<pool::Ran<Arc<case::Plan>>> = VecDeque::new();
    for &seed in &seeds {
        let wanted: Vec<Second> = seconds
            .iter()
            .copied()
            .filter(|s| seed <= sequences || short(&ran, *s))
            .collect();
        if wanted.is_empty() {
            break;
        }
        if planned.is_empty() {
            planned.extend(ahead.run(0, &[]).0);
        }
        let plan = planned
            .pop_front()
            .expect("the seed is planned")
            .unwrap_or_else(|p| p.raise());
        let before: BTreeMap<Second, usize> =
            wanted.iter().map(|s| (*s, reached(*s, &ran[s]))).collect();
        let pairs = pair_cases(&plan, &wanted, per);
        let cases: Vec<(&case::Plan, pool::Case)> = pairs
            .iter()
            .map(|(_, one, two)| (&*plan, (*one, Some(*two))))
            .collect();
        let (more, outcomes) = ahead.run(planned.len(), &cases);
        planned.extend(more);
        for ((second, one, two), paired) in pairs.into_iter().zip(outcomes) {
            let Some(paired) = paired.unwrap_or_else(|p| p.raise()) else {
                continue;
            };
            let counts = ran.entry(second).or_default();
            counts.judged += 1;
            counts.consumed += usize::from(paired.consumed);
            counts.opened += usize::from(paired.opened);
            counts.fired += usize::from(paired.read_failed);
            let (records, marks) = paired.verdicts.evidence;
            recorded += usize::from(records > 0);
            marked += usize::from(marks > 0);
            let cells: Vec<_> = [one, two]
                .into_iter()
                .filter(|p| p.injection.on_page())
                .map(|p| {
                    (
                        plan.fault(p.at, p.injection),
                        plan.kind(p.at).expect("a counted write-out has a kind"),
                        plan.owner(p.at),
                    )
                })
                .collect();
            let first = one.at;
            for v in paired.verdicts.violations {
                let pinned = KNOWN.iter().any(|k| {
                    k.seed == seed
                        && k.at == first
                        && k.then.is_some_and(|(t, _)| t == two)
                        && k.consumer == v.consumer
                        && k.what == v.what
                });
                let gated = pinned
                    || cells
                        .iter()
                        .any(|(f, k, o)| known(*f, *k, *o, v.consumer, &v.what).is_some());
                if !gated {
                    found.push((seed, first, two, v));
                }
            }
        }
        for second in wanted {
            if seed > sequences && reached(second, &ran[&second]) > before[&second] {
                past.entry(second).or_default().push(seed);
            }
        }
    }
    let mut text = format!(
        "pairs by second fault: {ran:?}; whose file held a damage record {recorded}, a durable \
         mark {marked}; reached in sequences past the tier's {past:?}\n"
    );
    for (seed, first, two, v) in &found {
        text += &format!(
            "pair lost at {first} then {two:?}, seed {seed}: {:?} {}; {}\n",
            v.consumer, v.what, v.detail
        );
    }
    let paired: Vec<&Known> = KNOWN
        .iter()
        .filter(|k| {
            k.then
                .is_some_and(|(t, _)| seconds.iter().any(|s| holds(*s, t)))
        })
        .collect();
    pool::map(&paired, |k| named(k), |k| rerun(k));
    let pinned: Vec<&PinnedPair> = PINNED_PAIRS
        .iter()
        .filter(|p| seconds.iter().any(|s| holds(*s, p.2)))
        .collect();
    pool::map(
        &pinned,
        |(seed, (at, _), then, _, _)| format!("pinned pair seed {seed} at {at} then {then:?}"),
        |p| rerun_pair(p),
    );
    assert_reached("pairs whose file held a damage record", recorded, 1);
    assert_reached("pairs whose file held a durable mark", marked, 1);
    let under: Vec<_> = ran
        .iter()
        .filter(|(second, _)| short(&ran, **second))
        .collect();
    assert_eq!(
        Vec::<(&Second, &Ran)>::new(),
        under,
        "second faults with fewer than {floor} pairs that reached their fault\n{text}"
    );
    assert_eq!(
        Vec::<(u64, usize, Point, Violation)>::new(),
        found,
        "violations\n{text}"
    );
    text
}

/// A pair that breaks no rule: its seed, the write-out of its lost write and that page's kind, its
/// second fault with the kind of page that fault's write-out writes, and what its case shows.
type PinnedPair = (u64, (usize, Kind), Point, Kind, Shows);

/// What a pinned pair's case shows besides an open with no violation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shows {
    /// Its planned read failure fired.
    ReadFailed,
    /// Its faulted run tore the rewrite of a status page the commit log settled.
    TornSettle,
}

/// The pairs the search must still reach: a lost write of the settings heap beside a damage list
/// page unreadable for good, which opens for verify; and a lost write of a status page whose
/// settle's rewrite the crash after the tear of an engine page at 679 tears.
const PINNED_PAIRS: &[PinnedPair] = &[
    (
        2,
        (610, Kind::Engine),
        Point {
            at: 639,
            injection: Injection::Unreadable { lasting: true },
        },
        Kind::DamageList,
        Shows::ReadFailed,
    ),
    (
        6,
        (677, Kind::CommitLog),
        Point {
            at: 679,
            injection: Injection::Torn,
        },
        Kind::Engine,
        Shows::TornSettle,
    ),
];

/// Fail unless the write-outs of the pair `p` still have their kinds and its case shows what it
/// pins, opens and shows no violation.
fn rerun_pair(&(seed, (at, kind), then, then_kind, shows): &PinnedPair) {
    let plan = plans::plan(seed, false);
    assert_eq!(
        (Some(kind), Some(then_kind), true),
        (
            plan.kind(at),
            plan.kind(then.at),
            plan.takes(then.at, then.injection)
        ),
        "pinned pair seed {seed} at {at} then {then:?} writes other cells; pin it again"
    );
    let first = Point {
        at,
        injection: Injection::Lost,
    };
    let o = case::run_pair(&plan, first, then).expect("the pinned pair's faults take");
    assert_eq!(
        (
            shows == Shows::ReadFailed,
            shows == Shows::TornSettle,
            true,
            Vec::<Violation>::new()
        ),
        (
            o.read_failed,
            o.torn_settle,
            o.opened,
            o.verdicts.violations
        ),
        "pinned pair seed {seed} at {at} then {then:?}: fired, torn settle, opened, violations"
    );
}

/// Returns the cases of `second` that reached what it is for: those whose planned read failure
/// fired for an unreadable block of the log, list page or marks page, those whose consumers'
/// open succeeded for the rest.
fn reached(second: Second, r: &Ran) -> usize {
    match second {
        Second::LogUnreadable | Second::ListUnreadable | Second::MarksUnreadable => r.fired,
        _ => r.opened,
    }
}

/// Returns whether the point `p` is a second fault of kind `second`, by its injection alone.
fn holds(second: Second, p: Point) -> bool {
    match (second, p.injection) {
        (Second::Lost | Second::Recycled, Injection::Lost)
        | (Second::Torn, Injection::Torn)
        | (Second::LogLost, Injection::LogLost { .. })
        | (Second::LogUnreadable, Injection::LogUnreadable { .. }) => true,
        (Second::UnreadableOnce | Second::MarksUnreadable, Injection::Unreadable { lasting }) => {
            !lasting
        }
        (Second::UnreadableLasting | Second::ListUnreadable, Injection::Unreadable { lasting }) => {
            lasting
        }
        (Second::Fold(f), Injection::Crash { point, .. }) => f == point,
        _ => false,
    }
}

#[test]
fn two_faults_lose_no_acknowledged_row_and_hide_no_stale_page() {
    eprintln!(
        "{}",
        pairs(&[
            Second::Lost,
            Second::Recycled,
            Second::Torn,
            Second::UnreadableOnce,
            Second::UnreadableLasting,
        ])
    );
}

#[test]
fn a_lost_write_beside_an_unreadable_list_or_marks_page_keeps_every_record_and_mark() {
    eprintln!(
        "{}",
        pairs(&[Second::ListUnreadable, Second::MarksUnreadable])
    );
}

#[test]
fn a_lost_write_beside_a_log_gap_keeps_every_record_and_mark() {
    eprintln!("{}", pairs(&[Second::LogLost, Second::LogUnreadable]));
}

#[test]
fn a_lost_write_beside_a_crash_in_a_fold_keeps_every_record_and_mark() {
    eprintln!("{}", pairs(&FOLDS.map(Second::Fold)));
}

/// Returns why the case `k` pins no longer shows `k`, or `None` when it does.
fn unpinned(k: &Known) -> Option<String> {
    let plan = plans::plan(k.seed, false);
    let writes = plan.writes.len();
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
        && (t.at >= writes || plan.kind(t.at) != Some(kind) || !plan.takes(t.at, t.injection))
    {
        return Some("second fault hits another kind or cannot take".into());
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
        let plan = plans::plan(seed, false);
        let writes = plan.writes.len();
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
                .filter(|&x| t.injection.on_page() || x == t.at)
                .filter(|&x| plan.takes(x, t.injection))
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
