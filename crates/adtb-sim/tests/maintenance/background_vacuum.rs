//! Background vacuum on the simulator: its trigger, the retention window's clock, its yield to a
//! waiting command, what it never writes, a close and a crash while it runs, and the snapshots
//! it leaves readable.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use adtb::runtime::{Clock, Entropy, Runtime, Threads};
use adtb::storage::{self, Lsn};
use adtb::testing::Plant;
use adtb::write::panic_text;
use adtb::{
    CallOptions, Database, Error, Log, OpenOptions, RejectReason, TxnCallOptions, TxnOptions,
};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_simkit::open;
use adtb_simkit::{api, common, crash, seeds};

const SCHEMA: &str = r"module app
add module app
add type Item { id: Key items, n: I64 }
  = A { a: I64 }
  | B { b: I64 }
add relation items : Item key id generate sequence
  index (n)
add type Other { id: Key others, n: I64 } = O { o: I64 }
add relation others : Other key id generate sequence
add type Wide { id: Key wides, t: Text } = W { w: I64 }
add relation wides : Wide key id generate sequence
";

const NAME: &str = "background.adt";

fn pooled() -> OpenOptions {
    OpenOptions {
        pool_bytes: Some(256 * 1024),
        ..open::options(2)
    }
}

/// Insert `rows` rows into `items` and into `others`, 100 to a call.
fn fill(db: &Database<SimRuntime>, rows: i64) {
    for from in (1..=rows).step_by(100) {
        let text: String = (from..=rows.min(from + 99))
            .map(|k| {
                let c = ["A { a", "B { b"][(k % 2) as usize];
                format!(
                    "insert items.{c}: {k}, id: items.key({k}), n: {k} }}\ninsert others.O {{ o: 0, id: others.key({k}), n: {k} }}\n"
                )
            })
            .collect();
        api::results(db, &text);
    }
}

fn budget(db: &Database<SimRuntime>, io: f64) {
    api::results(
        db,
        &format!("set setting VacuumBudget = {{ io: {io:?}, cpu: 1.0 }}"),
    );
}

const BUMP: &str = "update i in items.A set A { i with n: i.n + 1 }
update i in items.B set B { i with n: i.n + 1 }";

/// Returns the LSN of each page of `heaps`.
fn pages(db: &Database<SimRuntime>, heaps: &[(&str, &str)]) -> Vec<Lsn> {
    let t = db.testing();
    heaps
        .iter()
        .flat_map(|(r, c)| t.pages(t.heap(r, c).unwrap()).unwrap())
        .map(|p| storage::lsn(&t.probe().page(p).unwrap()))
        .collect()
}

const ITEMS: [(&str, &str); 2] = [("app::items", "A"), ("app::items", "B")];

fn sleep(rt: &SimRuntime, ms: u64) {
    common::sleep(rt, &mut rt.io(), Duration::from_millis(ms));
}

fn running(db: &Database<SimRuntime>) -> bool {
    !api::query(db, "from v in sys::vacuums select v").is_empty()
}

/// Returns `db` once a background vacuum of 1500 deleted rows of `items` runs at a budget of 0.01.
fn vacuuming(rt: &SimRuntime) -> Database<SimRuntime> {
    let db = open::opened(rt, NAME, &Log::new([SCHEMA]), &pooled());
    budget(&db, 0.0);
    fill(&db, 1500);
    api::results(&db, "delete i in items where true");
    budget(&db, 0.01);
    started(rt, &db);
    db
}

/// Wait until a background vacuum of `db` runs.
fn started(rt: &SimRuntime, db: &Database<SimRuntime>) {
    for _ in 0..200 {
        if running(db) {
            return;
        }
        sleep(rt, 10);
    }
    panic!("a background vacuum starts within 2 s");
}

/// Returns the pages of `items` after 200 rows, after 6 rounds of updating every row, or of
/// inserting 150 rows in a transaction that rolls back, and after 6 more.
fn grown(io: f64, aborted: bool) -> (usize, usize, usize) {
    api::once(21, |rt| {
        let db = open::opened(&rt, NAME, &Log::new([SCHEMA]), &pooled());
        budget(&db, io);
        fill(&db, 200);
        let one = pages(&db, &ITEMS).len();
        let rounds = || {
            for _ in 0..6 {
                if aborted {
                    let mut tx = db.begin(&TxnOptions::default()).unwrap();
                    let rows: String = (1000..1150)
                        .map(|k| format!("insert items.A {{ a: 0, id: items.key({k}), n: 0 }}\n"))
                        .collect();
                    api::in_txn_results(&mut tx, &rows);
                } else {
                    api::results(&db, BUMP);
                }
                sleep(&rt, 500);
            }
            pages(&db, &ITEMS).len()
        };
        (one, rounds(), rounds())
    })
}

#[test]
fn dead_versions_leave_with_no_command() {
    for aborted in [false, true] {
        let (one, k, two_k) = grown(0.1, aborted);
        let what = format!("aborted {aborted}: {two_k} pages after 12 rounds, {k} after 6");
        assert!(two_k <= k + one, "{what}");
        let (one, k, two_k) = grown(0.0, aborted);
        assert!(two_k > k + one, "with no budget, {what}");
    }
}

#[test]
fn versions_past_the_retention_window_leave_with_no_further_writes() {
    api::once(22, |rt| {
        let db = open::opened(&rt, NAME, &Log::new([SCHEMA]), &pooled());
        api::results(&db, "set setting RetentionWindow = { length: 2s }");
        fill(&db, 100);
        for _ in 0..3 {
            api::results(&db, BUMP);
        }
        sleep(&rt, 1000);
        let kept = || {
            let t = db.testing();
            (ITEMS.iter())
                .map(|(r, c)| t.pages(t.history(r, c).unwrap()).unwrap().len())
                .sum::<usize>()
        };
        assert!(kept() > 0, "the window keeps the old versions");
        sleep(&rt, 5000);
        assert_eq!(
            0,
            kept(),
            "every kept version left once the window passed it"
        );
        let steps = db.testing().charged().steps;
        sleep(&rt, 2000);
        let after = db.testing().charged().steps;
        assert_eq!(
            steps, after,
            "an empty history makes the relation due no more"
        );
    });
}

#[test]
fn a_background_vacuum_gives_way_to_a_command_waiting_for_it() {
    let commands = [
        "verify",
        "vacuum others",
        "set setting VacuumBudget = { io: 0.01, cpu: 1.0 }",
    ];
    for seed in seeds::sweep(0..3) {
        api::once(seed, |rt| {
            let db = vacuuming(&rt);
            sleep(&rt, rt.next_u64() % 2000);
            assert!(
                running(&db),
                "seed {seed}: the background vacuum still runs"
            );
            let before = db.testing().rebuild_waits();
            api::results(&db, commands[seed as usize % 3]);
            let waits = db.testing().rebuild_waits() - before;
            assert!(
                waits <= 40,
                "seed {seed}: {waits} waits of 5 ms for the lock"
            );
        });
    }
}

/// The tasks that wait for the vacuum lock.
const HOLDING: [&str; 5] = [
    "verify",
    "verify items",
    "vacuum others",
    "set setting VacuumBudget = { io: 0.01, cpu: 1.0 }",
    "reindex items",
];

#[test]
fn a_background_vacuum_gives_way_to_a_first_task_waiting_for_it() {
    let mut sim = Sim::new(25, Config::default());
    api::run(&mut sim, |rt| {
        let db = open::opened(&rt, NAME, &Log::new([SCHEMA]), &pooled());
        budget(&db, 0.0);
        fill(&db, 200);
        api::results(&db, "delete i in items where true");
        db.close().unwrap();
    });
    let files = sim.files();
    let ran: Vec<(&str, Result<bool, String>)> = (0..)
        .zip(HOLDING)
        .map(|(seed, text)| {
            let ran = catch_unwind(AssertUnwindSafe(|| {
                let mut sim = Sim::with_files(seed, Config::default(), files.clone());
                api::run(&mut sim, |rt| {
                    let db = open::opened(&rt, NAME, &Log::new([SCHEMA]), &pooled());
                    budget(&db, 0.01);
                    started(&rt, &db);
                    let owner = db.role_authority().session("owner").unwrap();
                    let asked = db.testing().vacuum_yields();
                    api::task(&rt, &owner, text).map(|()| db.testing().vacuum_yields() > asked)
                })
            }));
            let ran = match ran {
                Ok(done) => done.map_err(|e| format!("{e:?}")),
                Err(payload) => Err(panic_text(&*payload).unwrap_or_default()),
            };
            (text, ran)
        })
        .collect();
    let want: Vec<_> = HOLDING.iter().map(|&t| (t, Ok(true))).collect();
    assert_eq!(want, ran);
}

#[test]
fn a_background_vacuum_writes_no_page_of_a_quarantined_heap() {
    api::once(23, |rt| {
        let db = open::opened(&rt, NAME, &Log::new([SCHEMA]), &pooled());
        budget(&db, 0.0);
        fill(&db, 300);
        api::results(
            &db,
            "delete i in items where true\ndelete o in others where true",
        );
        let t = db.testing();
        let a = t.mark_of(t.heap("app::items", "A").unwrap()).unwrap();
        t.plant(Plant::Mark(a)).unwrap();
        let (items, others) = (pages(&db, &ITEMS), pages(&db, &[("app::others", "O")]));
        let waits = db.testing().rebuild_waits();
        budget(&db, 1.0);
        assert_eq!(
            waits,
            db.testing().rebuild_waits(),
            "no vacuum holds the lock at a budget of 0"
        );
        sleep(&rt, 3000);
        assert_ne!(
            others,
            pages(&db, &[("app::others", "O")]),
            "others is vacuumed"
        );
        assert_eq!(
            items,
            pages(&db, &ITEMS),
            "no page of a quarantined relation changes"
        );
    });
}

#[test]
fn a_close_during_a_background_vacuum_returns_promptly_and_leaves_it_sound() {
    let mut sim = Sim::new(24, Config::default());
    api::run(&mut sim, |rt| {
        let db = vacuuming(&rt);
        let (held, began) = (db.clone(), rt.now());
        db.close().unwrap();
        let took = rt.now().saturating_since(began);
        assert!(took < Duration::from_millis(500), "the close took {took:?}");
        let steps = held.testing().charged().steps;
        sleep(&rt, 1000);
        let after = held.testing().charged().steps;
        assert_eq!(steps, after, "no vacuum step runs once the close returns");
    });
    api::run(&mut sim, |rt| {
        let db = open::opened(&rt, NAME, &Log::new([SCHEMA]), &pooled());
        assert_eq!(Vec::<String>::new(), api::printed(db.verify(None).unwrap()));
    });
}

#[test]
fn a_file_after_a_crash_with_the_background_vacuum_running_holds_clean() {
    crash::silence_fatal_panics();
    for seed in seeds::sweep(0..5) {
        let mut sim = Sim::new(seed, Config::default());
        api::run(&mut sim, |rt| {
            open::opened(&rt, NAME, &Log::new([SCHEMA]), &pooled())
                .close()
                .unwrap();
        });
        sim.crash_at(sim.now() + crash::crash_after(seed, 1, 200_000, 2_000_000));
        let steps = Arc::new(AtomicU64::new(0));
        let seen = steps.clone();
        let outcome = crash::run_process(&mut sim, |rt| {
            let db = open::opened(&rt, NAME, &Log::new([SCHEMA]), &pooled());
            budget(&db, 1.0);
            let o = CallOptions::default();
            let mut n = 0_i64;
            loop {
                let t = format!("insert others.O {{ o: 0, id: others.key({n}), n: {n} }}");
                db.query_text(&t, &[], &o).ok();
                let gone = format!("delete o in others where o.n < {}", n - 20);
                db.query_text(&gone, &[], &o).ok();
                let charged = db.testing().charged().steps;
                seen.store(charged, Ordering::Relaxed);
                n += 1;
            }
        });
        assert_eq!(Outcome::Crashed, outcome, "seed {seed}");
        let steps = steps.load(Ordering::Relaxed);
        assert!(
            steps > 0,
            "seed {seed}: the background vacuum ran before the crash"
        );
        api::run(&mut sim, |rt| {
            let db = open::opened(&rt, NAME, &Log::new([SCHEMA]), &pooled());
            let found = api::printed(db.verify(None).unwrap());
            assert_eq!(Vec::<String>::new(), found, "seed {seed}");
        });
    }
}

/// The writes of the snapshot search, one of them by each turn of each writer. The last rolls
/// back.
const WRITES: [&str; 5] = [
    BUMP,
    "update i in items.A where i.n % 4 == 0 set B { b: i.a }",
    "delete i in items where i.n % 9 == 0",
    "insert items.A { a: 0, id: items.key(NEXT), n: NEXT }",
    "insert items.B { b: 0, id: items.key(NEXT), n: NEXT }",
];

/// Run write `at` of `WRITES`, its key `next`. Returns whether it ran, rolled back or not.
fn write(db: &Database<SimRuntime>, at: usize, next: u64) -> bool {
    let text = WRITES[at].replace("NEXT", &next.to_string());
    if at + 1 < WRITES.len() {
        return db.query_text(&text, &[], &CallOptions::default()).is_ok();
    }
    let Ok(mut tx) = db.begin(&TxnOptions::default()) else {
        return false;
    };
    tx.query_text(&text, &[], &TxnCallOptions::default())
        .is_ok()
}

#[test]
fn a_background_vacuum_removes_no_version_a_transaction_reads() {
    let done = [(); 5].map(|()| Arc::new(AtomicUsize::new(0)));
    let mut overlapped = 0;
    for seed in seeds::sweep(0..4) {
        api::once(seed, |rt| {
            let db = open::opened(&rt, NAME, &Log::new([SCHEMA]), &pooled());
            budget(&db, 1.0);
            fill(&db, 60);
            let writers: Vec<_> = (0..2_u64)
                .map(|w| {
                    let (db, done) = (db.clone(), done.clone());
                    rt.spawn("writer", move || {
                        for r in 0..40 {
                            let at = ((w + r) % 5) as usize;
                            if write(&db, at, 1000 + w * 100 + r) {
                                done[at].fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    })
                })
                .collect();
            let read = "from i in items select i";
            let read_only = TxnOptions {
                read_only: true,
                ..TxnOptions::default()
            };
            for r in 0..8 {
                let mut tx = db.begin(&read_only).unwrap();
                let first = api::in_txn(&mut tx, read);
                let steps = db.testing().charged().steps;
                for _ in 0..5 {
                    sleep(&rt, 50 + rt.next_u64() % 100);
                    assert_eq!(first, api::in_txn(&mut tx, read), "seed {seed}, read {r}");
                }
                let ran = db.testing().charged().steps > steps;
                let now = api::query(&db, read);
                overlapped += usize::from(ran && first.iter().any(|row| !now.contains(row)));
                tx.commit(None).unwrap();
            }
            for w in writers {
                rt.join(w);
            }
            let found = api::printed(db.verify(None).unwrap());
            assert_eq!(Vec::<String>::new(), found, "seed {seed}");
        });
    }
    for (write, done) in WRITES.iter().zip(&done) {
        seeds::assert_reached(write, done.load(Ordering::Relaxed), 1);
    }
    seeds::assert_reached(
        "a worker step inside a snapshot whose rows were later deleted or retagged",
        overlapped,
        1,
    );
}

#[test]
fn the_background_vacuum_and_a_scrub_pass_together_stay_within_the_budget() {
    api::once(25, |rt| {
        let db = open::opened(&rt, NAME, &Log::new([SCHEMA]), &pooled());
        budget(&db, 0.0);
        fill(&db, 2000);
        api::results(&db, "delete i in items where true");
        api::results(
            &db,
            "set setting ScrubInterval = { every: Some { value: 1s } }",
        );
        budget(&db, 0.1);
        let (mut vacuumed, mut scrubbed) = (false, false);
        for _ in 0..3000 {
            let now = running(&db);
            scrubbed |= now && !api::query(&db, "from j in sys::jobs select j").is_empty();
            if vacuumed && !now {
                break;
            }
            vacuumed |= now;
            sleep(&rt, 20);
        }
        assert!(
            vacuumed && scrubbed,
            "a scrub pass ran beside the background vacuum"
        );
        let charged = db.testing().charged();
        let window = rt
            .now()
            .saturating_since(charged.since.expect("a step was charged"));
        let allowed = window.mul_f64(0.1) + charged.longest * 2;
        assert!(
            charged.busy <= allowed,
            "{charged:?} over {window:?}, at most {allowed:?}"
        );
    });
}

/// Returns the versions `vacuum REL` removed.
fn removed(db: &Database<SimRuntime>, rel: &str) -> i64 {
    let rows = api::printed(api::results(db, &format!("vacuum {rel}")).remove(0));
    rows.iter()
        .map(|r| {
            let at = r.find("removed: ").expect("a vacuum row has `removed`") + 9;
            let digits: String = r[at..].chars().take_while(char::is_ascii_digit).collect();
            digits.parse::<i64>().expect("a count")
        })
        .sum()
}

#[test]
fn a_relation_with_all_visible_pages_is_due_at_its_page_count() {
    api::once(26, |rt| {
        let db = open::opened(&rt, NAME, &Log::new([SCHEMA]), &pooled());
        let t = "t".repeat(700);
        for from in (1..=600).step_by(100) {
            let text: String = (from..from + 100)
                .map(|k| format!("insert wides.W {{ w: 0, id: wides.key({k}), t: \"{t}\" }}\n"))
                .collect();
            api::results(&db, &text);
        }
        removed(&db, "wides");
        removed(&db, "wides");
        let pages = pages(&db, &[("app::wides", "W")]).len();
        assert!((50..400).contains(&pages), "{pages} pages");
        let dead = pages * 3 / 2;
        api::results(
            &db,
            &format!("delete w in wides where value(key w) <= {dead}"),
        );
        sleep(&rt, 3000);
        assert_eq!(
            0,
            removed(&db, "wides"),
            "{dead} dead versions over {pages} pages"
        );
    });
}

#[test]
fn dead_versions_a_reader_holds_stay_counted_until_it_ends() {
    api::once(27, |rt| {
        let db = open::opened(&rt, NAME, &Log::new([SCHEMA]), &pooled());
        budget(&db, 1.0);
        fill(&db, 200);
        sleep(&rt, 1000);
        let read_only = TxnOptions {
            read_only: true,
            ..TxnOptions::default()
        };
        let mut tx = db.begin(&read_only).unwrap();
        assert_eq!(200, api::in_txn(&mut tx, "from i in items select i").len());
        let steps = db.testing().charged().steps;
        api::results(&db, "delete i in items where true");
        sleep(&rt, 2000);
        assert!(
            db.testing().charged().steps > steps,
            "a vacuum ran while the reader held its versions"
        );
        tx.commit(None).unwrap();
        sleep(&rt, 70_000);
        assert_eq!(
            0,
            removed(&db, "items"),
            "the worker removed them once the reader ended"
        );
    });
}

/// Assert `quiet` is vacuumed of its deleted rows while `churn` runs during each background
/// vacuum, so the relation it writes is due again as each one ends.
fn vacuumed_beside(churn: &'static str, quiet: &str) {
    api::once(28, |rt| {
        let db = open::opened(&rt, NAME, &Log::new([SCHEMA]), &pooled());
        budget(&db, 1.0);
        fill(&db, 1500);
        sleep(&rt, 2000);
        budget(&db, 0.01);
        let stop = Arc::new(AtomicUsize::new(0));
        let churned = Arc::new(AtomicUsize::new(0));
        let (churning, stopping, count, clock) =
            (db.clone(), stop.clone(), churned.clone(), rt.clone());
        let t = rt.spawn("churn", move || {
            while stopping.load(Ordering::Relaxed) == 0 {
                if running(&churning) {
                    api::results(&churning, churn);
                    count.fetch_add(1, Ordering::Relaxed);
                    while running(&churning) {
                        sleep(&clock, 2);
                    }
                }
                sleep(&clock, 2);
            }
        });
        api::results(&db, BUMP);
        api::results(&db, &format!("delete q in {quiet} where true"));
        sleep(&rt, 5000);
        let left = removed(&db, quiet);
        stop.store(1, Ordering::Relaxed);
        rt.join(t);
        seeds::assert_reached(
            "a write during a background vacuum",
            churned.load(Ordering::Relaxed),
            1,
        );
        assert_eq!(0, left, "{quiet} is vacuumed beside `{churn}`");
    });
}

#[test]
fn a_relation_due_beside_one_that_churns_is_still_vacuumed() {
    vacuumed_beside(BUMP, "others");
}

#[test]
fn versions_a_failed_call_inserted_count_as_dead() {
    api::once(29, |rt| {
        let db = open::opened(&rt, NAME, &Log::new([SCHEMA]), &pooled());
        budget(&db, 1.0);
        fill(&db, 1);
        sleep(&rt, 1000);
        let mut text: String = (1000..1300)
            .map(|k| format!("insert items.A {{ a: 0, id: items.key({k}), n: 0 }}\n"))
            .collect();
        text.push_str("insert items.A { a: 0, id: items.key(1), n: 0 }");
        let failed = db.query_text(&text, &[], &CallOptions::default());
        let Err(Error::Rejected {
            reason: RejectReason::KeyViolation { relation, key },
        }) = failed
        else {
            panic!("the last item repeats a key: {failed:?}");
        };
        assert_eq!(("app::items", "items.key(1)"), (&*relation, &*key));
        sleep(&rt, 3000);
        assert_eq!(
            0,
            removed(&db, "items"),
            "the worker removed the taken back versions"
        );
    });
}
