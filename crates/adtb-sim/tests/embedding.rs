//! Calls in process over the simulator: cancellation tokens and deadlines, closing a database
//! with registrations and clones still held, and results as Arrow record batches.

#[expect(dead_code, reason = "this binary uses part of the API helpers")]
#[path = "support/api.rs"]
mod api;
#[expect(dead_code, reason = "this binary uses part of the seed helpers")]
#[path = "support/seeds.rs"]
mod seeds;

use std::time::Duration;

use adtb::batch::{Datum, Type};
use adtb::key::{Scalar, Value as Stored};
use adtb::runtime::{Clock, Done, Entropy, Io, Op, Runtime, Threads};
use adtb::{
    CallOptions, Called, CancelToken, Database, Error, Field, LoadOptions, Log, OpenOptions,
    Registration, RejectReason, TxnCallOptions, TxnOptions, Value,
};
use adtb_sim::{Config, Sim, SimRuntime};

use api::{in_txn, once, options, printed, result, run};

const SHOP: &str = r"module shop
add module shop
add type Customer = { id: Key customers, name: Text }
add relation customers : Customer key id generate sequence
add type Payment { id: Key payments, customer: Key customers }
  = Card { last4: Text }
  | Invoice { due: Text }
add relation payments : Payment key id generate sequence
";

const APP: &str = r"import shop
query customers () -> Set Customer = from c in customers select c
query payments () -> Set Payment = from p in payments select p
query cards () -> Set Payment.Card = from c in payments.Card select c
query names () -> Set Text = from c in customers select c.name
txn sign_up (n: Text) -> Key customers =
  transact
    let c = insert customers { name: n }
    return key c
";

const CUSTOMERS: u32 = 1;
const PAYMENTS: u32 = 2;
const CARDS: u32 = 3;
const NAMES: u32 = 4;
const SIGN_UP: u32 = 5;

const SEEDS: std::ops::Range<u64> = 0..6;

fn log() -> Log {
    Log::new([SHOP])
}

fn with_app() -> OpenOptions {
    OpenOptions {
        registrations: vec![Registration::new("app", APP)],
        ..options(4)
    }
}

fn open(rt: &SimRuntime) -> Database<SimRuntime> {
    Database::open_in(rt.clone(), "app.adt", &log(), &with_app()).expect("the database opens")
}

fn names(db: &Database<SimRuntime>) -> Vec<String> {
    let mut out = printed(result(db, "from c in customers select c.name"));
    out.sort();
    out
}

fn reason(e: &Error) -> &RejectReason {
    match e {
        Error::Rejected { reason } => reason,
        other => panic!("not a rejection: {other:?}"),
    }
}

fn is_closed(e: &Error) -> bool {
    matches!(
        e,
        Error::Rejected {
            reason: RejectReason::Closed,
        }
    )
}

fn cancelled(tok: &CancelToken) -> CallOptions {
    CallOptions {
        cancel: Some(tok.clone()),
        ..CallOptions::default()
    }
}

/// Advance the simulated clock by `d`.
fn sleep(rt: &SimRuntime, d: Duration) {
    let mut io = rt.io();
    let ticket = io.submit(Op::Timer { at: rt.now() + d });
    loop {
        if let Some(c) = io.poll() {
            assert_eq!(ticket, c.ticket);
            assert_eq!(Ok(Done::Fired), c.result);
            return;
        }
        rt.park();
    }
}

#[test]
fn a_token_once_cancelled_stays_cancelled_and_clones_share_it() {
    let tok = CancelToken::new();
    let other = tok.clone();
    assert!(!other.is_cancelled());
    tok.cancel();
    assert!(other.is_cancelled());
    assert!(other.is_cancelled(), "a cancelled token stays cancelled");
    assert!(!CancelToken::new().is_cancelled());
}

#[test]
fn a_call_started_with_a_cancelled_token_returns_cancelled_without_running() {
    once(0, |rt| {
        let db = open(&rt);
        let tok = CancelToken::new();
        tok.cancel();
        let got = db.query_text(r#"insert customers { name: "ann" }"#, &[], &cancelled(&tok));
        assert_eq!(
            Some(&RejectReason::Cancelled),
            got.as_ref().err().map(reason)
        );
        let app = db.registered("app").expect("app binds");
        let got = app.call(SIGN_UP, &[Value::text("bob")], &cancelled(&tok));
        assert_eq!(
            Some(&RejectReason::Cancelled),
            got.as_ref().err().map(reason)
        );
        assert!(names(&db).is_empty(), "a cancelled call has no effect");
    });
}

#[test]
fn a_cancelled_call_in_a_transaction_has_no_effect_and_leaves_it_open() {
    once(1, |rt| {
        let db = open(&rt);
        let tok = CancelToken::new();
        tok.cancel();
        let mut t = db.begin(&TxnOptions::default()).expect("begin");
        t.query_text(
            r#"insert customers { name: "ann" }"#,
            &[],
            &TxnCallOptions::default(),
        )
        .expect("an insert");
        let opts = TxnCallOptions {
            cancel: Some(tok),
            ..TxnCallOptions::default()
        };
        let got = t.query_text(r#"insert customers { name: "bob" }"#, &[], &opts);
        assert_eq!(
            Some(&RejectReason::Cancelled),
            got.as_ref().err().map(reason)
        );
        assert_eq!(
            vec![r#""ann""#],
            in_txn(&mut t, "from c in customers select c.name"),
            "the transaction stays open without the cancelled call's effects"
        );
        t.commit(None).expect("the transaction commits");
        assert_eq!(vec![r#""ann""#], names(&db));
    });
}

#[test]
fn a_call_whose_deadline_has_passed_returns_deadline_exceeded_without_running() {
    once(2, |rt| {
        let db = open(&rt);
        let opts = CallOptions {
            deadline: Some(rt.wall()),
            ..CallOptions::default()
        };
        let got = db.query_text(r#"insert customers { name: "ann" }"#, &[], &opts);
        assert_eq!(
            Some(&RejectReason::DeadlineExceeded),
            got.as_ref().err().map(reason)
        );
        assert!(names(&db).is_empty());
    });
}

#[test]
fn a_call_with_a_deadline_to_come_runs() {
    once(3, |rt| {
        let db = open(&rt);
        let opts = CallOptions {
            deadline: Some(rt.wall() + Duration::from_secs(60)),
            limit: Some(Duration::from_secs(60)),
            ..CallOptions::default()
        };
        db.query_text(r#"insert customers { name: "ann" }"#, &[], &opts)
            .expect("the call runs before its deadline");
        assert_eq!(vec![r#""ann""#], names(&db));
    });
}

#[test]
fn a_transaction_whose_deadline_has_passed_refuses_its_calls() {
    once(4, |rt| {
        let db = open(&rt);
        let opts = TxnOptions {
            deadline: Some(rt.wall() + Duration::from_secs(1)),
            ..TxnOptions::default()
        };
        let mut t = db.begin(&opts).expect("begin before the deadline");
        in_txn(&mut t, "from c in customers select c.name");
        sleep(&rt, Duration::from_secs(2));
        let got = t.query_text(
            r#"insert customers { name: "ann" }"#,
            &[],
            &TxnCallOptions::default(),
        );
        assert_eq!(
            Some(&RejectReason::DeadlineExceeded),
            got.as_ref().err().map(reason)
        );
    });
}

/// Run `write` in a simulation seeded `seed` with a token another thread cancels after as many
/// turns of the scheduler as the seed draws, and check that the write either commits, its row present then and after a
/// reopen, or fails with `Cancelled` and has no effect. Returns whether it committed.
fn racing(
    seed: u64,
    write: impl Fn(&Database<SimRuntime>, &CallOptions) -> Result<(), Error>,
) -> bool {
    let mut sim = Sim::new(seed, Config::default());
    let returned = run(&mut sim, |rt| {
        let db = open(&rt);
        let tok = CancelToken::new();
        let turns = rt.next_u64() % 4000;
        let canceller = {
            let (rt2, tok) = (rt.clone(), tok.clone());
            rt.spawn("canceller", move || {
                for _ in 0..turns {
                    rt2.yield_now();
                }
                tok.cancel();
            })
        };
        let got = write(&db, &cancelled(&tok));
        rt.join(canceller);
        let present = names(&db) == vec![r#""ann""#];
        match &got {
            Ok(()) => assert!(present, "seed {seed}: a write that returned committed"),
            Err(e) => {
                assert_eq!(&RejectReason::Cancelled, reason(e), "seed {seed}");
                assert!(!present, "seed {seed}: a cancelled write has no effect");
            }
        }
        db.close().expect("close");
        got.is_ok()
    });
    let present = run(&mut sim, |rt| names(&open(&rt)) == vec![r#""ann""#]);
    assert_eq!(returned, present, "seed {seed}: the reopened file agrees");
    returned
}

/// Check that the sweep saw both outcomes of a race, `committed` of each seed's.
fn both(committed: &[bool]) {
    if committed.len() >= SEEDS.count() {
        assert!(
            committed.contains(&true),
            "no write committed: {committed:?}"
        );
        assert!(
            committed.contains(&false),
            "no write was cancelled: {committed:?}"
        );
    }
}

#[test]
fn a_cancellation_racing_a_commit_either_commits_or_has_no_effect() {
    let committed: Vec<bool> = seeds::sweep(SEEDS)
        .map(|seed| {
            racing(seed, |db, opts| {
                db.query_text(r#"insert customers { name: "ann" }"#, &[], opts)
                    .map(drop)
            })
        })
        .collect();
    both(&committed);
}

#[test]
fn a_cancellation_racing_a_registered_transaction_either_commits_or_has_no_effect() {
    let committed: Vec<bool> = seeds::sweep(SEEDS)
        .map(|seed| {
            racing(seed, |db, opts| {
                let app = db.registered("app").expect("app binds");
                app.call(SIGN_UP, &[Value::text("ann")], opts).map(drop)
            })
        })
        .collect();
    both(&committed);
}

#[test]
fn a_registration_held_past_close_refuses_its_calls_and_keeps_nothing_open() {
    let mut sim = Sim::new(5, Config::default());
    run(&mut sim, |rt| {
        let db = open(&rt);
        let app = db.registered("app").expect("app binds");
        db.close().expect("close");
        let got = app.call(NAMES, &[], &CallOptions::default());
        assert!(got.as_ref().is_err_and(is_closed), "{got:?}");
        assert_eq!("app", app.name());
        assert_eq!(Some(NAMES), app.ordinal("names"));
        let again = open(&rt);
        again
            .query_text(
                r#"insert customers { name: "ann" }"#,
                &[],
                &CallOptions::default(),
            )
            .expect("the reopened database writes");
        drop(app);
        again.close().expect("close");
    });
}

#[test]
fn closing_one_handle_closes_the_database_for_its_clones() {
    once(6, |rt| {
        let db = open(&rt);
        let other = db.clone();
        db.close().expect("close");
        let got = other.query_text("from c in customers select c", &[], &CallOptions::default());
        assert!(got.as_ref().is_err_and(is_closed), "{got:?}");
        let got = other.begin(&TxnOptions::default()).map(drop);
        assert!(got.as_ref().is_err_and(is_closed), "{got:?}");
        let got = other.registered("app");
        assert!(got.as_ref().is_err_and(is_closed), "{got:?}");
        let got = other.load(&LoadOptions::default()).map(drop);
        assert!(got.as_ref().is_err_and(is_closed), "{got:?}");
        other
            .close()
            .expect("closing a closed database again succeeds");
    });
}

#[test]
fn a_transaction_open_at_close_ends_uncommitted() {
    let mut sim = Sim::new(7, Config::default());
    run(&mut sim, |rt| {
        let db = open(&rt);
        let other = db.clone();
        let mut t = other.begin(&TxnOptions::default()).expect("begin");
        t.query_text(
            r#"insert customers { name: "ann" }"#,
            &[],
            &TxnCallOptions::default(),
        )
        .expect("an insert");
        db.close().expect("close");
        let got = t.commit(None);
        assert!(got.as_ref().is_err_and(is_closed), "{got:?}");
    });
    let after = run(&mut sim, |rt| names(&open(&rt)));
    assert!(
        after.is_empty(),
        "nothing of the open transaction committed"
    );
}

#[test]
fn a_dropped_transaction_of_a_closed_database_rolls_back_quietly() {
    once(8, |rt| {
        let db = open(&rt);
        let other = db.clone();
        let t = other.begin(&TxnOptions::default()).expect("begin");
        db.close().expect("close");
        drop(t);
        let t = {
            let again = open(&rt);
            let t = again.begin(&TxnOptions::default()).expect("begin");
            t.rollback().expect("rollback");
            again
        };
        t.close().expect("close");
    });
}

/// Returns the batches of call `ordinal` of `app` as Arrow, with their schema's fields.
fn arrow(db: &Database<SimRuntime>, ordinal: u32) -> (Vec<Field>, Vec<Vec<Datum>>) {
    let app = db.registered("app").expect("app binds");
    let stream = app
        .call_arrow(ordinal, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("call {ordinal} as Arrow: {e:?}"));
    let fields = stream.schema().fields.clone();
    let mut rows = Vec::new();
    for b in stream {
        let b = b.expect("a batch");
        assert_eq!(
            b.schema().fields,
            fields,
            "every batch has the stream's schema"
        );
        for r in 0..b.num_rows() {
            rows.push(b.batch().datums(r));
        }
    }
    rows.sort_by_key(|r| format!("{r:?}"));
    (fields, rows)
}

fn seed_shop(db: &Database<SimRuntime>) {
    db.query_text(
        r#"transact
  let c = insert customers { name: "ann" }
  insert payments.Card { customer: key c, last4: "4242" }
  insert payments.Invoice { customer: key c, due: "may" }"#,
        &[],
        &CallOptions::default(),
    )
    .expect("the shop seeds");
}

fn text(t: &str) -> Datum {
    Datum::Scalar(Stored::Text(t.into()))
}

#[test]
fn a_record_result_is_one_column_per_field_in_declared_order() {
    once(9, |rt| {
        let db = open(&rt);
        seed_shop(&db);
        let (fields, rows) = arrow(&db, CUSTOMERS);
        let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(vec!["id", "name"], names);
        assert_eq!(
            Type::Scalar(Scalar::I64),
            fields[0].ty,
            "a sequence key is an I64"
        );
        assert_eq!(Type::Scalar(Scalar::Text), fields[1].ty);
        assert_eq!(1, rows.len());
        assert_eq!(text("ann"), rows[0][1]);
    });
}

#[test]
fn a_sum_result_is_one_union_column_whose_children_carry_constructor_ids() {
    once(10, |rt| {
        let db = open(&rt);
        seed_shop(&db);
        let (fields, rows) = arrow(&db, PAYMENTS);
        assert_eq!(1, fields.len());
        assert_eq!("row", fields[0].name);
        let Type::Sum(ctors) = &fields[0].ty else {
            panic!("a sum result is a union column: {:?}", fields[0].ty);
        };
        assert_eq!(
            2,
            ctors.len(),
            "one child per constructor, Card then Invoice"
        );
        assert_ne!(
            ctors[0].id, ctors[1].id,
            "each child carries its own constructor id"
        );
        let got: Vec<(u32, Datum)> = rows
            .iter()
            .map(|r| match &r[0] {
                Datum::Sum { ctor, fields } => (*ctor, fields[2].clone()),
                other => panic!("a row of a union column is a sum: {other:?}"),
            })
            .collect();
        assert_eq!(vec![(0, text("4242")), (1, text("may"))], got);
    });
}

#[test]
fn a_result_of_one_constructor_is_the_columns_of_its_fields() {
    once(11, |rt| {
        let db = open(&rt);
        seed_shop(&db);
        let (fields, rows) = arrow(&db, CARDS);
        let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(vec!["id", "customer", "last4"], names);
        assert_eq!(
            vec![text("4242")],
            rows.iter().map(|r| r[2].clone()).collect::<Vec<_>>()
        );
    });
}

#[test]
fn a_result_of_base_values_is_one_column_named_value() {
    once(12, |rt| {
        let db = open(&rt);
        seed_shop(&db);
        let (fields, rows) = arrow(&db, NAMES);
        assert_eq!(
            vec![Field {
                name: "value".into(),
                ty: Type::Scalar(Scalar::Text)
            }],
            fields
        );
        assert_eq!(vec![vec![text("ann")]], rows);
    });
}

#[test]
fn a_registered_transaction_as_arrow_returns_its_result() {
    once(13, |rt| {
        let db = open(&rt);
        let app = db.registered("app").expect("app binds");
        let stream = app
            .call_arrow(SIGN_UP, &[Value::text("ann")], &CallOptions::default())
            .expect("the transaction commits");
        assert_eq!("value", stream.schema().fields[0].name);
        let rows: usize = stream.map(|b| b.expect("a batch").num_rows()).sum();
        assert_eq!(1, rows);
        assert_eq!(vec![r#""ann""#], names(&db));
        let app = db.registered("app").expect("app binds");
        assert!(matches!(
            app.call(NAMES, &[], &CallOptions::default()),
            Ok(Called::Rows(_))
        ));
    });
}

#[test]
fn a_large_result_streams_in_several_batches_of_one_schema() {
    once(14, |rt| {
        let db = open(&rt);
        db.query_text(
            "insert customers from (from a in [0, 1, 2], b in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9], \
             c in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9], d in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9] \
             select { name: print(a * 1000 + b * 100 + c * 10 + d) })",
            &[],
            &CallOptions::default(),
        )
        .expect("the customers insert");
        let app = db.registered("app").expect("app binds");
        let stream = app
            .call_arrow(NAMES, &[], &CallOptions::default())
            .expect("the names");
        let sizes: Vec<usize> = stream.map(|b| b.expect("a batch").num_rows()).collect();
        assert!(sizes.len() > 1, "{sizes:?}");
        assert_eq!(3000, sizes.iter().sum::<usize>());
    });
}

#[test]
fn the_open_time_settings_show_read_only_in_sys_process_options() {
    once(15, |rt| {
        let db = open(&rt);
        let mut got = printed(result(
            &db,
            r#"from o in sys::process_options select o.name ++ "=" ++ o.value ++ (if o.derived then " derived" else "")"#,
        ));
        got.sort();
        assert!(got.contains(&r#""workers=4""#.to_owned()), "{got:?}");
        assert!(
            got.iter()
                .any(|o| o.starts_with(r#""query_memory_bytes="#) && o.ends_with(r#" derived""#)),
            "a setting the program gave none of is derived: {got:?}"
        );
        let refused = db.query_text(
            r#"insert sys::process_options { id: sys::process_options.key(99), name: "x", value: "y", derived: false }"#,
            &[],
            &CallOptions::default(),
        );
        assert!(
            refused.is_err(),
            "no write changes an open-time setting: {refused:?}"
        );
    });
}
