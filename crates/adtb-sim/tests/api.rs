//! The in-process API over the simulator: open, ad hoc text, interactive transactions and the
//! retrying closure.

use std::sync::{Arc, Mutex};

use adtb::runtime::Threads;
use adtb::{
    AdHocTransaction, CallOptions, CreateOptions, Database, DynRows, Error, Log, OpenError,
    OpenOptions, RejectReason, RetryReason, TxnCallOptions, TxnOptions, Value,
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
    rows.map(|r| print(&r.expect("a row"), &form)).collect()
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
        let unindexed = SHOP.replace("  index (customer)\n", "");
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
            "module shop\nadd type Tag = { id: Key tags, name: Text }\nadd relation tags : Tag key id generate sequence\n",
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
            let at = t.commit().expect("commit");
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
        t.commit().expect("commit");
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
        t.commit().expect("commit");
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
        t.commit().expect("commit");
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
        };
        let mut t = db.begin(&opts).expect("a deferrable transaction begins");
        assert_eq!(vec![r#""ann""#], in_txn(&mut t, NAMES));
        t.commit().expect("commit");
    });
}

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
            a.commit().expect("a commits");
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
            assert_eq!(Err(err), b.commit(), "seed {seed}: the commit");
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
            assert_eq!(vec!["0"], in_txn(&mut a, count));
            assert_eq!(vec!["0"], in_txn(&mut b, count));
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
            let ca = wa.and_then(|_| a.commit());
            let cb = wb.and_then(|_| b.commit());
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

#[test]
fn a_closed_file_verifies_clean_with_every_page_accounted_for() {
    let mut sim = Sim::new(7, Config::default());
    run(&mut sim, |rt| {
        let db = open(&rt);
        sign_up(&db, "ann");
        db.close().expect("close");
    });
    run(&mut sim, |rt| drop(open(&rt)));
    run(&mut sim, |rt| {
        let options = adtb::storage::Options {
            page_size: 8192,
            threads: 4,
            ..adtb::storage::Options::new("app.adt")
        };
        let (pager, _) = adtb::storage::Pager::open(rt.clone(), &options).expect("the pager");
        let (catalog, txns) = adtb::catalog::Catalog::open(rt, &pager).expect("the catalog");
        assert!(catalog.sequences().is_some(), "the counters page is named");
        let report = adtb::verify::verify(&pager, &catalog.inventory(&txns.commit_log()));
        assert!(report.is_clean() && report.complete, "{report:#?}");
    });
}
