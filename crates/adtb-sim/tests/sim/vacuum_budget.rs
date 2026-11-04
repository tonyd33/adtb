//! `VacuumBudget` on the simulated clock: the pace of `vacuum r`, a budget of 0, and the share
//! vacuum and the scrub take together.

use std::time::Duration;

use adtb::runtime::{Clock, Runtime, Threads};
use adtb::vacuum::Charged;
use adtb::{CallOptions, CancelToken, Database, Error, Log, OpenOptions, RejectReason};
use adtb_sim::SimRuntime;
use adtb_simkit::{api, common};

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

fn open(rt: &SimRuntime) -> Database<SimRuntime> {
    let options = OpenOptions {
        pool_bytes: Some(256 * 1024),
        ..api::options(2)
    };
    Database::open_in(rt.clone(), "budget.adt", &Log::new([SCHEMA]), &options)
        .expect("the database opens")
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

/// Returns the simulated time `vacuum items` takes at budget `io`, `cpu`.
fn timed(io: f64, cpu: f64) -> Duration {
    api::once(5, |rt| {
        let db = open(&rt);
        fill(&db);
        budget(&db, io, cpu);
        let began = rt.now();
        assert_eq!(DELETED, vacuum(&db), "io {io}, cpu {cpu}");
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
        let charged = db.write_path().pacer().charged(&rt);
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
                (0.75 / share..=1.5 / share).contains(&ratio),
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
    let allowed = together.mul_f64(0.1) + beside.longest * 2;
    assert!(
        beside.busy <= allowed,
        "{beside:?} over {together:?}, at most {allowed:?}"
    );
}

#[test]
fn a_budget_of_0_does_no_vacuum_work_until_the_call_is_cancelled() {
    api::once(6, |rt| {
        let db = open(&rt);
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
        let db = open(&rt);
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

/// Returns a database of `WIDE_ROWS` rows of `WIDE`, all but every third deleted.
fn wide(rt: &SimRuntime) -> Database<SimRuntime> {
    let options = OpenOptions {
        pool_bytes: Some(256 * 1024),
        ..api::options(2)
    };
    let db = Database::open_in(rt.clone(), "wide.adt", &Log::new([WIDE]), &options)
        .expect("the database opens");
    for from in (1..=WIDE_ROWS).step_by(100) {
        let text: String = (from..from + 100)
            .map(|k| {
                let c = ["A { a", "B { b"][(k % 2) as usize];
                let t = "x".repeat(if k % 10 == 0 { 3000 } else { 1 });
                format!(
                    "insert items.{c}: {k}, id: items.key({k}), k: \"{k:0>150}\", t: \"{t}\" }}\n"
                )
            })
            .collect();
        api::results(&db, &text);
    }
    api::results(&db, "delete i in items where value(key i) % 3 != 0");
    db
}

#[test]
fn no_step_of_a_vacuum_runs_longer_than_one_unit_of_work() {
    api::once(8, |rt| {
        let db = wide(&rt);
        budget(&db, 1.0, 1.0);
        let began = rt.now();
        let rows = vacuumed(&db);
        let took = rt.now().saturating_since(began);
        let longest = db.write_path().pacer().charged(&rt).longest;
        assert_eq!(1000, count(&rows, "removed: "));
        assert!(
            count(&rows, "pages, ") > 0,
            "the sweep freed chunks: {rows:?}"
        );
        assert!(
            longest <= took / 16,
            "the longest step took {longest:?} of {took:?}"
        );
    });
}

#[test]
fn a_long_wait_at_a_budget_of_0_opens_no_io_queue_per_sleep() {
    api::once(9, |rt| {
        let db = open(&rt);
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
    });
}
