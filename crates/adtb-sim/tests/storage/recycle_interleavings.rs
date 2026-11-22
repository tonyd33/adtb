//! A recycle of the log racing a commit, a checkpoint and a second recycle at each boundary of
//! its checkpoint, and at every pair of them under the deep profile, judged by a crash once both
//! sides end.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use adtb::OpenOptions;
use adtb::runtime::Threads;
use adtb::testing::{Checkpoint, Knobs, Step, StepFn};
use adtb_sim::{Config, Outcome as Ran, Sim};
use adtb_simkit::api;
use adtb_simkit::elle::{self, Elems, Kind, Reply, Req};
use adtb_simkit::racers::{Called, Txn};
use adtb_simkit::steps::{self, Boundaries, Pair, Reached, Second, Stepping};
use adtb_testkit::isolation::{self as iso, Outcome};
use adtb_testkit::process;
use adtb_testkit::profile::{self, sweep};

const SEEDS: Range<u64> = 0..1;

/// The bytes of each log segment.
const SEGMENT: u64 = 16 * 1024;

/// The list key the setup appends to, and the racer appends to.
const KEY: i64 = 1;
/// The element the setup appends.
const FIRST: i64 = 10;
/// The element a racing append appends.
const RACED: i64 = 20;

/// The cells each case inserts before the race to grow the log by several segments, each padded
/// to a kilobyte.
const CELLS: i64 = 64;

/// The segments the recycle of each case removes.
const REMOVED: usize = 16;

/// The boundaries of a recycle's checkpoint.
const RECYCLE: [Step; 2] = [
    Step::Recycle(Checkpoint::Folded),
    Step::Recycle(Checkpoint::Forced),
];

/// What races the recycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Racer {
    /// A transaction appending [`RACED`] to the list of [`KEY`] and committing.
    Append,
    Checkpoint,
    /// A second recycle, which waits for the recycle lock the first holds.
    Recycle,
}

/// Which side an interleaving stops first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum First {
    Recycle,
    Racer,
}

fn of_a_statement(s: Step) -> bool {
    matches!(s.function(), StepFn::Read | StepFn::Write)
}

fn of_a_recycle(s: Step) -> bool {
    s.function() == StepFn::Recycle
}

fn of_a_checkpoint(s: Step) -> bool {
    s.function() == StepFn::Checkpoint
}

fn options() -> OpenOptions {
    OpenOptions {
        testing: Knobs::without_background_vacuum().with_segment_size(SEGMENT),
        ..elle::options()
    }
}

type Files = BTreeMap<String, Vec<u8>>;

static FILES: Mutex<Option<Files>> = Mutex::new(None);

/// Returns a simulation seeded `seed` over the workload's closed database of [`SEGMENT`] byte
/// segments holding [`FIRST`] in the list of [`KEY`].
fn built(seed: u64) -> Sim {
    let mut files = FILES.lock().unwrap_or_else(PoisonError::into_inner);
    let files = files.get_or_insert_with(|| {
        let mut sim = Sim::new(0, Config::default());
        process::run(&mut sim, |rt| {
            let e = Elems::open(&rt, &options()).expect("the workload opens");
            let mut tx = e.begin(Kind::ReadWrite).expect("a transaction begins");
            e.append(&mut tx, KEY, FIRST).expect("an append");
            tx.commit(None).expect("the setup commits");
            drop(e);
        });
        sim.files()
    });
    Sim::with_files(seed, Config::default(), files.clone())
}

/// A step function of a case: a transaction, or a call on a thread of its own.
enum Side {
    Txn(Txn),
    Call(Called<usize>),
}

impl Stepping for Side {
    fn boundaries(&self) -> Boundaries {
        match self {
            Side::Txn(t) => t.boundaries(),
            Side::Call(c) => c.boundaries(),
        }
    }

    fn start(&mut self, rt: &adtb_sim::SimRuntime) {
        match self {
            Side::Txn(t) => t.start(rt),
            Side::Call(c) => c.start(rt),
        }
    }

    fn ended(&mut self) -> bool {
        match self {
            Side::Txn(t) => t.ended(),
            Side::Call(c) => c.ended(),
        }
    }
}

/// What one case left before its crash, and what the open after it found.
#[derive(Debug, Default)]
struct Left {
    recycle: Option<Reached>,
    racer: Option<Reached>,
    /// The segments the recycle removed.
    removed: usize,
    raced: Vec<Reply>,
    /// What a racing call returned: the segments a second recycle removed.
    called: Option<usize>,
    history: Vec<iso::Txn>,
    /// The list of [`KEY`] and the cells after the open that follows the crash.
    list: Vec<i64>,
    cells: Vec<String>,
    verified: Vec<String>,
}

/// Run a recycle stopped at its `i`th boundary and `racer` at its `j`th, the side `first` names
/// stopped first, then crash and open again.
fn run(seed: u64, (racer, first): (Racer, First), (i, j): (usize, usize)) -> Left {
    let mut sim = built(seed);
    let left = Arc::new(Mutex::new(Left::default()));
    let out = left.clone();
    let ran = sim.run(move |rt| {
        let e = Arc::new(Elems::open(&rt, &options()).expect("the workload opens"));
        let db = e.database();
        let pad = "x".repeat(1000);
        for half in [0, CELLS / 2] {
            let cells: String = (half..half + CELLS / 2)
                .map(|n| format!("insert cells.Even {{ id: {n}, n: {n}, m: 0, pad: \"{pad}\" }}\n"))
                .collect();
            api::results(db, &cells);
        }
        db.testing().flush_pages(usize::MAX);
        db.testing().checkpoint();
        let recycling = |name| {
            let db2 = db.clone();
            let on = (name, of_a_recycle as fn(Step) -> bool);
            Called::spawn(&rt, on, move || db2.testing().recycle_log())
        };
        let mut recycle = recycling("recycle");
        let db2 = db.clone();
        let mut side = match racer {
            Racer::Append => {
                let reqs = vec![Req::Append(KEY, RACED), Req::Commit];
                Side::Txn(Txn::start(
                    &rt,
                    &e,
                    (1, Kind::ReadWrite),
                    of_a_statement,
                    reqs,
                ))
            }
            Racer::Checkpoint => {
                let on = ("checkpoint", of_a_checkpoint as fn(Step) -> bool);
                Side::Call(Called::spawn(&rt, on, move || {
                    db2.testing().checkpoint();
                    0
                }))
            }
            Racer::Recycle => Side::Call(recycling("second recycle")),
        };
        let [recycled, raced] = match first {
            First::Recycle => {
                let sides = (&mut recycle, &mut side);
                steps::interleave(&rt, db, sides, (i, j), steps::nothing_between())
            }
            First::Racer => {
                let sides = (&mut side, &mut recycle);
                let [r, c] = steps::interleave(&rt, db, sides, (j, i), steps::nothing_between());
                [c, r]
            }
        };
        let mut left = out.lock().unwrap_or_else(PoisonError::into_inner);
        left.recycle = Some(recycled);
        left.racer = Some(raced);
        left.removed = recycle.take();
        let setup = vec![iso::Op::Append {
            key: KEY,
            elem: FIRST,
        }];
        left.history = vec![iso::Txn::new(0, Outcome::Committed, setup)];
        match &mut side {
            Side::Txn(t) => {
                left.history.push(t.history());
                left.raced = t.replies().to_vec();
            }
            Side::Call(c) => left.called = Some(c.take()),
        }
        drop(left);
        rt.crash_after(Duration::ZERO);
        rt.yield_now();
        unreachable!("the run crashed");
    });
    assert_eq!(Ran::Crashed, ran, "the run crashes");
    process::run(&mut sim, |rt| {
        let e = Elems::open(&rt, &options()).expect("the workload opens after the crash");
        let mut left = left.lock().unwrap_or_else(PoisonError::into_inner);
        left.list = e.lists(KEY..KEY + 1).remove(0);
        left.cells = api::query(e.database(), "from c in cells select c.id");
        left.verified = e.verify();
    });
    Arc::try_unwrap(left)
        .expect("the run let go of what it left")
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner)
}

/// Returns each way `l`, a run of `racer`, breaks what a recycle raced by a step function leaves:
///
/// - the recycle removes [`REMOVED`] segments, or a second recycle does and the first none;
/// - the racer replies as it does alone;
/// - the open after the crash finds every cell and element committed before it, and `verify`
///   finds nothing: no checkpoint's redo point passed a record a page's write had not made
///   durable;
/// - the history, the open's read of the list ending it, is serializable.
fn judged(racer: Racer, what: &str, l: &Left) -> Vec<String> {
    let mut broken = Vec::new();
    let mut expect = |about: &str, ok: bool, got: &dyn std::fmt::Debug| {
        if !ok {
            broken.push(format!("{what}: {about}: {got:?}"));
        }
    };
    let mut list = vec![FIRST];
    match racer {
        Racer::Append => {
            expect("the recycle", l.removed == REMOVED, &l.removed);
            let done = vec![Ok(vec![]), Ok(vec![])];
            expect("the append's replies", l.raced == done, &l.raced);
            list.push(RACED);
        }
        Racer::Checkpoint => {
            expect("the recycle", l.removed == REMOVED, &l.removed);
            expect("the checkpoint", l.called == Some(0), &l.called);
        }
        Racer::Recycle => {
            let both = (l.removed, l.called);
            let one = both == (REMOVED, Some(0)) || both == (0, Some(REMOVED));
            expect("the two recycles", one, &both);
        }
    }
    expect("the list", l.list == list, &l.list);
    let mut cells: Vec<String> = (0..CELLS).map(|n| n.to_string()).collect();
    cells.sort();
    let mut found = l.cells.clone();
    found.sort();
    expect("the cells", found == cells, &found);
    expect("verify", l.verified.is_empty(), &l.verified);
    let mut history = l.history.clone();
    let last = vec![iso::Op::Read {
        key: KEY,
        list: l.list.clone(),
    }];
    history.push(iso::Txn::new(2, Outcome::Committed, last));
    let anomalies = match iso::check(&history) {
        Ok(found) => found.iter().map(|a| format!("{a:?}")).collect(),
        Err(malformed) => vec![format!("{malformed:?}")],
    };
    expect("the history", anomalies.is_empty(), &anomalies);
    broken
}

/// Returns the boundaries `racer` reaches run alone.
fn alone(racer: Racer) -> Vec<Step> {
    match racer {
        Racer::Append => vec![Step::VersionRead, Step::WriteInserted],
        Racer::Checkpoint => [Checkpoint::Folded, Checkpoint::Forced]
            .map(Step::Checkpoint)
            .to_vec(),
        Racer::Recycle => RECYCLE.to_vec(),
    }
}

/// Search `racer` against the recycle with the recycle stopped first, and under the deep profile
/// each side stopped first in turn, judging every case, and check that each side stopped first
/// stopped at its declared boundaries and the pairs run are the declared ones: the second side
/// whole, but a second recycle, which waits for the recycle lock the stopped one holds, under the
/// deep profile at each of its boundaries before.
fn race(racer: Racer) {
    let mut broken = Vec::new();
    let firsts: &[First] = if profile::every_case() {
        &[First::Recycle, First::Racer]
    } else {
        &[First::Recycle]
    };
    for seed in sweep(SEEDS) {
        for &first in firsts {
            let what = format!("{racer:?} {first:?} seed {seed}");
            let mut one = |(a, b): (usize, usize)| {
                let pair = match first {
                    First::Recycle => (a, b),
                    First::Racer => (b, a),
                };
                let l = run(seed, (racer, first), pair);
                broken.extend(judged(racer, &format!("{what} at {pair:?}"), &l));
                let (recycle, raced) = (l.recycle.expect("ran"), l.racer.expect("ran"));
                match first {
                    First::Recycle => [recycle, raced],
                    First::Racer => [raced, recycle],
                }
            };
            let (reached, pairs) = if profile::every_case() {
                steps::every_pair(&mut one)
            } else {
                steps::every_boundary(&mut one)
            };
            let (stops, second) = match first {
                First::Recycle => (RECYCLE.to_vec(), alone(racer)),
                First::Racer => (alone(racer), RECYCLE.to_vec()),
            };
            assert_eq!(stops, reached, "{what}: the first side's boundaries");
            let end = if racer == Racer::Recycle {
                Second::Blocked
            } else {
                Second::Whole
            };
            let mut declared: Vec<Pair> = Vec::new();
            for &stop in &stops {
                if profile::every_case() && end == Second::Whole {
                    declared.extend(second.iter().map(|&s| (stop, Second::At(s))));
                }
                declared.push((stop, end));
            }
            assert_eq!(declared, pairs, "{what}: the pairs run");
        }
    }
    assert_eq!(Vec::<String>::new(), broken);
}

#[test]
fn a_recycle_raced_at_each_boundary_of_its_checkpoint_loses_no_commit_to_a_crash() {
    for racer in [Racer::Append, Racer::Checkpoint, Racer::Recycle] {
        race(racer);
    }
}
