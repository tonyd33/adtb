//! The in-process API over the simulator: open, ad hoc text, interactive transactions and the
//! retrying closure.

use std::sync::{Arc, Mutex};

use adtb::runtime::Threads;
use adtb::{
    AdHocTransaction, CallOptions, CreateOptions, Database, DynRows, Error, Limit, Log, OpenError,
    OpenOptions, Position, RejectReason, RetryReason, TxnCallOptions, TxnOptions, Value,
};
use adtb_lang::textual::print;
use adtb_lang::types::{Base, Ty};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};

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

fn options() -> OpenOptions {
    OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(4),
        ..OpenOptions::default()
    }
}

fn open(rt: &SimRuntime) -> Database<SimRuntime> {
    Database::open_in(rt.clone(), "app.adt", &log(), &options()).expect("the database opens")
}

/// Runs `body` as one process of `sim` and returns what it returns.
fn run<T>(sim: &mut Sim, body: impl FnOnce(SimRuntime) -> T) -> T {
    let mut got = None;
    let outcome = sim.run(|rt| got = Some(body(rt)));
    assert_eq!(Outcome::Finished, outcome);
    got.expect("the run returns")
}

fn once<T>(seed: u64, body: impl FnOnce(SimRuntime) -> T) -> T {
    run(&mut Sim::new(seed, Config::default()), body)
}

fn text(t: &str) -> Value {
    Value::text(t)
}

fn name(n: &str) -> [(&'static str, Ty, Value); 1] {
    [("n", Ty::Base(Base::Text), text(n))]
}

fn printed(rows: DynRows) -> Vec<String> {
    let form = rows.form().clone();
    rows.map(|r| print(&r.expect("a row"), &form).expect("a row prints"))
        .collect()
}

/// Returns each row of the one result of `text` run in `db`, printed.
fn query(db: &Database<SimRuntime>, text: &str) -> Vec<String> {
    let mut out = db
        .query_text(text, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"));
    assert_eq!(1, out.len(), "`{text}` has one result");
    printed(out.remove(0))
}

fn in_txn(t: &mut AdHocTransaction<'_, SimRuntime>, text: &str) -> Vec<String> {
    let mut out = t
        .query_text(text, &[], &TxnCallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"));
    assert_eq!(1, out.len(), "`{text}` has one result");
    printed(out.remove(0))
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

fn is_invalid(e: &Error) -> bool {
    matches!(
        e,
        Error::Rejected {
            reason: RejectReason::Invalid { .. }
        }
    )
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
            let db = open(&rt);
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
        let db = open(&rt);
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
fn a_reopened_file_holds_what_was_committed() {
    let mut sim = Sim::new(1, Config::default());
    run(&mut sim, |rt| {
        let db = open(&rt);
        sign_up(&db, "ann");
        sign_up(&db, "bob");
        db.close().expect("close");
    });
    run(&mut sim, |rt| {
        let db = open(&rt);
        assert_eq!(vec![r#""ann""#, r#""bob""#], query(&db, NAMES));
        sign_up(&db, "cat");
        assert_eq!(
            3,
            query(&db, NAMES).len(),
            "a new key is drawn past the old ones"
        );
    });
}

#[test]
fn a_process_that_exits_without_closing_loses_nothing_committed() {
    let mut sim = Sim::new(2, Config::default());
    run(&mut sim, |rt| {
        let db = open(&rt);
        sign_up(&db, "ann");
        std::mem::forget(db);
    });
    run(&mut sim, |rt| {
        let db = open(&rt);
        assert_eq!(vec![r#""ann""#], query(&db, NAMES));
    });
}

#[test]
fn a_shipped_log_that_differs_from_the_file_diverges() {
    let mut sim = Sim::new(3, Config::default());
    run(&mut sim, |rt| drop(open(&rt)));
    run(&mut sim, |rt| {
        let other = Log::new([SHOP.replace("body: Text", "text: Text")]);
        let got = Database::open_in(rt, "app.adt", &other, &options());
        assert!(
            matches!(got, Err(OpenError::Diverged { version: 1 })),
            "{got:?}"
        );
    });
}

#[test]
fn a_shorter_shipped_log_opens_at_the_file_version() {
    let mut sim = Sim::new(4, Config::default());
    run(&mut sim, |rt| sign_up(&open(&rt), "ann"));
    run(&mut sim, |rt| {
        let db = Database::open_in(rt, "app.adt", &Log::default(), &options())
            .expect("an empty log is a prefix of the file's");
        assert_eq!(vec![r#""ann""#], query(&db, NAMES));
    });
}

#[test]
fn a_refused_first_migration_fails_open() {
    once(5, |rt| {
        let unindexed = SHOP.replace(
            "where not exists p in payments where p.customer == key c",
            "where not exists n in notes where n.body == c.name",
        );
        let got = Database::open_in(rt, "app.adt", &Log::new([unindexed]), &options());
        match got {
            Err(OpenError::MigrationRefused {
                version: 1,
                reason:
                    Error::Rejected {
                        reason: RejectReason::SchemaRefused { detail, .. },
                    },
            }) => assert!(detail.contains("shop::paid"), "{detail}"),
            other => panic!("not refused: {other:?}"),
        }
    });
}

#[test]
fn a_second_migration_applies_at_open() {
    let mut sim = Sim::new(6, Config::default());
    run(&mut sim, |rt| drop(open(&rt)));
    run(&mut sim, |rt| {
        let next = Log::new([
            SHOP,
            "module shop\nadd type Label = { id: Key tags, name: Text }\nadd relation tags : Label key id generate sequence\n",
        ]);
        let db = Database::open_in(rt, "app.adt", &next, &options())
            .expect("the second migration applies");
        assert!(query(&db, "from t in tags select t.name").is_empty());
    });
}

#[test]
fn text_that_does_not_parse_or_check_is_invalid() {
    once(0, |rt| {
        let db = open(&rt);
        for t in [
            "from c in",
            "from c in nothing select c",
            "from c in customers where c.name == 1 select c",
            "from c in customers where c.name == n select c",
        ] {
            let got = db.query_text(t, &[], &CallOptions::default());
            assert!(
                got.as_ref().is_err_and(is_invalid),
                "`{t}` is not invalid: {got:?}"
            );
        }
    });
}

#[test]
fn parameters_bind_free_names_by_name() {
    once(0, |rt| {
        let db = open(&rt);
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
        let db = open(&rt);
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
        let db = open(&rt);
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
fn a_key_violation_names_the_key_in_the_textual_syntax() {
    once(0, |rt| {
        let db = open(&rt);
        let one = r#"insert notes { id: notes.key(1), body: "a" }"#;
        db.query_text(one, &[], &CallOptions::default())
            .expect("the first insert");
        let err = db
            .query_text(one, &[], &CallOptions::default())
            .expect_err("the key is held");
        match reason(&err) {
            RejectReason::KeyViolation { relation, key } => {
                assert_eq!("shop::notes", relation);
                assert_eq!("notes.key(1)", key);
            }
            other => panic!("{other:?}"),
        }
    });
}

#[test]
fn a_violated_constraint_is_rejected_naming_it_with_its_rows() {
    once(0, |rt| {
        let db = open(&rt);
        let err = db
            .query_text(
                r#"insert customers { name: "ann" }"#,
                &[],
                &CallOptions::default(),
            )
            .expect_err("a customer without a payment");
        match reason(&err) {
            RejectReason::Violated {
                constraint, report, ..
            } => {
                assert_eq!("shop::paid", constraint);
                assert_eq!(1, report.len(), "{report:?}");
            }
            other => panic!("{other:?}"),
        }
        assert!(query(&db, NAMES).is_empty());
    });
}

#[test]
fn a_fail_in_a_block_is_a_user_error() {
    once(0, |rt| {
        let db = open(&rt);
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
        let db = open(&rt);
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
            let db = open(&rt);
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
        let db = open(&rt);
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
fn a_failing_call_undoes_all_its_items_and_leaves_the_transaction_open() {
    once(0, |rt| {
        let db = open(&rt);
        let mut t = db.begin(&TxnOptions::default()).expect("begin");
        t.query_text(
            r#"insert notes { body: "kept" }"#,
            &[],
            &TxnCallOptions::default(),
        )
        .expect("the first call");
        let err = t
            .query_text(
                r#"insert notes { body: "undone" }
insert notes { id: notes.key(1), body: "collides" }"#,
                &[],
                &TxnCallOptions::default(),
            )
            .expect_err("the second item collides");
        assert!(
            matches!(reason(&err), RejectReason::KeyViolation { .. }),
            "{err:?}"
        );
        assert_eq!(
            vec![r#""kept""#],
            in_txn(&mut t, "from n in notes select n.body")
        );
        t.query_text(
            r#"insert notes { body: "later" }"#,
            &[],
            &TxnCallOptions::default(),
        )
        .expect("a later call");
        t.commit(None).expect("commit");
        assert_eq!(
            vec![r#""kept""#, r#""later""#],
            query(&db, "from n in notes select n.body")
        );
    });
}

#[test]
fn constraints_hold_after_every_call_of_an_interactive_transaction() {
    once(0, |rt| {
        let db = open(&rt);
        let mut t = db.begin(&TxnOptions::default()).expect("begin");
        let err = t
            .query_text(
                "insert customers { name: n }",
                &name("ann"),
                &TxnCallOptions::default(),
            )
            .expect_err("a customer without a payment");
        assert!(
            matches!(reason(&err), RejectReason::Violated { constraint, .. } if constraint == "shop::paid"),
            "{err:?}"
        );
        t.query_text(SIGN_UP, &name("ann"), &TxnCallOptions::default())
            .expect("one call inserting both");
        t.commit(None).expect("commit");
        assert_eq!(vec![r#""ann""#], query(&db, NAMES));
    });
}

#[test]
fn a_read_only_transaction_refuses_writes() {
    once(0, |rt| {
        let db = open(&rt);
        let opts = TxnOptions {
            read_only: true,
            ..TxnOptions::default()
        };
        let mut t = db.begin(&opts).expect("begin");
        assert!(in_txn(&mut t, NAMES).is_empty());
        let got = t.query_text(
            r#"insert notes { body: "a" }"#,
            &[],
            &TxnCallOptions::default(),
        );
        assert!(got.as_ref().is_err_and(is_invalid), "{got:?}");
        t.commit(None).expect("commit");
    });
}

#[test]
fn a_read_only_transaction_refuses_a_fail_as_a_write() {
    once(0, |rt| {
        let db = open(&rt);
        let opts = TxnOptions {
            read_only: true,
            ..TxnOptions::default()
        };
        let mut t = db.begin(&opts).expect("begin");
        let got = t.query_text(
            "if count(n for n in notes) == 0 then fail \"empty\" else skip",
            &[],
            &TxnCallOptions::default(),
        );
        let refused = Error::Rejected {
            reason: RejectReason::Invalid {
                detail: "a read-only transaction does not write".into(),
            },
        };
        assert_eq!(Some(&refused), got.as_ref().err());
        t.commit(None).expect("commit");
    });
}

#[test]
fn a_deferrable_read_only_transaction_reads_and_commits() {
    once(0, |rt| {
        let db = open(&rt);
        sign_up(&db, "ann");
        let opts = TxnOptions {
            read_only: true,
            deferrable: true,
            ..TxnOptions::default()
        };
        let mut t = db.begin(&opts).expect("a deferrable transaction begins");
        assert_eq!(vec![r#""ann""#], in_txn(&mut t, NAMES));
        t.commit(None).expect("commit");
    });
}

#[test]
fn a_deferrable_block_runs_as_ad_hoc_text() {
    once(0, |rt| {
        let db = open(&rt);
        sign_up(&db, "ann");
        assert_eq!(
            vec!["1"],
            query(
                &db,
                "transact read_only deferrable\n  return count(c for c in customers)"
            )
        );
    });
}

#[test]
fn only_a_read_only_transaction_is_deferrable() {
    once(0, |rt| {
        let db = open(&rt);
        let opts = TxnOptions {
            deferrable: true,
            ..TxnOptions::default()
        };
        let got = db.begin(&opts).map(drop);
        assert!(got.as_ref().is_err_and(is_invalid), "{got:?}");
    });
}

#[test]
fn a_deferrable_transaction_begins_once_the_writers_running_at_its_snapshot_end() {
    for seed in SEEDS {
        once(seed, |rt| {
            let db = open(&rt);
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
        let db = open(&rt);
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
            ..options()
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
        let db = open(&rt);
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
        let db = open(&rt);
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
fn a_deadline_is_refused_until_the_engine_runs_one() {
    once(0, |rt| {
        let db = open(&rt);
        let opts = CallOptions {
            deadline: Some(std::time::SystemTime::UNIX_EPOCH),
            ..CallOptions::default()
        };
        let got = db.query_text(NAMES, &[], &opts);
        assert!(got.as_ref().is_err_and(is_invalid), "{got:?}");
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
            let db = open(&rt);
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
            let db = open(&rt);
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
            let db = open(&rt);
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
        let db = open(&rt);
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
        let db = open(&rt);
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
            let db = open(&rt);
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

/// Returns what `verify` finds in the closed file `app.adt`, and whether its counters page is
/// named.
fn verified(rt: SimRuntime) -> (adtb::verify::Report, bool) {
    let options = adtb::storage::Options {
        page_size: 8192,
        threads: 4,
        ..adtb::storage::Options::new("app.adt")
    };
    let (pager, _) = adtb::storage::Pager::open(rt.clone(), &options).expect("the pager");
    let (catalog, txns) = adtb::catalog::Catalog::open(rt, &pager).expect("the catalog");
    let report = adtb::verify::verify(&pager, &catalog.inventory(&txns.commit_log()));
    (report, catalog.sequences().is_some())
}

#[test]
fn a_closed_file_verifies_clean_with_every_page_accounted_for() {
    let mut sim = Sim::new(7, Config::default());
    run(&mut sim, |rt| {
        let db = open(&rt);
        sign_up(&db, "ann");
        db.close().expect("close");
    });
    run(&mut sim, |rt| drop(open(&rt)));
    let (report, named) = run(&mut sim, verified);
    assert!(named, "the counters page is named");
    assert!(report.is_clean() && report.complete, "{report:#?}");
}

#[test]
fn a_crash_as_the_first_open_names_its_counters_page_leaks_no_page() {
    let seed = 11;
    let mut sim = Sim::new(seed, Config::default());
    sim.record();
    run(&mut sim, |rt| std::mem::forget(open(&rt)));
    let recording = sim.recording();
    let flushes = recording.flushes();
    let from = flushes[flushes.len().saturating_sub(3)];
    for at in from..=recording.len() {
        let files = recording.images(at, 1, seed + at as u64).swap_remove(0);
        let mut sim = Sim::with_files(seed + at as u64, Config::default(), files);
        run(&mut sim, |rt| open(&rt).close().expect("close"));
        let (report, named) = run(&mut sim, verified);
        assert!(named, "crash at {at}: the counters page is named");
        assert!(
            report.is_clean() && report.complete,
            "crash at {at}: {report:#?}"
        );
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
        &options(),
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
                &options(),
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
