//! `VacuumBudget` on the simulated clock: the pace of `vacuum r`, a budget of 0, and the share
//! vacuum and the scrub take together.

use std::collections::BTreeSet;
use std::time::Duration;

use adtb::runtime::{Clock, Runtime, Threads};
use adtb::testing::Knobs;
use adtb::vacuum::Charged;
use adtb::{CallOptions, CancelToken, Database, Error, Log, OpenOptions, RejectReason};
use adtb_harness::common;
use adtb_sim::SimRuntime;
use adtb_simkit::api;
use adtb_simkit::open;

const SCHEMA: &str = r"module app
add module app
add type Item { id: Key items, n: I64 }
  = A { a: I64 }
  | B { b: I64 }
add relation items : Item key id generate sequence
  index (n)
";

const ROWS: i64 = 200;

/// The rows `fill` deletes.
const DELETED: i64 = ROWS - ROWS / 3;

fn pooled() -> OpenOptions {
    OpenOptions {
        pool_bytes: Some(256 * 1024),
        ..open::options(2)
    }
}

/// Insert `ROWS` rows and delete all but every third.
fn fill(db: &Database<SimRuntime>) {
    let text: String = (1..=ROWS)
        .map(|k| {
            let c = ["A { a", "B { b"][(k % 2) as usize];
            format!("insert items.{c}: {k}, id: items.key({k}), n: {k} }}\n")
        })
        .collect();
    api::results(db, &text);
    api::results(db, "delete i in items where value(key i) % 3 != 0");
}

fn budget(db: &Database<SimRuntime>, io: f64, cpu: f64) {
    let text = format!("set setting VacuumBudget = {{ io: {io:?}, cpu: {cpu:?} }}");
    api::results(db, &text);
}

/// Returns the sum over `rows` of the number after `label` in each.
fn count(rows: &[String], label: &str) -> i64 {
    rows.iter()
        .map(|r| {
            let at = r.find(label).expect("a vacuum row has the label") + label.len();
            let end = at
                + r[at..]
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(r.len() - at);
            r[at..end].parse::<i64>().expect("a count")
        })
        .sum()
}

fn vacuumed(db: &Database<SimRuntime>) -> Vec<String> {
    api::printed(api::results(db, "vacuum items").remove(0))
}

/// Returns the versions `vacuum items` removed, summed over its rows.
fn vacuum(db: &Database<SimRuntime>) -> i64 {
    count(&vacuumed(db), "removed: ")
}

/// Returns the simulated time `vacuum items` of `wide` takes at budget `io`, `cpu`.
fn timed(io: f64, cpu: f64) -> Duration {
    api::once(5, |rt| {
        let db = wide(&rt);
        budget(&db, io, cpu);
        let began = rt.now();
        assert_eq!(1000, vacuum(&db), "io {io}, cpu {cpu}");
        rt.now().saturating_since(began)
    })
}

/// Returns what the pacer was charged by `vacuum items` of `wide` at a budget of 0.1, beside a
/// scrub pass when `scrubbing`, and the time from its first step to the end.
fn shared(scrubbing: bool) -> (Charged, Duration) {
    api::once(5, |rt| {
        let db = wide(&rt);
        if scrubbing {
            budget(&db, 0.2, 1.0);
            api::results(
                &db,
                "set setting ScrubInterval = { every: Some { value: 1s } }",
            );
            common::sleep(&rt, &mut rt.io(), Duration::from_secs(2));
            assert_eq!(1, api::query(&db, JOBS).len(), "a scrub pass runs");
        }
        budget(&db, 0.1, 1.0);
        assert_eq!(1000, vacuum(&db));
        if scrubbing {
            assert_eq!(1, api::query(&db, JOBS).len(), "the pass ran throughout");
        }
        let charged = db.testing().charged();
        let since = charged.since.expect("a step was charged");
        (charged, rt.now().saturating_since(since))
    })
}

const JOBS: &str = "from j in sys::jobs select j";

#[test]
fn vacuum_takes_the_least_fraction_of_the_time_it_runs() {
    let full = timed(1.0, 1.0).as_secs_f64();
    assert!(full > 0.0, "a vacuum reading pages takes simulated time");
    for b in [0.5_f64, 0.05] {
        for (io, cpu) in [(b, 1.0), (1.0, b), (b, b / 2.0)] {
            let share = io.min(cpu);
            let ratio = timed(io, cpu).as_secs_f64() / full;
            assert!(
                (0.9 / share..=1.1 / share).contains(&ratio),
                "io {io}, cpu {cpu}: {ratio} times as long as at 1.0"
            );
        }
    }
}

#[test]
fn vacuum_and_a_scrub_pass_together_stay_within_the_budget() {
    let (alone, window) = shared(false);
    let (beside, together) = shared(true);
    assert!(
        alone.busy >= window.mul_f64(0.05),
        "a vacuum alone uses its budget: {alone:?} over {window:?}"
    );
    assert!(
        beside.steps > alone.steps,
        "the scrub charges the vacuum's pacer: {beside:?} beside, {alone:?} alone"
    );
    assert!(
        together <= window * 3,
        "beside a scrub pass the vacuum took {together:?}, alone {window:?}"
    );
    let allowed = together.mul_f64(0.1) + beside.longest * 2;
    assert!(
        beside.busy <= allowed,
        "{beside:?} over {together:?}, at most {allowed:?}"
    );
}

#[test]
fn a_budget_of_0_does_no_vacuum_work_until_the_call_is_cancelled() {
    api::once(6, |rt| {
        let db = open::opened(&rt, "budget.adt", &Log::new([SCHEMA]), &pooled());
        fill(&db);
        budget(&db, 0.0, 1.0);
        let token = CancelToken::new();
        let (canceller, clock) = (token.clone(), rt.clone());
        let after = Duration::from_secs(2);
        let t = rt.spawn("canceller", move || {
            common::sleep(&clock, &mut clock.io(), after);
            canceller.cancel();
        });
        let began = rt.now();
        let options = CallOptions {
            cancel: Some(token),
            ..CallOptions::default()
        };
        let got = db.query_text("vacuum items", &[], &options).map(|_| ());
        rt.join(t);
        assert!(
            matches!(
                got,
                Err(Error::Rejected {
                    reason: RejectReason::Cancelled
                })
            ),
            "{got:?}"
        );
        assert!(rt.now().saturating_since(began) >= after);
        budget(&db, 1.0, 1.0);
        assert_eq!(DELETED, vacuum(&db), "the stopped vacuum removed nothing");
    });
}

#[test]
fn a_close_stops_a_vacuum_waiting_on_its_budget() {
    api::once(7, |rt| {
        let db = open::opened(&rt, "budget.adt", &Log::new([SCHEMA]), &pooled());
        fill(&db);
        budget(&db, 0.0, 1.0);
        let vacuuming = db.clone();
        let t = rt.spawn("vacuum", move || {
            let got = vacuuming.query_text("vacuum items", &[], &CallOptions::default());
            assert!(
                matches!(
                    got,
                    Err(Error::Rejected {
                        reason: RejectReason::Cancelled
                    })
                ),
                "{:?}",
                got.map(|_| ())
            );
        });
        common::sleep(&rt, &mut rt.io(), Duration::from_secs(1));
        db.close().expect("the database closes");
        rt.join(t);
    });
}

const WIDE: &str = r"module app
add module app
add type Item { id: Key items, k: Text, t: Text }
  = A { a: I64 }
  | B { b: I64 }
add relation items : Item key id generate sequence
  index (k)
";

/// The rows of `WIDE`, every tenth with a value stored out of line.
const WIDE_ROWS: i64 = 1500;

/// Returns a database of `WIDE_ROWS` rows of `WIDE`, all but every third deleted, the first
/// with a value of 400 KB when `big`, the deleted versions kept in history when `retained`.
fn wide_as(rt: &SimRuntime, big: bool, retained: bool) -> Database<SimRuntime> {
    let options = OpenOptions {
        pool_bytes: Some(256 * 1024),
        testing: Knobs::without_background_vacuum(),
        ..open::options(2)
    };
    let db = Database::open_in(rt.clone(), "wide.adt", &Log::new([WIDE]), &options)
        .expect("the database opens");
    for from in (1..=WIDE_ROWS).step_by(100) {
        let text: String = (from..from + 100)
            .map(|k| {
                let c = ["A { a", "B { b"][(k % 2) as usize];
                let len = match k {
                    1 if big => 400_000,
                    _ if k % 10 == 0 => 3000,
                    _ => 1,
                };
                let t = "x".repeat(len);
                format!(
                    "insert items.{c}: {k}, id: items.key({k}), k: \"{k:0>150}\", t: \"{t}\" }}\n"
                )
            })
            .collect();
        api::results(&db, &text);
    }
    if retained {
        api::results(&db, "set setting RetentionWindow = { length: 1h }");
    }
    api::results(&db, "delete i in items where value(key i) % 3 != 0");
    db
}

/// Returns a database of `WIDE` whose 1500 rows were each inserted by its own commit and all
/// deleted, the commits and the versions older than a retention window of 1 s.
fn expired(rt: &SimRuntime) -> Database<SimRuntime> {
    let options = OpenOptions {
        pool_bytes: Some(256 * 1024),
        testing: Knobs::without_background_vacuum(),
        ..open::options(2)
    };
    let db = Database::open_in(rt.clone(), "expired.adt", &Log::new([WIDE]), &options)
        .expect("the database opens");
    api::results(&db, "set setting RetentionWindow = { length: 1s }");
    for k in 1..=1500 {
        let text = format!("insert items.A {{ a: {k}, id: items.key({k}), k: \"{k}\", t: \"x\" }}");
        api::results(&db, &text);
    }
    api::results(&db, "delete i in items where true");
    common::sleep(rt, &mut rt.io(), Duration::from_secs(2));
    db
}

fn wide(rt: &SimRuntime) -> Database<SimRuntime> {
    wide_as(rt, false, false)
}

/// Returns the chunks `vacuum items` reports freed, the relation's count that every row repeats.
fn chunks(rows: &[String]) -> i64 {
    let each: BTreeSet<i64> = rows
        .iter()
        .map(|r| count(std::slice::from_ref(r), "pages, "))
        .collect();
    assert_eq!(
        1,
        each.len(),
        "every row names the relation's chunks: {rows:?}"
    );
    each.into_iter().sum()
}

/// Returns the simulated time `vacuum items` of `db` takes at a budget of 1.0, its rows, and what
/// the pacer was charged.
fn stepped(rt: &SimRuntime, db: &Database<SimRuntime>) -> (Duration, Vec<String>, Charged) {
    budget(db, 1.0, 1.0);
    let began = rt.now();
    let rows = vacuumed(db);
    let took = rt.now().saturating_since(began);
    (took, rows, db.testing().charged())
}

/// Assert that no step of a vacuum that took `took` ran longer than `parts`th of it, and that the
/// pacer was charged all but 2% of it.
fn paced_throughout(what: &str, took: Duration, charged: Charged, parts: u32) {
    assert!(
        charged.longest <= took / parts,
        "{what}: the longest step took {:?} of {took:?}",
        charged.longest
    );
    assert!(
        charged.busy >= took.mul_f64(0.98),
        "{what}: {:?} of {took:?} charged",
        charged.busy
    );
}

#[test]
fn no_step_of_a_vacuum_runs_longer_than_one_unit_of_work() {
    for (seed, big, retained) in [(8, false, false), (11, true, false), (12, false, true)] {
        let what = format!("big {big}, retained {retained}");
        api::once(seed, |rt| {
            let db = wide_as(&rt, big, retained);
            let (took, rows, charged) = stepped(&rt, &db);
            assert!(
                chunks(&rows) > 0,
                "{what}: the sweep freed chunks: {rows:?}"
            );
            if !retained {
                assert_eq!(1000, count(&rows, "removed: "), "{what}");
            }
            paced_throughout(&what, took, charged, 16);
        });
    }
}

#[test]
fn no_step_of_a_vacuum_expiring_history_runs_longer_than_one_unit_of_work() {
    api::once(14, |rt| {
        let db = expired(&rt);
        let (took, rows, charged) = stepped(&rt, &db);
        assert_eq!(1500, count(&rows, "removed: "));
        paced_throughout("expired", took, charged, 4);
    });
}

#[test]
fn a_long_wait_at_a_budget_of_0_opens_no_io_queue_per_sleep() {
    api::once(9, |rt| {
        let db = open::opened(&rt, "budget.adt", &Log::new([SCHEMA]), &pooled());
        fill(&db);
        budget(&db, 0.0, 1.0);
        let token = CancelToken::new();
        let (canceller, clock) = (token.clone(), rt.clone());
        let t = rt.spawn("canceller", move || {
            common::sleep(&clock, &mut clock.io(), Duration::from_secs(3600));
            canceller.cancel();
        });
        let options = CallOptions {
            cancel: Some(token),
            ..CallOptions::default()
        };
        let before = rt.queues();
        let got = db.query_text("vacuum items", &[], &options).map(|_| ());
        rt.join(t);
        assert!(
            matches!(
                got,
                Err(Error::Rejected {
                    reason: RejectReason::Cancelled
                })
            ),
            "{got:?}"
        );
        assert!(
            rt.queues() - before <= 2,
            "{} queues opened by an hour's wait",
            rt.queues() - before
        );
        let busy = db.testing().charged().busy;
        assert!(busy < Duration::from_secs(1), "the wait charged {busy:?}");
    });
}

/// Returns the steps the pacer was charged by `vacuum items` of a relation of `ctors`
/// constructors over the same rows, with a second index when `second`.
fn steps(ctors: usize, second: bool) -> u64 {
    let names: Vec<String> = (0..ctors)
        .map(|c| format!("C{c} {{ f{c}: I64 }}"))
        .collect();
    let schema = format!(
        "module app\nadd module app\n\
         add type Item {{ id: Key items, k: Text, m: Text }}\n  = {}\n\
         add relation items : Item key id generate sequence\n  index (k)\n{}",
        names.join("\n  | "),
        if second { "  index (m)\n" } else { "" }
    );
    api::once(10, |rt| {
        let options = OpenOptions {
            pool_bytes: Some(256 * 1024),
            ..open::options(2)
        };
        let db = Database::open_in(rt, "passes.adt", &Log::new([schema.as_str()]), &options)
            .expect("the database opens");
        for from in (1..=600).step_by(100) {
            let text: String = (from..from + 100)
                .map(|k| {
                    let c = k % ctors;
                    format!(
                        "insert items.C{c} {{ f{c}: {k}, id: items.key({k}), k: \"{k:0>150}\", m: \"{k:0>150}\" }}\n"
                    )
                })
                .collect();
            api::results(&db, &text);
        }
        api::results(&db, "delete i in items where value(key i) % 3 != 0");
        budget(&db, 1.0, 1.0);
        assert_eq!(400, vacuum(&db));
        db.testing().charged().steps
    })
}

#[test]
fn a_vacuum_walks_each_index_of_a_relation_once() {
    let one = steps(1, true) - steps(1, false);
    let three = steps(3, true) - steps(3, false);
    assert!(one >= 5, "the second index has leaves: {one} steps");
    assert!(
        three <= one + 2,
        "{three} steps for the second index of three constructors, {one} of one"
    );
}

/// Run `vacuum items` of `wide_as(retained)` at a budget of 0.5, cancelled, or the database
/// closed when `close`, once the pacer has been charged `at` steps when given. Returns the steps
/// charged by then, the I/O queues the vacuum opened and what it returned.
fn cancelled(retained: bool, at: Option<u64>, close: bool) -> (u64, usize, Result<(), Error>) {
    api::once(13, |rt| {
        let db = wide_as(&rt, false, retained);
        budget(&db, 0.5, 1.0);
        let token = CancelToken::new();
        let (canceller, clock, watched) = (token.clone(), rt.clone(), db.clone());
        let seen = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let into = seen.clone();
        let t = at.map(|at| {
            rt.spawn("canceller", move || {
                loop {
                    let steps = watched.testing().charged().steps;
                    if steps >= at {
                        if close {
                            watched.close().expect("the database closes");
                        } else {
                            canceller.cancel();
                        }
                        into.store(steps, std::sync::atomic::Ordering::Relaxed);
                        return;
                    }
                    common::sleep(&clock, &mut clock.io(), Duration::from_micros(20));
                }
            })
        });
        let options = CallOptions {
            cancel: Some(token),
            ..CallOptions::default()
        };
        let before = rt.queues();
        let got = db.query_text("vacuum items", &[], &options).map(|_| ());
        let queues = rt.queues() - before;
        let steps = match t {
            Some(t) => {
                rt.join(t);
                seen.load(std::sync::atomic::Ordering::Relaxed)
            }
            None => db.testing().charged().steps,
        };
        (steps, queues, got)
    })
}

#[test]
fn a_cancel_or_close_in_the_last_steps_of_a_vacuum_refuses_it() {
    for retained in [false, true] {
        let (steps, queues, got) = cancelled(retained, None, false);
        assert!(got.is_ok(), "{got:?}");
        assert!(
            queues <= 1,
            "retained {retained}: {queues} queues for one vacuum"
        );
        for close in [false, true] {
            let at = steps - 10;
            let (when, _, got) = cancelled(retained, Some(at), close);
            assert!(when < steps - 1, "cancelled at step {when} of {steps}");
            assert!(
                matches!(
                    got,
                    Err(Error::Rejected {
                        reason: RejectReason::Cancelled
                    })
                ),
                "retained {retained}, close {close}, stopped at step {when} of {steps}: {got:?}"
            );
        }
    }
}
