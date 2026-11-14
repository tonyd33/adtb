//! The in-process API over the simulator: open, ad hoc text, interactive transactions and the
//! retrying closure.

use adtb_harness::process::{once, run};
use adtb_simkit::api;
use adtb_simkit::open;

use std::sync::{Arc, Mutex};

use adtb::runtime::Threads;
use adtb::{
    CallOptions, Database, Error, Limit, Log, OpenError, OpenOptions, Position, RejectReason,
    RetryReason, TxnCallOptions, TxnOptions, Value,
};
use adtb_lang::types::{Base, Ty};
use adtb_sim::{Config, Sim, SimRuntime};

use adtb_simkit::open::options;
use api::{in_txn, printed, query, result};

const SHOP: &str = r"module shop
add module shop
add type Note = { id: Key notes, body: Text }
add relation notes : Note key id generate sequence
add type Customer = { id: Key customers, name: Text }
add relation customers : Customer key id generate sequence
add type Payment { id: Key payments, customer: Key customers }
  = Card { last4: Text }
  | Invoice { due: Text }
add relation payments : Payment
  key id generate sequence
  index (customer)
add constraint paid =
  from c in customers
  where not exists p in payments where p.customer == key c
  select c
";

/// Inserts a customer named `n` with a card, as one statement.
const SIGN_UP: &str = r#"transact
  let c = insert customers { name: n }
  insert payments.Card { customer: key c, last4: "4242" }
  return key c"#;

const NAMES: &str = "from c in customers select c.name";

const SEEDS: std::ops::Range<u64> = 0..3;

fn log() -> Log {
    Log::new([SHOP])
}

fn text(t: &str) -> Value {
    Value::text(t)
}

fn name(n: &str) -> [(&'static str, Ty, Value); 1] {
    [("n", Ty::Base(Base::Text), text(n))]
}

fn sign_up(db: &Database<SimRuntime>, n: &str) {
    db.query_text(SIGN_UP, &name(n), &CallOptions::default())
        .unwrap_or_else(|e| panic!("signing up {n} fails: {e:?}"));
}

fn reason(e: &Error) -> &RejectReason {
    match e {
        Error::Rejected { reason } => reason,
        other => panic!("not a rejection: {other:?}"),
    }
}

#[test]
fn a_handle_is_shared_across_threads() {
    fn shareable<T: Send + Sync + Clone>() {}
    shareable::<Database<SimRuntime>>();
    shareable::<Database>();
}

#[test]
fn a_missing_file_is_not_found_without_create() {
    once(0, |rt| {
        let got = Database::open_in(rt, "app.adt", &log(), &OpenOptions::default());
        assert!(matches!(got, Err(OpenError::NotFound)), "{got:?}");
    });
}

#[test]
fn open_creates_the_file_and_applies_the_first_migration() {
    for seed in SEEDS {
        once(seed, |rt| {
            let db = open::opened(&rt, open::FILE, &log(), &options(4));
            assert!(
                query(&db, NAMES).is_empty(),
                "seed {seed}: a new database holds no customers"
            );
            sign_up(&db, "ann");
            assert_eq!(
                vec![r#""ann""#.to_string()],
                query(&db, NAMES),
                "seed {seed}"
            );
            db.close().expect("close");
        });
    }
}

#[test]
fn results_carry_their_type() {
    once(0, |rt| {
        let db = open::opened(&rt, open::FILE, &log(), &options(4));
        let out = db
            .query_text("count(c for c in customers)", &[], &CallOptions::default())
            .expect("a count");
        assert_eq!(1, out.len());
        assert_eq!(&Ty::Base(Base::I64), out[0].ty());
        let out = db
            .query_text(NAMES, &[], &CallOptions::default())
            .expect("the names");
        assert_eq!(&Ty::Base(Base::Text), out[0].ty());
    });
}

#[test]
fn a_process_that_exits_without_closing_loses_nothing_committed() {
    let mut sim = Sim::new(2, Config::default());
    run(&mut sim, |rt| {
        let db = open::opened(&rt, open::FILE, &log(), &options(4));
        sign_up(&db, "ann");
        std::mem::forget(db);
    });
    run(&mut sim, |rt| {
        let db = open::opened(&rt, open::FILE, &log(), &options(4));
        assert_eq!(vec![r#""ann""#], query(&db, NAMES));
    });
}

#[test]
fn parameters_bind_free_names_by_name() {
    once(0, |rt| {
        let db = open::opened(&rt, open::FILE, &log(), &options(4));
        sign_up(&db, "ann");
        sign_up(&db, "bob");
        let mut out = db
            .query_text(
                "from c in customers where c.name == n select c.name",
                &name("bob"),
                &CallOptions::default(),
            )
            .expect("a query with a parameter");
        assert_eq!(vec![r#""bob""#], printed(out.remove(0)));
    });
}

#[test]
fn every_item_of_a_text_returns_its_result_in_order() {
    once(0, |rt| {
        let db = open::opened(&rt, open::FILE, &log(), &options(4));
        sign_up(&db, "ann");
        let out = db
            .query_text(
                r#"insert notes { body: "a" }
(count(n for n in notes))
from c in customers select c.name"#,
                &[],
                &CallOptions::default(),
            )
            .expect("three items");
        let got: Vec<Vec<String>> = out.into_iter().map(printed).collect();
        assert_eq!(
            vec![vec!["1".to_string()], vec![r#""ann""#.to_string()]],
            got
        );
    });
}

#[test]
fn a_failing_item_applies_nothing_of_its_text() {
    once(0, |rt| {
        let db = open::opened(&rt, open::FILE, &log(), &options(4));
        let got = db.query_text(
            r#"insert notes { id: notes.key(1), body: "a" }
insert notes { id: notes.key(1), body: "b" }"#,
            &[],
            &CallOptions::default(),
        );
        let err = got.expect_err("the second item collides on the key");
        assert!(
            matches!(reason(&err), RejectReason::KeyViolation { relation, .. } if relation == "shop::notes"),
            "{err:?}"
        );
        assert!(query(&db, "from n in notes select n.body").is_empty());
    });
}

#[test]
fn a_fail_in_a_block_is_a_user_error() {
    once(0, |rt| {
        let db = open::opened(&rt, open::FILE, &log(), &options(4));
        let err = db
            .query_text(
                r#"transact
  insert notes { body: "a" }
  fail "stop""#,
                &[],
                &CallOptions::default(),
            )
            .expect_err("the block fails");
        assert_eq!(
            &RejectReason::UserError {
                message: "stop".into()
            },
            reason(&err)
        );
        assert!(query(&db, "from n in notes select n.body").is_empty());
    });
}

#[test]
fn a_block_returns_its_result() {
    once(0, |rt| {
        let db = open::opened(&rt, open::FILE, &log(), &options(4));
        let mut out = db
            .query_text(SIGN_UP, &name("ann"), &CallOptions::default())
            .expect("the block commits");
        assert_eq!(vec!["customers.key(1)"], printed(out.remove(0)));
    });
}

#[test]
fn an_interactive_transaction_sees_its_own_writes_and_publishes_them_at_commit() {
    for seed in SEEDS {
        once(seed, |rt| {
            let db = open::opened(&rt, open::FILE, &log(), &options(4));
            let mut t = db.begin(&TxnOptions::default()).expect("begin");
            t.query_text(SIGN_UP, &name("ann"), &TxnCallOptions::default())
                .expect("the sign up");
            assert_eq!(vec![r#""ann""#], in_txn(&mut t, NAMES), "seed {seed}");
            assert!(
                query(&db, NAMES).is_empty(),
                "seed {seed}: seen before commit"
            );
            let at = t.commit(None).expect("commit");
            assert!(at.lsn > 0, "seed {seed}: {at:?}");
            assert_eq!(vec![r#""ann""#], query(&db, NAMES), "seed {seed}");
        });
    }
}

#[test]
fn rollback_and_drop_discard_a_transaction() {
    once(0, |rt| {
        let db = open::opened(&rt, open::FILE, &log(), &options(4));
        let mut t = db.begin(&TxnOptions::default()).expect("begin");
        t.query_text(SIGN_UP, &name("ann"), &TxnCallOptions::default())
            .expect("the sign up");
        t.rollback().expect("rollback");
        let mut t = db.begin(&TxnOptions::default()).expect("begin");
        t.query_text(SIGN_UP, &name("bob"), &TxnCallOptions::default())
            .expect("the sign up");
        drop(t);
        assert!(query(&db, NAMES).is_empty());
    });
}

#[test]
fn a_deferrable_transaction_begins_once_the_writers_running_at_its_snapshot_end() {
    for seed in SEEDS {
        once(seed, |rt| {
            let db = open::opened(&rt, open::FILE, &log(), &options(4));
            let events = Arc::new(Mutex::new(Vec::new()));
            let mut w = db.begin(&TxnOptions::default()).expect("begin the writer");
            w.query_text(
                r#"insert notes { body: "w" }"#,
                &[],
                &TxnCallOptions::default(),
            )
            .expect("the writer writes");
            let reader = {
                let db = db.clone();
                let events = events.clone();
                rt.spawn("reader", move || {
                    let opts = TxnOptions {
                        read_only: true,
                        deferrable: true,
                        ..TxnOptions::default()
                    };
                    let mut t = db.begin(&opts).expect("a deferrable transaction begins");
                    events.lock().unwrap().push("begun");
                    in_txn(&mut t, "count(n for n in notes)");
                    t.commit(None).expect("the reader commits");
                })
            };
            events.lock().unwrap().push("writer commits");
            w.commit(None).expect("the writer commits");
            rt.join(reader);
            assert_eq!(
                vec!["writer commits", "begun"],
                *events.lock().unwrap(),
                "seed {seed}"
            );
        });
    }
}

#[test]
fn a_serialization_failure_names_the_transaction_by_its_id() {
    once(0, |rt| {
        let db = open::opened(&rt, open::FILE, &log(), &options(4));
        seed_note(&db);
        let mut a = db.begin(&TxnOptions::default()).expect("begin a");
        let mut b = db.begin(&TxnOptions::default()).expect("begin b");
        assert_ne!(a.id(), b.id());
        assert!(b.id().0 > 0, "{:?}", b.id());
        a.query_text(BUMP, &body("a"), &TxnCallOptions::default())
            .expect("a writes");
        a.commit(None).expect("a commits");
        let id = b.id();
        let got = b.query_text(BUMP, &body("b"), &TxnCallOptions::default());
        match got {
            Err(Error::Retry {
                reason: RetryReason::Serialization { txn, .. },
            }) => assert_eq!(id, txn),
            other => panic!("not a serialization failure: {other:?}"),
        }
    });
}

const UNBOUNDED: &str = "fix n = [1] union (from r in n select r + 1) in count(x for x in n)";
const FIVE_ROUNDS: &str =
    "fix n = [1] union (from r in n where r < 5 select r + 1) in count(x for x in n)";

fn rounds_exceeded(e: &Error) -> Option<i64> {
    match e {
        Error::Rejected {
            reason:
                RejectReason::StatementLimit {
                    limit: Limit::Rounds { rounds },
                },
        } => Some(*rounds),
        _ => None,
    }
}

#[test]
fn the_round_limit_of_an_open_rejects_a_longer_fixpoint() {
    once(0, |rt| {
        let opts = OpenOptions {
            fix_rounds: Some(3),
            ..options(4)
        };
        let db = Database::open_in(rt, "app.adt", &log(), &opts).expect("the database opens");
        let got = db.query_text(FIVE_ROUNDS, &[], &CallOptions::default());
        let err = got.expect_err("five rounds run past a limit of three");
        assert_eq!(Some(3), rounds_exceeded(&err), "{err:?}");
        let more = CallOptions {
            fix_rounds: Some(50),
            ..CallOptions::default()
        };
        let got = db
            .query_text(FIVE_ROUNDS, &[], &more)
            .expect("fifty rounds suffice");
        assert_eq!(
            vec!["5"],
            printed(got.into_iter().next().expect("a result"))
        );
    });
}

#[test]
fn the_round_limit_of_a_call_rejects_an_unbounded_fixpoint() {
    once(0, |rt| {
        let db = open::opened(&rt, open::FILE, &log(), &options(4));
        let call = CallOptions {
            fix_rounds: Some(20),
            ..CallOptions::default()
        };
        let err = db
            .query_text(UNBOUNDED, &[], &call)
            .expect_err("an unbounded fix runs past its limit");
        assert_eq!(Some(20), rounds_exceeded(&err), "{err:?}");
        let mut t = db.begin(&TxnOptions::default()).expect("begin");
        let in_call = TxnCallOptions {
            fix_rounds: Some(4),
            ..TxnCallOptions::default()
        };
        let err = t
            .query_text(FIVE_ROUNDS, &[], &in_call)
            .expect_err("five rounds run past a limit of four");
        assert_eq!(Some(4), rounds_exceeded(&err), "{err:?}");
        assert_eq!(vec!["5"], in_txn(&mut t, FIVE_ROUNDS));
        t.commit(None).expect("commit");
    });
}

#[test]
fn a_read_after_a_position_past_the_history_is_refused() {
    once(0, |rt| {
        let db = open::opened(&rt, open::FILE, &log(), &options(4));
        let mut t = db.begin(&TxnOptions::default()).expect("begin");
        t.query_text(
            r#"insert notes { body: "a" }"#,
            &[],
            &TxnCallOptions::default(),
        )
        .expect("an insert");
        let at = t.commit(None).expect("commit");
        let after = |position| CallOptions {
            after: Some(position),
            ..CallOptions::default()
        };
        let got = db.query_text(NOTES, &[], &after(at));
        assert_eq!(1, got.expect("a read after its own commit").len());
        let past = Position {
            epoch: at.epoch,
            lsn: at.lsn + 1_000_000,
        };
        let got = db.query_text(NOTES, &[], &after(past));
        assert!(
            matches!(
                &got,
                Err(Error::Rejected {
                    reason: RejectReason::PositionNotOnHistory { position },
                }) if *position == past
            ),
            "{got:?}"
        );
    });
}

#[test]
fn a_call_whose_deadline_passed_before_it_starts_is_refused_with_deadline_exceeded() {
    once(0, |rt| {
        let db = open::opened(&rt, open::FILE, &log(), &options(4));
        let opts = CallOptions {
            deadline: Some(std::time::SystemTime::UNIX_EPOCH),
            ..CallOptions::default()
        };
        let got = db.query_text(NAMES, &[], &opts);
        assert!(
            matches!(
                got,
                Err(Error::Rejected {
                    reason: RejectReason::DeadlineExceeded
                })
            ),
            "{got:?}"
        );
    });
}

const NOTES: &str = "from n in notes select n.body";

fn is_serialization(e: &Error) -> bool {
    matches!(
        e,
        Error::Retry {
            reason: RetryReason::Serialization { .. }
        }
    )
}

const BUMP: &str = "update n in notes where key n == notes.key(1) set Note { n with body: b }";

fn body(b: &str) -> [(&'static str, Ty, Value); 1] {
    [("b", Ty::Base(Base::Text), text(b))]
}

fn seed_note(db: &Database<SimRuntime>) {
    db.query_text(
        r#"insert notes { id: notes.key(1), body: "0" }"#,
        &[],
        &CallOptions::default(),
    )
    .expect("the note");
}

#[test]
fn concurrent_interactive_transactions_fail_one_with_a_serialization_failure() {
    for seed in SEEDS {
        once(seed, |rt| {
            let db = open::opened(&rt, open::FILE, &log(), &options(4));
            seed_note(&db);
            let mut a = db.begin(&TxnOptions::default()).expect("begin a");
            let mut b = db.begin(&TxnOptions::default()).expect("begin b");
            a.query_text(BUMP, &body("a"), &TxnCallOptions::default())
                .expect("a writes");
            a.commit(None).expect("a commits");
            let err = b
                .query_text(BUMP, &body("b"), &TxnCallOptions::default())
                .expect_err("b writes the row a committed after b's snapshot");
            assert!(is_serialization(&err), "seed {seed}: {err:?}");
            let later = b.query_text(NAMES, &[], &TxnCallOptions::default());
            assert_eq!(
                Err(err.clone()),
                later.map(drop),
                "seed {seed}: a later call"
            );
            assert_eq!(Err(err), b.commit(None), "seed {seed}: the commit");
            assert_eq!(
                vec![r#""a""#],
                query(&db, "from n in notes select n.body"),
                "seed {seed}"
            );
        });
    }
}

#[test]
fn a_write_skew_between_interactive_transactions_does_not_commit_both() {
    for seed in SEEDS {
        once(seed, |rt| {
            let db = open::opened(&rt, open::FILE, &log(), &options(4));
            let count = "count(n for n in notes)";
            let mut a = db.begin(&TxnOptions::default()).expect("begin a");
            let mut b = db.begin(&TxnOptions::default()).expect("begin b");
            assert_eq!(vec!["0"], in_txn(&mut a, count), "seed {seed}");
            assert_eq!(vec!["0"], in_txn(&mut b, count), "seed {seed}");
            let wa = a.query_text(
                r#"insert notes { body: "a" }"#,
                &[],
                &TxnCallOptions::default(),
            );
            let wb = b.query_text(
                r#"insert notes { body: "b" }"#,
                &[],
                &TxnCallOptions::default(),
            );
            let ca = wa.and_then(|_| a.commit(None));
            let cb = wb.and_then(|_| b.commit(None));
            assert!(
                ca.is_err() || cb.is_err(),
                "seed {seed}: both commit after reading no notes"
            );
            for e in [&ca, &cb].into_iter().filter_map(|r| r.as_ref().err()) {
                assert!(is_serialization(e), "seed {seed}: {e:?}");
            }
        });
    }
}

#[test]
fn the_retrying_closure_reruns_after_a_serialization_failure() {
    for seed in SEEDS {
        once(seed, |rt| {
            let db = open::opened(&rt, open::FILE, &log(), &options(4));
            seed_note(&db);
            let mut runs = 0;
            let done = db
                .transact(&TxnOptions::default(), 3, |t| {
                    runs += 1;
                    let read = in_txn(t, "from n in notes select n.body");
                    if runs == 1 {
                        db.query_text(BUMP, &body("other"), &CallOptions::default())
                            .expect("a competing write commits");
                    }
                    t.query_text(BUMP, &body("mine"), &TxnCallOptions::default())?;
                    Ok(read)
                })
                .unwrap_or_else(|e| panic!("seed {seed}: the closure fails: {e:?}"));
            assert_eq!(2, runs, "seed {seed}");
            assert_eq!(2, done.attempts, "seed {seed}");
            assert_eq!(vec![r#""other""#], done.value, "seed {seed}");
            assert_eq!(
                vec![r#""mine""#],
                query(&db, "from n in notes select n.body"),
                "seed {seed}"
            );
        });
    }
}

#[test]
fn the_retrying_closure_returns_the_failure_once_its_attempts_are_spent() {
    once(0, |rt| {
        let db = open::opened(&rt, open::FILE, &log(), &options(4));
        seed_note(&db);
        let mut runs = 0;
        let got = db.transact(&TxnOptions::default(), 2, |t| {
            runs += 1;
            in_txn(t, "from n in notes select n.body");
            db.query_text(
                BUMP,
                &body(&format!("other {runs}")),
                &CallOptions::default(),
            )
            .expect("a competing write commits");
            t.query_text(BUMP, &body("mine"), &TxnCallOptions::default())
        });
        assert_eq!(2, runs);
        assert!(got.as_ref().is_err_and(is_serialization), "{got:?}");
    });
}

#[test]
fn the_retrying_closure_does_not_rerun_other_failures() {
    once(0, |rt| {
        let db = open::opened(&rt, open::FILE, &log(), &options(4));
        let mut runs = 0;
        let got = db.transact(&TxnOptions::default(), 5, |t| {
            runs += 1;
            t.query_text(
                r#"insert customers { name: "ann" }"#,
                &[],
                &TxnCallOptions::default(),
            )
        });
        assert_eq!(1, runs);
        assert!(
            matches!(
                &got,
                Err(Error::Rejected {
                    reason: RejectReason::Violated { .. }
                })
            ),
            "{got:?}"
        );
    });
}

#[test]
fn concurrent_text_from_many_threads_commits_every_write() {
    for seed in SEEDS {
        once(seed, |rt| {
            let db = open::opened(&rt, open::FILE, &log(), &options(4));
            seed_note(&db);
            let failed = Arc::new(Mutex::new(Vec::new()));
            let threads: Vec<_> = (0..4)
                .map(|i| {
                    let db = db.clone();
                    let failed = failed.clone();
                    rt.spawn(&format!("writer {i}"), move || {
                        for j in 0..3 {
                            let text = format!(
                                r#"transact
  update m in notes where key m == notes.key(1) set Note {{ m with body: "{i}{j}" }}
  insert notes {{ body: "{i}{j}" }}"#
                            );
                            if let Err(e) = db.query_text(&text, &[], &CallOptions::default()) {
                                failed.lock().unwrap().push(e);
                            }
                        }
                    })
                })
                .collect();
            for t in threads {
                rt.join(t);
            }
            let failed = failed.lock().unwrap();
            assert!(failed.is_empty(), "seed {seed}: {failed:?}");
            assert_eq!(
                vec!["13"],
                query(&db, "count(n for n in notes)"),
                "seed {seed}"
            );
        });
    }
}

/// A migration of `shop` adding `wides`, a record of `n` fields of type `ty`.
fn wide(n: usize, ty: &str) -> String {
    let fields: String = (0..n).map(|i| format!(", f{i}: {ty}")).collect();
    format!(
        "module shop\nadd type Wide = {{ id: I64{fields} }}\nadd relation wides : Wide key id\n"
    )
}

/// An insert of row `k` of `wides`, every one of its `n` text fields holding `t`.
fn wide_row(k: i64, n: usize, t: &str) -> String {
    let fields: String = (0..n).map(|i| format!(", f{i}: \"{t}\"")).collect();
    format!("insert wides {{ id: {k}{fields} }}")
}

fn open_wide(rt: &SimRuntime, n: usize) -> Database<SimRuntime> {
    Database::open_in(
        rt.clone(),
        "app.adt",
        &Log::new([SHOP.to_string(), wide(n, "Text")]),
        &options(4),
    )
    .expect("the database opens")
}

fn assert_too_large(e: &Error) {
    match reason(e) {
        RejectReason::RowTooLarge {
            relation,
            ctor,
            size,
            limit,
        } => {
            assert_eq!("shop::wides", relation);
            assert_eq!("Wide", ctor);
            assert!(size > limit, "{size} over {limit}");
        }
        other => panic!("not too large: {other:?}"),
    }
}

#[test]
fn a_row_too_large_is_rejected_naming_its_relation_constructor_size_and_limit() {
    once(0, |rt| {
        let db = open_wide(&rt, 600);
        let long = "y".repeat(30);
        let err = db
            .query_text(&wide_row(1, 600, &long), &[], &CallOptions::default())
            .expect_err("the row does not fit");
        assert_too_large(&err);
        assert!(query(&db, "from w in wides select w.id").is_empty());
        db.query_text(&wide_row(1, 600, ""), &[], &CallOptions::default())
            .expect("a row of empty fields fits");
        assert_eq!(vec!["1"], query(&db, "from w in wides select w.id"));
    });
}

#[test]
fn an_update_to_a_row_too_large_is_rejected() {
    once(1, |rt| {
        let db = open_wide(&rt, 600);
        db.query_text(&wide_row(1, 600, ""), &[], &CallOptions::default())
            .expect("a row of empty fields fits");
        let long = "y".repeat(30);
        let err = db
            .query_text(
                &format!(
                    "update w in wides where w.id == 1 set Wide {{ w with {} }}",
                    (0..600)
                        .map(|i| format!("f{i}: \"{long}\""))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                &[],
                &CallOptions::default(),
            )
            .expect_err("the new row does not fit");
        assert_too_large(&err);
        assert_eq!(
            vec![r#""""#],
            query(&db, "from w in wides select w.f0"),
            "the old row stays"
        );
    });
}

#[test]
fn a_row_too_large_takes_precedence_over_a_key_violation() {
    once(2, |rt| {
        let db = open_wide(&rt, 600);
        db.query_text(&wide_row(1, 600, ""), &[], &CallOptions::default())
            .expect("a row of empty fields fits");
        let long = "y".repeat(30);
        let err = db
            .query_text(&wide_row(1, 600, &long), &[], &CallOptions::default())
            .expect_err("the key is held and the row does not fit");
        assert_too_large(&err);
    });
}

#[test]
fn a_migration_declaring_a_constructor_that_can_never_fit_is_refused_naming_it() {
    for (seed, (n, ty)) in [(600, "Decimal 0"), (4000, "Text")].into_iter().enumerate() {
        once(seed as u64, |rt| {
            let got = Database::open_in(
                rt,
                "app.adt",
                &Log::new([SHOP.to_string(), wide(n, ty)]),
                &options(4),
            );
            match got {
                Err(OpenError::MigrationRefused {
                    version: 2,
                    reason:
                        Error::Rejected {
                            reason: RejectReason::SchemaRefused { detail, .. },
                        },
                }) => assert!(detail.contains("wides.Wide"), "seed {seed}: {detail}"),
                other => panic!("seed {seed}: {n} fields of {ty} are not refused: {other:?}"),
            }
        });
    }
}

#[test]
fn uuid_formatting_is_total_but_storage_checks_its_domain() {
    once(0, |rt| {
        let log = Log::new([
            "module app\nadd module app\nadd type Token = { id: Key tokens, value: Uuid }\nadd relation tokens : Token key id generate sequence",
        ]);
        let db = Database::open_in(rt, "uuid.adt", &log, &options(1)).expect("the database opens");
        for payload in [
            "",
            "x",
            "abcdefghijklmno",
            "abcdefghijklmnop",
            "abcdefghijklmnopq",
        ] {
            let value = format!("Uuid(bytes({payload:?}))");
            let printed = if payload.len() == 16 {
                "uuid(\"61626364-6566-6768-696a-6b6c6d6e6f70\")".to_owned()
            } else {
                let hex: String = payload.bytes().map(|b| format!("{b:02x}")).collect();
                format!("Uuid(0x{hex})")
            };
            assert_eq!(vec![printed.clone()], query(&db, &value), "{value}");
            assert_eq!(
                vec![format!("{printed:?}")],
                query(&db, &format!("print({value})")),
                "{value}"
            );
            let text = if payload.len() == 16 {
                "61626364-6566-6768-696a-6b6c6d6e6f70"
            } else {
                &printed
            };
            assert_eq!(
                vec![format!("{text:?}")],
                query(&db, &format!("text({value})"))
            );
            assert_eq!(
                vec!["True"],
                query(
                    &db,
                    &format!("Uuid.parse(print({value})) == Some {{ value: {value} }}")
                )
            );
            let mut rows = result(&db, &format!("unwrap({value})"));
            assert_eq!(
                Some(Ok(Value::Bytes(payload.as_bytes().into()))),
                rows.next()
            );
            assert!(rows.next().is_none());
            let insert = format!("insert tokens {{ id: tokens.key(1), value: {value} }}");
            let written = db.query_text(&insert, &[], &CallOptions::default());
            if payload.len() == 16 {
                written.expect("sixteen-byte UUID values are stored");
                db.query_text("delete t in tokens", &[], &CallOptions::default())
                    .expect("the stored control is removed");
            } else {
                let error = written.expect_err("the write boundary checks the UUID domain");
                match reason(&error) {
                    RejectReason::Violated {
                        constraint, report, ..
                    } => {
                        assert_eq!("prelude::Uuid.domain", constraint);
                        assert!(!report.is_empty(), "{error:?}");
                    }
                    other => panic!("{other:?}"),
                }
            }
            assert!(query(&db, "tokens").is_empty());
        }
        db.close().expect("the database closes");
    });
}

/// Commit `notes` filling several pages, the first with body "first".
fn seed_notes(db: &Database<SimRuntime>) {
    let pad = "x".repeat(200);
    let rows: Vec<String> = std::iter::once(r#"{ id: notes.key(1), body: "first" }"#.to_owned())
        .chain((2..=200).map(|i| format!(r#"{{ id: notes.key({i}), body: "{i}{pad}" }}"#)))
        .collect();
    let text = format!("insert notes from [{}]", rows.join(", "));
    db.query_text(&text, &[], &CallOptions::default())
        .expect("the notes are inserted");
}

/// Run `exists` in one transaction and an insert into `notes` in another, each writing what
/// the other read. Returns how each transaction ended.
fn exists_beside_an_insert(
    db: &Database<SimRuntime>,
    exists: &str,
) -> [Result<Position, Error>; 2] {
    let opts = TxnCallOptions::default;
    let mut a = db.begin(&TxnOptions::default()).expect("begin a");
    let mut b = db.begin(&TxnOptions::default()).expect("begin b");
    in_txn(&mut a, exists);
    in_txn(
        &mut b,
        "from n in notes where key n == notes.key(2) select n.body",
    );
    let a_wrote = a.query_text(
        r#"update n in notes where key n == notes.key(2) set Note { n with body: "a" }"#,
        &[],
        &opts(),
    );
    let b_wrote = b.query_text(r#"insert notes { body: "b" }"#, &[], &opts());
    let a_ended = a_wrote.and_then(|_| a.commit(None));
    let b_ended = b_wrote.and_then(|_| b.commit(None));
    [a_ended, b_ended]
}

#[test]
fn an_exists_that_stopped_at_a_row_does_not_conflict_with_an_insert_beside_it() {
    for seed in SEEDS {
        let ends = once(seed, |rt| {
            let db = open::opened(&rt, open::FILE, &log(), &options(4));
            seed_notes(&db);
            exists_beside_an_insert(&db, r#"exists n in notes where n.body == "first""#)
        });
        assert!(ends.iter().all(Result::is_ok), "seed {seed}: {ends:?}");
    }
}

#[test]
fn an_exists_that_read_every_row_conflicts_with_an_insert_beside_it() {
    for seed in SEEDS {
        let ends = once(seed, |rt| {
            let db = open::opened(&rt, open::FILE, &log(), &options(4));
            seed_notes(&db);
            exists_beside_an_insert(&db, r#"exists n in notes where n.body == "none""#)
        });
        assert!(
            ends.iter().any(|e| e.as_ref().is_err_and(is_serialization)),
            "seed {seed}: {ends:?}"
        );
        assert!(ends.iter().any(Result::is_ok), "seed {seed}: {ends:?}");
    }
}

#[test]
fn a_temp_record_row_prints_its_fields_in_declaration_order() {
    once(0, |rt| {
        let db = open::opened(&rt, open::FILE, &log(), &options(4));
        let mut t = db.begin(&TxnOptions::default()).expect("begin");
        api::in_txn_results(
            &mut t,
            "temp relation st : { k: I64, b: I64 } key k\ninsert st { k: 1, b: 2 }",
        );
        assert_eq!(
            vec!["st { k: 1, b: 2 }"],
            in_txn(&mut t, "from s in st select s")
        );
        let mut one = api::in_txn_results(
            &mut t,
            "temp relation u : { k: I64, b: I64 } key k\ninsert u { k: 1, b: 2 }\nfrom s in u select s",
        );
        assert_eq!(
            vec!["u { k: 1, b: 2 }"],
            printed(one.pop().expect("a result"))
        );
        t.commit(None).expect("commit");
    });
}
