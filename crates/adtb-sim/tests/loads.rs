//! Bulk append in process over the simulator: Arrow batches pushed to the appenders of a load,
//! checked against the row types of their constructor relations, and inserted as one change set
//! at commit, all or nothing, a crash included.

#[expect(dead_code, reason = "this binary uses part of the API helpers")]
#[path = "support/api.rs"]
mod api;

use std::sync::Arc;

use adtb::batch::BatchBuilder;
use adtb::key::Value as Stored;
use adtb::{
    ArrowSchema, CallOptions, CancelToken, Database, Error, Field, Load, LoadOptions, Log,
    RecordBatch, RejectReason,
};
use adtb_sim::{Config, Sim, SimRuntime};

use api::{once, options, printed, result, run};

const SHOP: &str = r#"module shop
add module shop
add type Customer = { id: Key customers, name: Text }
add relation customers : Customer key id generate sequence
add type Payment { id: Key payments, customer: Key customers }
  = Card { last4: Text }
  | Invoice { due: Text }
add relation payments : Payment
  key id generate sequence
  index (customer)
add type Note = { id: Key notes, body: Text, tag: Text }
add relation notes : Note key id generate sequence
  default tag = "none"
add constraint paid =
  from c in customers
  where not exists p in payments where p.customer == key c
  select c
"#;

fn log() -> Log {
    Log::new([SHOP])
}

fn open(rt: &SimRuntime) -> Database<SimRuntime> {
    Database::open_in(rt.clone(), "app.adt", &log(), &options(4)).expect("the database opens")
}

fn query(db: &Database<SimRuntime>, text: &str) -> Vec<String> {
    let mut out = printed(result(db, text));
    out.sort();
    out
}

fn reason(e: &Error) -> &RejectReason {
    match e {
        Error::Rejected { reason } => reason,
        other => panic!("not a rejection: {other:?}"),
    }
}

fn text(t: &str) -> Stored {
    Stored::Text(t.into())
}

/// Returns a batch of `rows` over the fields of `full` named in `names`, in that order.
fn batch(full: &ArrowSchema, names: &[&str], rows: &[Vec<Stored>]) -> RecordBatch {
    let fields: Vec<Field> = names
        .iter()
        .map(|n| {
            full.fields
                .iter()
                .find(|f| f.name == *n)
                .unwrap_or_else(|| panic!("the row has field {n}"))
                .clone()
        })
        .collect();
    let schema = Arc::new(ArrowSchema { fields });
    let mut b = BatchBuilder::new(schema.batch_schema());
    for r in rows {
        b.push_row(r);
    }
    RecordBatch::new(schema, b.finish()).expect("the batch is of its schema")
}

/// Push `rows` of the fields `names` to the appender of `target` in `load`.
fn push(
    load: &mut Load<'_, SimRuntime>,
    target: &str,
    names: &[&str],
    rows: &[Vec<Stored>],
) -> Result<(), Error> {
    let mut a = load.appender(target).expect("the appender");
    let b = batch(a.schema(), names, rows);
    a.push(&b)
}

/// Stage a customer of key `k` named `name` paying by card, and commit nothing yet.
fn customer(load: &mut Load<'_, SimRuntime>, k: i64, name: &str) {
    push(
        load,
        "customers",
        &["id", "name"],
        &[vec![Stored::I64(k), text(name)]],
    )
    .expect("a customer");
    push(
        load,
        "payments.Card",
        &["customer", "last4"],
        &[vec![Stored::I64(k), text("4242")]],
    )
    .expect("a card");
}

#[test]
fn an_appender_offers_the_row_type_of_its_constructor_relation() {
    once(0, |rt| {
        let db = open(&rt);
        let mut load = db.load(&LoadOptions::default()).expect("a load");
        let card = load.appender("payments.Card").expect("payments.Card");
        let names: Vec<&str> = card
            .schema()
            .fields
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(vec!["id", "customer", "last4"], names);
        let notes = load
            .appender("shop::notes")
            .expect("a qualified record relation");
        assert_eq!(3, notes.schema().fields.len());
        for unknown in ["payments", "payments.Cash", "nothing", "nothing.Card"] {
            let got = load.appender(unknown).map(drop);
            assert!(
                matches!(
                    got,
                    Err(Error::Rejected {
                        reason: RejectReason::Invalid { .. }
                    })
                ),
                "{unknown}: {got:?}"
            );
        }
    });
}

#[test]
fn a_load_into_one_relation_commits_every_row_and_fills_generated_keys_and_defaults() {
    once(1, |rt| {
        let db = open(&rt);
        let mut load = db.load(&LoadOptions::default()).expect("a load");
        let rows: Vec<Vec<Stored>> = (0..50).map(|i| vec![text(&format!("n{i:02}"))]).collect();
        push(&mut load, "notes", &["body"], &rows).expect("the notes push");
        push(&mut load, "notes", &["body"], &[vec![text("late")]]).expect("a second batch");
        let report = load.commit().expect("the load commits");
        assert_eq!(51, report.rows);
        assert_eq!(vec!["51"], query(&db, "count(n for n in notes)"));
        assert_eq!(
            vec![r#""none""#],
            query(&db, "from n in notes select n.tag"),
            "the default fills the field the batches leave out"
        );
        assert_eq!(
            vec!["51"],
            query(&db, "count(k for k in (from n in notes select key n))"),
            "each row draws a key of its own"
        );
    });
}

#[test]
fn equal_rows_of_a_load_are_each_inserted() {
    once(2, |rt| {
        let db = open(&rt);
        let mut load = db.load(&LoadOptions::default()).expect("a load");
        let same = vec![text("same")];
        push(&mut load, "notes", &["body"], &[same.clone(), same]).expect("the push");
        assert_eq!(2, load.commit().expect("commit").rows);
        assert_eq!(vec!["2"], query(&db, "count(n for n in notes)"));
    });
}

#[test]
fn a_load_into_several_relations_is_one_change_set_checked_at_commit() {
    once(3, |rt| {
        let db = open(&rt);
        let mut load = db.load(&LoadOptions::default()).expect("a load");
        customer(&mut load, 1, "ann");
        customer(&mut load, 2, "bob");
        let report = load.commit().expect("each customer lands with its payment");
        assert_eq!(4, report.rows);
        assert_eq!(
            vec![r#""ann""#, r#""bob""#],
            query(&db, "from c in customers select c.name")
        );
        assert_eq!(vec!["2"], query(&db, "count(p for p in payments.Card)"));
    });
}

#[test]
fn a_load_breaking_a_constraint_aborts_with_the_violation_and_inserts_nothing() {
    once(4, |rt| {
        let db = open(&rt);
        let mut load = db.load(&LoadOptions::default()).expect("a load");
        customer(&mut load, 1, "ann");
        push(
            &mut load,
            "customers",
            &["id", "name"],
            &[vec![Stored::I64(2), text("cy")]],
        )
        .expect("a customer without a payment");
        let err = load.commit().expect_err("cy has no payment");
        match reason(&err) {
            RejectReason::Violated { constraint, .. } => assert_eq!("shop::paid", constraint),
            other => panic!("{other:?}"),
        }
        assert!(query(&db, "from c in customers select c.name").is_empty());
        assert!(query(&db, "from p in payments select key p").is_empty());
    });
}

#[test]
fn two_staged_rows_with_one_key_fail_with_a_key_violation_at_commit() {
    once(5, |rt| {
        let db = open(&rt);
        let mut load = db.load(&LoadOptions::default()).expect("a load");
        push(
            &mut load,
            "notes",
            &["id", "body", "tag"],
            &[
                vec![Stored::I64(7), text("a"), text("x")],
                vec![Stored::I64(7), text("b"), text("x")],
            ],
        )
        .expect("rows land in staging unconstrained");
        let err = load.commit().expect_err("one key twice");
        assert!(
            matches!(reason(&err), RejectReason::KeyViolation { .. }),
            "{err:?}"
        );
        assert!(query(&db, "from n in notes select n.body").is_empty());
    });
}

#[test]
fn two_identical_staged_rows_with_one_key_fail_with_a_key_violation_at_commit() {
    once(5, |rt| {
        let db = open(&rt);
        let mut load = db.load(&LoadOptions::default()).expect("a load");
        let row = vec![Stored::I64(7), text("a"), text("x")];
        push(
            &mut load,
            "notes",
            &["id", "body", "tag"],
            &[row.clone(), row],
        )
        .expect("rows land in staging unconstrained");
        let err = load.commit().expect_err("one key twice");
        assert_eq!(
            &RejectReason::KeyViolation {
                relation: "shop::notes".into(),
                key: "notes.key(7)".into()
            },
            reason(&err)
        );
        assert!(query(&db, "from n in notes select n.body").is_empty());
    });
}

#[test]
fn a_row_equal_to_another_once_its_default_is_filled_still_violates_the_key() {
    once(5, |rt| {
        let db = open(&rt);
        let mut load = db.load(&LoadOptions::default()).expect("a load");
        push(
            &mut load,
            "notes",
            &["id", "body", "tag"],
            &[vec![Stored::I64(7), text("a"), text("none")]],
        )
        .expect("a row with its tag");
        push(
            &mut load,
            "notes",
            &["id", "body"],
            &[vec![Stored::I64(7), text("a")]],
        )
        .expect("a row with the default tag");
        let err = load.commit().expect_err("one key twice");
        assert!(
            matches!(reason(&err), RejectReason::KeyViolation { .. }),
            "{err:?}"
        );
        assert!(query(&db, "from n in notes select n.body").is_empty());
    });
}

#[test]
fn a_batch_of_another_schema_is_refused_at_push_and_stages_nothing() {
    once(6, |rt| {
        let db = open(&rt);
        let mut load = db.load(&LoadOptions::default()).expect("a load");
        let full = load.appender("notes").expect("notes").schema().clone();
        let wrong_type = Arc::new(ArrowSchema {
            fields: vec![Field {
                name: "body".into(),
                ty: full.fields[0].ty.clone(),
            }],
        });
        let mut b = BatchBuilder::new(wrong_type.batch_schema());
        b.push_row(&[Stored::I64(1)]);
        let wrong_type = RecordBatch::new(wrong_type, b.finish()).expect("a batch");
        let refused = [
            batch(&full, &["tag"], &[vec![text("x")]]),
            batch(&full, &["tag", "body"], &[vec![text("x"), text("y")]]),
            wrong_type,
        ];
        for b in &refused {
            let got = load.appender("notes").expect("notes").push(b);
            assert!(
                matches!(
                    got,
                    Err(Error::Rejected {
                        reason: RejectReason::SchemaMismatch { .. }
                    })
                ),
                "{:?}: {got:?}",
                b.schema()
            );
        }
        assert_eq!(0, load.commit().expect("an empty load commits").rows);
        assert!(query(&db, "from n in notes select n.body").is_empty());
    });
}

#[test]
fn a_load_dropped_without_commit_discards_its_staging() {
    once(7, |rt| {
        let db = open(&rt);
        let mut load = db.load(&LoadOptions::default()).expect("a load");
        push(&mut load, "notes", &["body"], &[vec![text("gone")]]).expect("the push");
        drop(load);
        assert!(query(&db, "from n in notes select n.body").is_empty());
    });
}

#[test]
fn a_key_committed_after_the_load_opened_fails_it_with_a_serialization_failure() {
    once(8, |rt| {
        let db = open(&rt);
        let mut load = db.load(&LoadOptions::default()).expect("a load");
        push(
            &mut load,
            "notes",
            &["id", "body", "tag"],
            &[vec![Stored::I64(9), text("load"), text("x")]],
        )
        .expect("the push");
        db.query_text(
            r#"insert notes { id: notes.key(9), body: "other", tag: "y" }"#,
            &[],
            &CallOptions::default(),
        )
        .expect("another writer commits the key first");
        let err = load.commit().expect_err("the key collides");
        assert!(err.is_serialization(), "{err:?}");
        assert_eq!(
            vec![r#""other""#],
            query(&db, "from n in notes select n.body")
        );
    });
}

#[test]
fn loaded_text_is_normalized_to_nfc() {
    once(9, |rt| {
        let db = open(&rt);
        let mut load = db.load(&LoadOptions::default()).expect("a load");
        push(&mut load, "notes", &["body"], &[vec![text("cafe\u{301}")]]).expect("the push");
        load.commit().expect("commit");
        assert_eq!(
            vec!["1"],
            query(&db, "count(n for n in notes where n.body == \"caf\u{e9}\")")
        );
    });
}

#[test]
fn a_cancelled_load_inserts_nothing() {
    once(10, |rt| {
        let db = open(&rt);
        let tok = CancelToken::new();
        let mut load = db
            .load(&LoadOptions {
                cancel: Some(tok.clone()),
                ..LoadOptions::default()
            })
            .expect("a load");
        push(&mut load, "notes", &["body"], &[vec![text("a")]]).expect("the push");
        tok.cancel();
        let err = load.commit().expect_err("the load is cancelled");
        assert_eq!(&RejectReason::Cancelled, reason(&err));
        assert!(query(&db, "from n in notes select n.body").is_empty());
    });
}

/// Returns the customers and cards of the file `files` holds, reopened.
fn reopened(seed: u64, files: std::collections::BTreeMap<String, Vec<u8>>) -> (usize, usize) {
    let mut sim = Sim::with_files(seed, Config::default(), files);
    run(&mut sim, |rt| {
        let db = open(&rt);
        let customers = query(&db, "from c in customers select key c").len();
        let cards = query(&db, "from p in payments.Card select key p").len();
        (customers, cards)
    })
}

#[test]
fn a_crash_during_a_loads_commit_reopens_to_all_of_it_or_none() {
    let seed = 11;
    let mut sim = Sim::new(seed, Config::default());
    sim.record();
    run(&mut sim, |rt| drop(open(&rt)));
    let start = sim.recording().len();
    run(&mut sim, |rt| {
        let db = open(&rt);
        let mut load = db.load(&LoadOptions::default()).expect("a load");
        for k in 1..=40 {
            customer(&mut load, k, &format!("c{k}"));
        }
        load.commit().expect("the load commits");
    });
    let recording = sim.recording();
    let n = recording.len();
    let mut seen = Vec::new();
    for at in (start..=n).step_by((n - start).div_ceil(16).max(1)) {
        let files = recording.images(at, 1, seed + at as u64).swap_remove(0);
        let got = reopened(seed + at as u64, files);
        assert!(got == (0, 0) || got == (40, 40), "at event {at}: {got:?}");
        seen.push(got);
    }
    let files = recording.images(n, 1, seed).swap_remove(0);
    assert_eq!(
        (40, 40),
        reopened(seed, files),
        "the acknowledged load survives"
    );
    assert!(
        seen.windows(2).all(|w| w[0] <= w[1]),
        "a later event reopens to less: {seen:?}"
    );
}
