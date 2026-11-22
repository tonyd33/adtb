//! A second vacuum of a relation, and a vacuum of another relation sharing its large-value store,
//! each with its history copies, sweep and expiry, attempted while a running vacuum is stopped at
//! a pace point: at the first of each run of one kind, and under the deep profile at every pace
//! point and every pair. Each racer runs once with a deadline that passes while the vacuum is
//! stopped, and once with none.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use adtb::runtime::{Clock, ThreadId, Threads};
use adtb::testing::{Knobs, Step, StepFn, Vacuum};
use adtb::{CallOptions, Database, Error, Log, OpenOptions, RejectReason};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_simkit::api;
use adtb_simkit::open;
use adtb_simkit::steps::{self, Boundaries, Pair, Reached, Second, Stepping, Stop};
use adtb_testkit::process;
use adtb_testkit::profile::{self, sweep};
use adtb_testkit::refused_as;

const SEEDS: Range<u64> = 0..1;

const FILE: &str = "vacuums.adt";

const SCHEMA: &str = r"module app
add module app
add type Item { id: Key items, k: Text, t: Text }
  = A { a: I64 }
  | B { b: I64 }
add relation items : Item key id generate sequence
  index (k)
add type Other = { id: Key others, t: Text }
add relation others : Other key id generate sequence
add type Tick = { id: Key ticks, n: I64 }
add relation ticks : Tick key id generate sequence
";

/// The rows of each relation, every one with a value stored out of line in one chunk, and in
/// `items` with a key of the index on `k` long enough that the index takes more than one leaf.
const ROWS: i64 = 8;

/// The commits into `ticks` under the retention window, enough that its record of commit times
/// takes more than one leaf.
const COMMITS: i64 = 400;

/// How long a racing vacuum waits for the vacuum lock.
const WAIT: Duration = Duration::from_millis(20);

/// What `vacuum items` reports once every row is deleted, for each constructor past its id: the
/// pages and the chunks of the relation freed, and the versions removed.
const ITEMS: [&str; 2] = [
    "\"1 pages, 8 out-of-line chunks of the relation\", removed: 4 }",
    "\"1 pages, 8 out-of-line chunks of the relation\", removed: 4 }",
];

/// What `vacuum others` reports once every row is deleted.
const OTHERS: [&str; 1] = ["\"1 pages, 8 out-of-line chunks of the relation\", removed: 8 }"];

/// What `vacuum items` reports run again, for each of its constructors.
const NOTHING: [&str; 2] = [
    "\"0 pages, 0 out-of-line chunks of the relation\", removed: 0 }",
    "\"0 pages, 0 out-of-line chunks of the relation\", removed: 0 }",
];

/// The kinds of pace point a vacuum of `items` reaches, in the order it first reaches them.
const PACED: [Vacuum; 6] = [
    Vacuum::Scan,
    Vacuum::Keep,
    Vacuum::Leaf,
    Vacuum::Prune,
    Vacuum::Sweep,
    Vacuum::Expire,
];

/// What races the vacuum of `items`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Racer {
    /// A second `vacuum items`.
    Same,
    /// `vacuum others`, whose relation shares the store of `items`.
    Other,
}

impl Racer {
    fn text(self) -> &'static str {
        match self {
            Racer::Same => "vacuum items",
            Racer::Other => "vacuum others",
        }
    }
}

fn options() -> OpenOptions {
    OpenOptions {
        testing: Knobs::without_background_vacuum(),
        ..open::options(2)
    }
}

type Files = BTreeMap<String, Vec<u8>>;

static FILES: Mutex<Option<Files>> = Mutex::new(None);

/// Returns a simulation seeded `seed` over a closed database of [`ROWS`] rows of each relation,
/// each with a value of 3000 bytes, and in `items` a key of 1500, all deleted under a retention
/// window of an hour after [`COMMITS`] commits into `ticks`.
fn built(seed: u64) -> Sim {
    let mut files = FILES.lock().unwrap_or_else(PoisonError::into_inner);
    let files = files.get_or_insert_with(|| {
        let mut sim = Sim::new(0, Config::default());
        process::run(&mut sim, |rt| {
            let db = Database::open_in(rt, FILE, &Log::new([SCHEMA]), &options())
            .expect("the database opens");
            let t = "x".repeat(3000);
            let text: String = (1..=ROWS)
                .map(|k| {
                    let c = ["A { a", "B { b"][(k % 2) as usize];
                    format!(
                        "insert items.{c}: {k}, id: items.key({k}), k: \"{k:0>1500}\", t: \"{t}\" }}\n\
                         insert others {{ id: others.key({k}), t: \"{t}\" }}\n"
                    )
                })
                .collect();
            api::results(&db, &text);
            api::results(&db, "set setting RetentionWindow = { length: 1h }");
            for n in 1..=COMMITS {
                api::results(&db, &format!("insert ticks {{ id: ticks.key({n}), n: {n} }}"));
            }
            api::results(&db, "delete i in items where true");
            api::results(&db, "delete o in others where true");
            db.close().expect("the database closes");
        });
        sim.files()
    });
    Sim::with_files(seed, Config::default(), files.clone())
}

fn of_a_vacuum(s: Step) -> bool {
    s.function() == StepFn::Vacuum
}

type Reply = Result<Vec<String>, Error>;

/// A vacuum call of `text` on a thread of its own once started, waiting `wait` at most for the
/// vacuum lock when set. Its reply is each row it returns past the constructor, sorted.
struct Call {
    thread: ThreadId,
    go: Arc<AtomicBool>,
    reply: Arc<Mutex<Option<Reply>>>,
}

impl Call {
    fn spawn(
        rt: &SimRuntime,
        db: &Database<SimRuntime>,
        text: &'static str,
        wait: Option<Duration>,
    ) -> Call {
        let go = Arc::new(AtomicBool::new(false));
        let reply = Arc::new(Mutex::new(None));
        let (go2, reply2) = (go.clone(), reply.clone());
        let (db, rt2, driver) = (db.clone(), rt.clone(), rt.current());
        let thread = rt.spawn("call", move || {
            while !go2.load(Ordering::Acquire) {
                rt2.park();
            }
            let options = CallOptions {
                deadline: wait.map(|w| rt2.wall() + w),
                ..CallOptions::default()
            };
            let got = db.query_text(text, &[], &options).map(|mut rows| {
                let printed = api::printed(rows.remove(0)).into_iter();
                let freed = |r: String| r.split_once("freed: ").map(|(_, f)| f.to_owned());
                let mut freed: Vec<String> = printed.map(|r| freed(r).expect("a row")).collect();
                freed.sort();
                freed
            });
            *reply2.lock().unwrap_or_else(PoisonError::into_inner) = Some(got);
            rt2.unpark(driver);
        });
        Call { thread, go, reply }
    }

    fn reply(&self) -> Reply {
        let reply = self.reply.lock().unwrap_or_else(PoisonError::into_inner);
        reply.clone().expect("the call replied")
    }
}

impl Stepping for Call {
    fn boundaries(&self) -> Boundaries {
        (self.thread, of_a_vacuum)
    }

    fn start(&mut self, rt: &SimRuntime) {
        self.go.store(true, Ordering::Release);
        rt.unpark(self.thread);
    }

    fn ended(&mut self) -> bool {
        let reply = self.reply.lock().unwrap_or_else(PoisonError::into_inner);
        reply.is_some()
    }
}

/// What one case left.
#[derive(Debug)]
struct Ran {
    vacuum: Reached,
    racer: Reached,
    vacuumed: Reply,
    raced: Reply,
    /// The pauses calls made for the vacuum lock while the racer ran.
    waits: u64,
    /// What `vacuum items` and then `vacuum others` reported, run after both sides ended.
    after: [Reply; 2],
    /// The rows `verify` reports over the whole database, printed.
    verified: Vec<String>,
}

/// Run `vacuum items` stopped at its `i`th pace point, with `racer` run to its `j`th or its end.
fn run(racer: Racer, seed: u64, (i, j): (usize, usize)) -> Ran {
    process::run(&mut built(seed), |rt| {
        let db = Database::open_in(rt.clone(), FILE, &Log::new([SCHEMA]), &options())
            .expect("the database opens");
        let mut first = Call::spawn(&rt, &db, "vacuum items", None);
        let mut second = Call::spawn(&rt, &db, racer.text(), Some(WAIT));
        let before = db.testing().rebuild_waits();
        let [vacuum, raced] =
            steps::interleave(&rt, &db, (&mut first, &mut second), (i, j), Box::new(|| {}));
        let waits = db.testing().rebuild_waits() - before;
        let after = ["vacuum items", "vacuum others"].map(|text| {
            let mut again = Call::spawn(&rt, &db, text, None);
            again.start(&rt);
            while !again.ended() {
                rt.park();
            }
            again.reply()
        });
        let verified = api::printed(db.verify(None).expect("verify runs"));
        let ran = Ran {
            vacuum,
            racer: raced,
            vacuumed: first.reply(),
            raced: second.reply(),
            waits,
            after,
            verified,
        };
        db.close().expect("the database closes");
        ran
    })
}

/// The driver's turns a racer has to pause for the vacuum lock twice.
const PAUSES: usize = 10_000;

/// Run `vacuum items` stopped at its `i`th pace point, start `racer` with no deadline, let it pause
/// twice for the vacuum lock, and then let the vacuum go on. Check that the racer reaches its first
/// pace point only after the vacuum reached its last, and replies as it does run alone after the
/// vacuum.
fn waits_out(racer: Racer, seed: u64, i: usize) {
    let what = format!("{racer:?} at {i} with no deadline");
    process::run(&mut built(seed), |rt| {
        let db = Database::open_in(rt.clone(), FILE, &Log::new([SCHEMA]), &options())
            .expect("the database opens");
        let mut first = Call::spawn(&rt, &db, "vacuum items", None);
        let mut second = Call::spawn(&rt, &db, racer.text(), None);
        let [a, b] = Stop::set_each(
            &rt,
            &db,
            [(first.boundaries(), i), (second.boundaries(), 1)],
        );
        first.start(&rt);
        a.stopped(&rt, &mut || first.ended().then_some(()))
            .unwrap_or_else(|()| panic!("{what}: the vacuum ended first"));
        let before = db.testing().rebuild_waits();
        second.start(&rt);
        let turns = (0..PAUSES)
            .take_while(|_| db.testing().rebuild_waits() < before + 2)
            .inspect(|_| rt.yield_to_event())
            .count();
        assert!(
            turns < PAUSES,
            "{what}: the racer paused {} times",
            db.testing().rebuild_waits() - before
        );
        assert!(!b.has_stopped() && !second.ended(), "{what}");
        a.release(&rt);
        b.stopped(&rt, &mut || second.ended().then_some(()))
            .unwrap_or_else(|()| panic!("{what}: the racer reaches no pace point"));
        let reached = a.reached();
        b.release(&rt);
        while !(first.ended() && second.ended()) {
            rt.park();
        }
        assert_eq!(
            reached,
            a.reached(),
            "{what}: the vacuum's steps before the racer's first"
        );
        assert_eq!(rows(&ITEMS), first.reply(), "{what}");
        let alone = match racer {
            Racer::Same => rows(&NOTHING),
            Racer::Other => rows(&OTHERS),
        };
        assert_eq!(alone, second.reply(), "{what}");
        db.close().expect("the database closes");
    });
}

fn rows(rows: &[&str]) -> Reply {
    Ok(rows.iter().map(|r| (*r).to_owned()).collect())
}

/// Check `r`, a run with the racer started at the vacuum's pace point `at`:
///
/// - the racer reaches no pace point, pauses for the vacuum lock, and is refused at its deadline:
///   it waits for the lock the whole time the vacuum is stopped;
/// - the vacuum frees what the deletes left, its versions copied to history and their chunks
///   swept;
/// - run after, `vacuum items` frees nothing, and `vacuum others` frees all of its relation's
///   work;
/// - `verify` finds nothing.
#[track_caller]
fn check(racer: Racer, at: Step, r: &Ran) {
    let what = format!("{racer:?} at {at:?}");
    assert_eq!(Second::Whole, r.racer.second(1), "{what}");
    assert!(r.racer.steps().is_empty(), "{what}: {:?}", r.racer.steps());
    assert_ne!(0, r.waits, "{what}");
    refused_as(
        r.raced.clone(),
        &Error::Rejected {
            reason: RejectReason::DeadlineExceeded,
        },
        "the call's deadline passed",
    );
    assert_eq!(rows(&ITEMS), r.vacuumed, "{what}");
    assert_eq!(rows(&NOTHING), r.after[0], "{what}");
    assert_eq!(rows(&OTHERS), r.after[1], "{what}");
    assert_eq!(Vec::<String>::new(), r.verified, "{what}");
}

/// Run `case` at the first pace point of the vacuum and at each whose kind differs from the one
/// before, with the racer run to its end or blocked. Returns the pace points it stopped at and
/// each pair run.
fn each_change(mut case: impl FnMut((usize, usize)) -> [Reached; 2]) -> (Vec<Step>, Vec<Pair>) {
    let [first, raced] = case((1, usize::MAX));
    assert!(first.stopped(), "the vacuum reaches a pace point");
    let reached = first.steps().to_vec();
    let mut stops = vec![reached[0]];
    let mut pairs = vec![(reached[0], raced.second(usize::MAX))];
    for i in (2..=reached.len()).filter(|&i| reached[i - 1] != reached[i - 2]) {
        let [first, raced] = case((i, usize::MAX));
        assert_eq!(
            reached,
            first.steps(),
            "the vacuum stopped at {i} reaches as it did"
        );
        stops.push(reached[i - 1]);
        pairs.push((reached[i - 1], raced.second(usize::MAX)));
    }
    (stops, pairs)
}

/// Run the cases of `racer` the profile asks, checking each: under the deep profile every pair,
/// else [`each_change`]. Returns the kinds of pace point the vacuum stopped at, in the order it
/// first reached them.
fn search(racer: Racer) -> Vec<Vacuum> {
    let mut kinds = Vec::new();
    for seed in sweep(SEEDS) {
        let mut case = |pair: (usize, usize)| {
            let r = run(racer, seed, pair);
            if r.vacuum.stopped() {
                check(racer, r.vacuum.steps()[pair.0 - 1], &r);
                waits_out(racer, seed, pair.0);
            }
            [r.vacuum, r.racer]
        };
        let (stops, pairs) = if profile::every_case() {
            steps::every_pair(&mut case)
        } else {
            each_change(case)
        };
        assert!(pairs.iter().all(|(_, s)| *s == Second::Whole), "{pairs:?}");
        for stop in stops {
            let Step::Vacuum(kind) = stop else {
                panic!("the vacuum stops at a pace point, not {stop:?}");
            };
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
    }
    kinds
}

#[test]
fn a_second_vacuum_of_a_relation_waits_out_a_running_one_at_each_kind_of_pace_point() {
    assert_eq!(PACED.to_vec(), search(Racer::Same));
}

#[test]
fn a_vacuum_of_a_relation_sharing_the_store_waits_out_a_running_one_at_each_kind_of_pace_point() {
    assert_eq!(PACED.to_vec(), search(Racer::Other));
}
