//! Indexes and uniques over expressions and over paths into value-level sum fields, over the
//! in-process API: building them by migration, keeping them through crashes, and checking them
//! between concurrent writers.

use std::sync::{Arc, Mutex};

use adtb::runtime::Threads;
use adtb::{
    AdHocTransaction, CallOptions, CreateOptions, Database, DynRows, Error, Log, OpenError,
    OpenOptions, RejectReason, TxnCallOptions, TxnOptions,
};
use adtb_lang::textual::print;
use adtb_sim::{Config, Outcome, Sim, SimRuntime};

const BILLING: &str = r"module billing
add module billing
add type Status = Draft | Paid { ref: Text }
add type Invoice = { id: Key invoices, status: Status, email: Text, n: I64 }
add relation invoices : Invoice key id generate sequence
  unique (status.Paid.ref)
  index (lower(email))
";

const LOWER_UNIQUE: &str = "module billing\nadd unique (lower(email)) in invoices\n";

const EMAILS: &str = "from i in invoices select i.email";

const SEEDS: std::ops::Range<u64> = 0..3;

fn options() -> OpenOptions {
    OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(2),
        ..OpenOptions::default()
    }
}

fn open_with(rt: &SimRuntime, log: &[&str]) -> Database<SimRuntime> {
    Database::open_in(
        rt.clone(),
        "app.adt",
        &Log::new(log.iter().copied()),
        &options(),
    )
    .expect("the database opens")
}

fn open(rt: &SimRuntime) -> Database<SimRuntime> {
    open_with(rt, &[BILLING])
}

fn run<T>(sim: &mut Sim, body: impl FnOnce(SimRuntime) -> T) -> T {
    let mut got = None;
    let outcome = sim.run(|rt| got = Some(body(rt)));
    assert_eq!(Outcome::Finished, outcome);
    got.expect("the run returns")
}

fn once<T>(seed: u64, body: impl FnOnce(SimRuntime) -> T) -> T {
    run(&mut Sim::new(seed, Config::default()), body)
}

fn printed(rows: DynRows) -> Vec<String> {
    let form = rows.form().clone();
    let mut out: Vec<String> = rows
        .map(|r| print(&r.expect("a row"), &form).expect("a row prints"))
        .collect();
    out.sort();
    out
}

fn query(db: &Database<SimRuntime>, text: &str) -> Vec<String> {
    let mut out = db
        .query_text(text, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"));
    assert_eq!(1, out.len(), "`{text}` has one result");
    printed(out.remove(0))
}

fn write(db: &Database<SimRuntime>, text: &str) -> Result<(), Error> {
    db.query_text(text, &[], &CallOptions::default()).map(drop)
}

fn in_txn(t: &mut AdHocTransaction<'_, SimRuntime>, text: &str) -> Result<(), Error> {
    t.query_text(text, &[], &TxnCallOptions::default())
        .map(drop)
}

fn insert(email: &str, status: &str) -> String {
    format!(r#"insert invoices {{ status: {status}, email: "{email}", n: 0 }}"#)
}

fn paid(r: &str) -> String {
    format!(r#"Paid {{ ref: "{r}" }}"#)
}

fn is_violated(e: &Error) -> bool {
    matches!(
        e,
        Error::Rejected {
            reason: RejectReason::Violated { .. }
        }
    )
}

/// Returns the emails of the invoices whose lowered email is `lowered`, read through the
/// expression index.
fn by_lower(db: &Database<SimRuntime>, lowered: &str) -> Vec<String> {
    query(
        db,
        &format!(r#"from i in invoices where lower(i.email) == "{lowered}" select i.email"#),
    )
}

/// Returns the emails of the invoices paid with reference `r`, read through the path index.
fn by_ref(db: &Database<SimRuntime>, r: &str) -> Vec<String> {
    query(
        db,
        &format!(
            r#"from i in invoices where (match i.status | Paid p -> p.ref == "{r}" | Draft -> false) select i.email"#
        ),
    )
}

#[test]
fn an_expression_index_finds_every_row_whose_expression_equals_the_value() {
    for seed in SEEDS {
        once(seed, |rt| {
            let db = open(&rt);
            for e in ["Ann@X", "bo@x", "ANN@x"] {
                write(&db, &insert(e, "Draft")).expect("the insert");
            }
            assert_eq!(
                vec![r#""ANN@x""#, r#""Ann@X""#],
                by_lower(&db, "ann@x"),
                "seed {seed}"
            );
            write(
                &db,
                r#"update i in invoices where i.email == "Ann@X" set Invoice { i with email: "cy@x" }"#,
            )
            .expect("the update");
            assert_eq!(vec![r#""ANN@x""#], by_lower(&db, "ann@x"), "seed {seed}");
            assert_eq!(vec![r#""cy@x""#], by_lower(&db, "cy@x"), "seed {seed}");
            write(
                &db,
                r#"update i in invoices where i.email == "bo@x" set Invoice { i with n: 7 }"#,
            )
            .expect("an update of a field the expression does not read");
            assert_eq!(vec![r#""bo@x""#], by_lower(&db, "bo@x"), "seed {seed}");
        });
    }
}

#[test]
fn a_path_index_holds_only_rows_whose_field_holds_its_constructor() {
    once(0, |rt| {
        let db = open(&rt);
        write(&db, &insert("a@x", "Draft")).expect("a draft");
        write(&db, &insert("b@x", &paid("r1"))).expect("a paid invoice");
        write(&db, &insert("c@x", "Draft")).expect("another draft");
        assert_eq!(vec![r#""b@x""#], by_ref(&db, "r1"));
        write(
            &db,
            r#"update i in invoices where i.email == "b@x" set Invoice { i with status: Draft }"#,
        )
        .expect("the retag of the field");
        assert!(by_ref(&db, "r1").is_empty());
        write(&db, &insert("d@x", &paid("r1"))).expect("the reference is free again");
        assert_eq!(vec![r#""d@x""#], by_ref(&db, "r1"));
    });
}

#[test]
fn an_expression_unique_added_over_rows_sharing_its_value_is_refused() {
    let mut sim = Sim::new(1, Config::default());
    run(&mut sim, |rt| {
        let db = open(&rt);
        write(&db, &insert("Ann@X", "Draft")).expect("the first");
        write(&db, &insert("ANN@x", "Draft")).expect("the second");
        db.close().expect("close");
    });
    run(&mut sim, |rt| {
        let log = Log::new([BILLING, LOWER_UNIQUE]);
        match Database::open_in(rt, "app.adt", &log, &options()) {
            Err(OpenError::MigrationRefused { version: 2, .. }) => {}
            other => panic!("not refused: {other:?}"),
        }
    });
}

#[test]
fn an_expression_unique_added_by_migration_holds_for_later_writes() {
    let mut sim = Sim::new(2, Config::default());
    run(&mut sim, |rt| {
        let db = open(&rt);
        write(&db, &insert("Ann@X", "Draft")).expect("the first");
        write(&db, &insert("bo@x", "Draft")).expect("the second");
        db.close().expect("close");
    });
    run(&mut sim, |rt| {
        let db = open_with(&rt, &[BILLING, LOWER_UNIQUE]);
        let got = write(&db, &insert("aNN@x", "Draft")).expect_err("a duplicate lowered email");
        assert!(is_violated(&got), "{got:?}");
        write(&db, &insert("cy@x", "Draft")).expect("a new lowered email");
        assert_eq!(vec![r#""Ann@X""#], by_lower(&db, "ann@x"));
    });
}

#[test]
fn a_migration_adding_an_expression_unique_survives_a_crash_whole_or_not_at_all() {
    let seed = 3;
    let mut sim = Sim::new(seed, Config::default());
    run(&mut sim, |rt| {
        let db = open(&rt);
        for e in ["Ann@X", "bo@x", "cy@x"] {
            write(&db, &insert(e, "Draft")).expect("the insert");
        }
        db.close().expect("close");
    });
    sim.record();
    run(&mut sim, |rt| {
        std::mem::forget(open_with(&rt, &[BILLING, LOWER_UNIQUE]));
    });
    let recording = sim.recording();
    for at in 0..=recording.len() {
        let files = recording.images(at, 1, seed + at as u64).swap_remove(0);
        let mut sim = Sim::with_files(seed + at as u64, Config::default(), files);
        run(&mut sim, |rt| {
            let db = open_with(&rt, &[BILLING, LOWER_UNIQUE]);
            assert_eq!(
                vec![r#""Ann@X""#, r#""bo@x""#, r#""cy@x""#],
                query(&db, EMAILS),
                "crash at {at}"
            );
            assert_eq!(vec![r#""Ann@X""#], by_lower(&db, "ann@x"), "crash at {at}");
            let got = write(&db, &insert("ANN@X", "Draft"));
            assert!(
                got.as_ref().is_err_and(is_violated),
                "crash at {at}: {got:?}"
            );
            db.close().expect("close");
        });
    }
}

#[test]
fn expression_and_path_indexes_hold_every_committed_row_after_a_crash() {
    let seed = 4;
    let mut sim = Sim::new(seed, Config::default());
    run(&mut sim, |rt| drop(open(&rt)));
    sim.record();
    run(&mut sim, |rt| {
        let db = open(&rt);
        for (i, e) in ["Ann@X", "bo@x", "cy@x"].into_iter().enumerate() {
            write(&db, &insert(e, &paid(&format!("r{i}")))).expect("the insert");
        }
        std::mem::forget(db);
    });
    let recording = sim.recording();
    for at in 0..=recording.len() {
        let files = recording.images(at, 1, seed + at as u64).swap_remove(0);
        let mut sim = Sim::with_files(seed + at as u64, Config::default(), files);
        run(&mut sim, |rt| {
            let db = open(&rt);
            let held = query(&db, EMAILS);
            for (i, e) in ["Ann@X", "bo@x", "cy@x"].into_iter().enumerate() {
                let quoted = format!("{e:?}");
                let r = format!("r{i}");
                if held.contains(&quoted) {
                    assert_eq!(
                        vec![quoted.clone()],
                        by_lower(&db, &e.to_lowercase()),
                        "crash at {at}"
                    );
                    assert_eq!(vec![quoted], by_ref(&db, &r), "crash at {at}");
                    let got = write(&db, &insert("z@x", &paid(&r)));
                    assert!(
                        got.as_ref().is_err_and(is_violated),
                        "crash at {at}: {got:?}"
                    );
                } else {
                    assert!(by_lower(&db, &e.to_lowercase()).is_empty(), "crash at {at}");
                    assert!(by_ref(&db, &r).is_empty(), "crash at {at}");
                }
            }
            db.close().expect("close");
        });
    }
}

#[test]
fn a_writer_of_a_path_unique_value_committed_after_its_snapshot_fails_to_serialize() {
    for seed in SEEDS {
        once(seed, |rt| {
            let db = open(&rt);
            let mut a = db.begin(&TxnOptions::default()).expect("begin a");
            let mut b = db.begin(&TxnOptions::default()).expect("begin b");
            in_txn(&mut a, EMAILS).expect("a reads");
            in_txn(&mut b, EMAILS).expect("b reads");
            in_txn(&mut a, &insert("a@x", &paid("r9"))).expect("a inserts");
            a.commit(None).expect("a commits");
            let got =
                in_txn(&mut b, &insert("b@x", &paid("r9"))).and_then(|()| b.commit(None).map(drop));
            assert!(
                got.as_ref().is_err_and(Error::is_serialization),
                "seed {seed}: {got:?}"
            );
            assert_eq!(vec![r#""a@x""#], by_ref(&db, "r9"), "seed {seed}");
        });
    }
}

#[test]
fn an_upsert_on_a_path_unique_against_a_row_committed_after_its_snapshot_fails_to_serialize() {
    for seed in SEEDS {
        once(seed, |rt| {
            let db = open(&rt);
            let upsert = |email: &str| {
                format!(
                    r#"insert invoices {{ status: Paid {{ ref: "r9" }}, email: "{email}", n: 0 }}
  unless conflict on (status.Paid.ref)
  else update old set Invoice {{ old with n: old.n + 1 }}"#
                )
            };
            let mut a = db.begin(&TxnOptions::default()).expect("begin a");
            let mut b = db.begin(&TxnOptions::default()).expect("begin b");
            in_txn(&mut a, EMAILS).expect("a reads");
            in_txn(&mut b, EMAILS).expect("b reads");
            in_txn(&mut a, &upsert("a@x")).expect("a upserts");
            a.commit(None).expect("a commits");
            let got = in_txn(&mut b, &upsert("b@x")).and_then(|()| b.commit(None).map(drop));
            assert!(
                got.as_ref().is_err_and(Error::is_serialization),
                "seed {seed}: {got:?}"
            );
            assert_eq!(
                vec![r#"{ email: "a@x", n: 0 }"#],
                query(&db, "from i in invoices select { email: i.email, n: i.n }"),
                "seed {seed}"
            );
        });
    }
}

#[test]
fn a_writer_of_a_path_unique_value_another_is_writing_waits_and_fails_if_it_commits() {
    for seed in SEEDS {
        once(seed, |rt| {
            let db = open(&rt);
            let events: Arc<Mutex<Vec<&str>>> = Arc::new(Mutex::new(Vec::new()));
            let mut a = db.begin(&TxnOptions::default()).expect("begin a");
            in_txn(&mut a, &insert("a@x", &paid("r9"))).expect("a inserts");
            let ended = Arc::new(Mutex::new(None));
            let waiter = {
                let db = db.clone();
                let events = events.clone();
                let ended = ended.clone();
                rt.spawn("b", move || {
                    let mut b = db.begin(&TxnOptions::default()).expect("begin b");
                    in_txn(&mut b, EMAILS).expect("b reads");
                    events.lock().unwrap().push("b begun");
                    let got = in_txn(&mut b, &insert("b@x", &paid("r9")))
                        .and_then(|()| b.commit(None).map(drop));
                    events.lock().unwrap().push("b ends");
                    *ended.lock().unwrap() = Some(got);
                })
            };
            while events.lock().unwrap().is_empty() {
                rt.yield_now();
            }
            events.lock().unwrap().push("a commits");
            a.commit(None).expect("a commits");
            rt.join(waiter);
            let got = ended.lock().unwrap().take().expect("b ends");
            assert!(
                got.as_ref().is_err_and(Error::is_serialization),
                "seed {seed}: {got:?}"
            );
            assert_eq!(
                vec!["b begun", "a commits", "b ends"],
                *events.lock().unwrap(),
                "seed {seed}"
            );
            assert_eq!(vec![r#""a@x""#], by_ref(&db, "r9"), "seed {seed}");
        });
    }
}

#[test]
fn a_writer_of_a_path_unique_value_another_is_writing_proceeds_if_it_aborts() {
    for seed in SEEDS {
        once(seed, |rt| {
            let db = open(&rt);
            let mut a = db.begin(&TxnOptions::default()).expect("begin a");
            in_txn(&mut a, &insert("a@x", &paid("r9"))).expect("a inserts");
            let ended = Arc::new(Mutex::new(None));
            let begun = Arc::new(Mutex::new(false));
            let waiter = {
                let db = db.clone();
                let (ended, begun) = (ended.clone(), begun.clone());
                rt.spawn("b", move || {
                    let mut b = db.begin(&TxnOptions::default()).expect("begin b");
                    in_txn(&mut b, EMAILS).expect("b reads");
                    *begun.lock().unwrap() = true;
                    let got = in_txn(&mut b, &insert("b@x", &paid("r9")))
                        .and_then(|()| b.commit(None).map(drop));
                    *ended.lock().unwrap() = Some(got);
                })
            };
            while !*begun.lock().unwrap() {
                rt.yield_now();
            }
            a.rollback().expect("a rolls back");
            rt.join(waiter);
            let got = ended.lock().unwrap().take().expect("b ends");
            assert!(got.is_ok(), "seed {seed}: {got:?}");
            assert_eq!(vec![r#""b@x""#], by_ref(&db, "r9"), "seed {seed}");
        });
    }
}
