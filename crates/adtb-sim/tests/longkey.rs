#[expect(
    dead_code,
    reason = "sharing API helpers across binaries would remove unused helpers"
)]
#[path = "support/api.rs"]
mod api;

use adtb::{
    CallOptions, Database, Error, Log, OpenError, RejectReason, TxnCallOptions, TxnOptions,
};
use adtb_sim::{Config, Sim, SimRuntime};

use api::{options, result, run, sorted};

const BASE: &str = "module app\nadd module app\n\
    add type Item = { id: I64, text: Text }\n\
    add relation items : Item key id\n";

fn open(rt: SimRuntime, log: &[&str]) -> Result<Database<SimRuntime>, OpenError> {
    Database::open_in(rt, "app.adt", &Log::new(log.iter().copied()), &options(2))
}

fn insert(id: i64, text: &str) -> String {
    format!(r#"insert items {{ id: {id}, text: "{text}" }}"#)
}

#[test]
fn oversized_primary_key_rejects_only_the_statement() {
    rejected_write(&BASE.replace("key id", "key text"), "key text", 0, 0);
}

#[test]
fn oversized_unique_key_rejects_only_the_statement() {
    rejected_write(&format!("{BASE}  unique (text)\n"), "unique (text)", 0, 1);
}

#[test]
fn oversized_nonunique_key_rejects_only_the_statement() {
    rejected_write(&format!("{BASE}  index (text)\n"), "index (text)", 0, 2);
}

#[test]
fn oversized_composite_key_rejects_only_the_statement() {
    rejected_write(
        &format!("{BASE}  index (id, text)\n"),
        "index (id, text)",
        8,
        3,
    );
}

#[test]
fn oversized_expression_key_rejects_only_the_statement() {
    rejected_write(
        &format!("{BASE}  index (lower(text))\n"),
        "index (lower(text))",
        0,
        4,
    );
}

fn rejected_write(schema: &str, clause: &str, extra: i64, seed: u64) {
    let mut sim = Sim::new(seed, Config::default());
    run(&mut sim, |rt| {
        let db = open(rt, &[schema]).expect("create");
        let mut tx = db.begin(&TxnOptions::default()).expect("begin");
        tx.query_text(&insert(2, "before"), &[], &TxnCallOptions::default())
            .expect("an earlier statement");
        let failed = tx
            .query_text(
                &format!(
                    "({}, {})",
                    insert(0, "small"),
                    insert(1, &"x".repeat(20_000))
                ),
                &[],
                &TxnCallOptions::default(),
            )
            .expect_err("an oversized index key");
        assert_key_error(&failed, "Item", clause, 20_002 + extra, 1607);
        let fitting = "X".repeat(1605 - extra as usize);
        tx.query_text(&insert(3, &fitting), &[], &TxnCallOptions::default())
            .expect("an exact-boundary key fits");
        let error = tx
            .query_text(
                &insert(4, &(fitting + "x")),
                &[],
                &TxnCallOptions::default(),
            )
            .expect_err("one payload byte over");
        assert_key_error(&error, "Item", clause, 1608, 1607);
        tx.query_text(
            "delete i in items where i.id == 3",
            &[],
            &TxnCallOptions::default(),
        )
        .expect("remove the boundary row");
        assert_eq!(
            vec![r#""before""#],
            api::in_txn(&mut tx, "from i in items select i.text"),
            "seed {seed}"
        );
        tx.query_text(&insert(1, "usable"), &[], &TxnCallOptions::default())
            .expect("the transaction remains usable");
        tx.commit(None).expect("commit");
        db.close().expect("close");
    });
    run(&mut sim, |rt| {
        let db = open(rt, &[schema]).expect("reopen");
        assert_eq!(
            vec![r#""before""#, r#""usable""#],
            sorted(result(&db, "from i in items select i.text")),
            "seed {seed}"
        );
        db.close().expect("close");
    });
}

#[test]
fn migration_built_oversized_unique_is_refused_atomically() {
    refused_migration("add unique (text) in items", 1605, 0);
}

#[test]
fn migration_built_oversized_index_is_refused_atomically() {
    refused_migration("add index (text) in items", 1605, 1);
}

#[test]
fn migration_built_oversized_composite_index_is_refused_atomically() {
    refused_migration("add index (id, text) in items", 1597, 2);
}

#[test]
fn migration_built_oversized_expression_index_is_refused_atomically() {
    refused_migration("add index (lower(text)) in items", 1605, 3);
}

#[test]
fn migration_built_oversized_primary_key_is_refused_atomically() {
    refused_migration("rekey relation items key text", 1605, 4);
}

fn refused_migration(operator: &str, fitting_len: usize, seed: u64) {
    let migration = format!("module app\n{operator}\n");
    let fitting = "x".repeat(fitting_len);
    let mut sim = Sim::new(seed, Config::default());
    run(&mut sim, |rt| {
        let db = open(rt, &[BASE]).expect("create");
        db.query_text(
            &insert(1, &"x".repeat(20_000)),
            &[],
            &CallOptions::default(),
        )
        .expect("large unindexed text is valid");
        db.close().expect("close");
    });
    run(&mut sim, |rt| {
        let failed =
            open(rt, &[BASE, &migration]).expect_err("the migration exceeds the index limit");
        match failed {
            OpenError::MigrationRefused {
                version: 2,
                reason:
                    Error::Rejected {
                        reason: RejectReason::SchemaRefused { detail, .. },
                    },
            } => {
                assert!(detail.contains("app::items."), "seed {seed}: {detail}");
                let size = 20_002 + 1605 - fitting_len;
                assert!(
                    detail.contains(&format!("key payload of {size} bytes")),
                    "seed {seed}: {detail}"
                );
                assert!(
                    detail.contains("the limit is 1607 bytes"),
                    "seed {seed}: {detail}"
                );
            }
            other => panic!("seed {seed}: {other:?}"),
        }
    });
    run(&mut sim, |rt| {
        let db = open(rt, &[BASE]).expect("the old schema reopens");
        assert_eq!(
            vec!["1"],
            sorted(result(&db, "from i in items select i.id")),
            "seed {seed}"
        );
        db.query_text(
            &format!(r#"update i in items set Item {{ i with text: "{fitting}" }}"#),
            &[],
            &CallOptions::default(),
        )
        .expect("the old schema remains writable");
        db.close().expect("close");
    });
    run(&mut sim, |rt| {
        let db = open(rt, &[BASE, &migration]).expect("the migration accepts a fitting key");
        assert_eq!(
            vec![format!("{fitting:?}")],
            sorted(result(&db, "from i in items select i.text")),
            "seed {seed}"
        );
        db.close().expect("close");
    });
}

fn assert_key_error(error: &Error, ctor: &str, clause: &str, size: i64, limit: i64) {
    assert_eq!(
        &Error::Rejected {
            reason: RejectReason::KeyTooLarge {
                relation: "app::items".into(),
                ctor: ctor.into(),
                index: clause.into(),
                size,
                limit,
            },
        },
        error
    );
    let value = error.value().to_string();
    assert!(value.contains("KeyTooLarge"), "{value}");
    assert!(value.contains(&format!("index: {clause:?}")), "{value}");
    assert!(error.to_string().contains("key payload"));
}

#[test]
fn oversized_update_preserves_old_row_and_transaction() {
    let schema = format!("{BASE}  index (text)\n");
    let mut sim = Sim::new(5, Config::default());
    run(&mut sim, |rt| {
        let db = open(rt, &[&schema]).expect("create");
        let mut tx = db.begin(&TxnOptions::default()).expect("begin");
        tx.query_text(&insert(1, "old"), &[], &TxnCallOptions::default())
            .expect("insert");
        let huge = "x".repeat(20_000);
        let error = tx
            .query_text(
                &format!(r#"update i in items set Item {{ i with text: "{huge}" }}"#),
                &[],
                &TxnCallOptions::default(),
            )
            .expect_err("oversized update");
        assert_key_error(&error, "Item", "index (text)", 20_002, 1607);
        assert_eq!(
            vec![r#""old""#],
            api::in_txn(&mut tx, "from i in items select i.text")
        );
        tx.query_text(&insert(2, "usable"), &[], &TxnCallOptions::default())
            .expect("usable");
        tx.commit(None).expect("commit");
        db.close().expect("close");
    });
    run(&mut sim, |rt| {
        let db = open(rt, &[&schema]).expect("reopen");
        assert_eq!(
            vec![r#""old""#, r#""usable""#],
            sorted(result(&db, "from i in items select i.text"))
        );
        db.close().expect("close");
    });
}

#[test]
fn oversized_retag_reports_destination_constructor_and_scope() {
    let schema = "module app\nadd module app\n\
        add type Item { id: I64 } = Plain | Named { text: Text }\n\
        add relation items : Item key id\n  index (text) in Named\n";
    let mut sim = Sim::new(6, Config::default());
    run(&mut sim, |rt| {
        let db = open(rt, &[schema]).expect("create");
        let mut tx = db.begin(&TxnOptions::default()).expect("begin");
        tx.query_text(
            "insert items.Plain { id: 1 }",
            &[],
            &TxnCallOptions::default(),
        )
        .expect("insert");
        let huge = "x".repeat(20_000);
        let error = tx
            .query_text(
                &format!(r#"update i in items.Plain set Named {{ text: "{huge}" }}"#),
                &[],
                &TxnCallOptions::default(),
            )
            .expect_err("oversized retag");
        assert_key_error(&error, "Named", "index (text) in Named", 20_002, 1607);
        assert_eq!(
            vec!["1"],
            api::in_txn(&mut tx, "from i in items.Plain select i.id")
        );
        tx.query_text(
            "update i in items.Plain set Named { text: \"usable\" }",
            &[],
            &TxnCallOptions::default(),
        )
        .expect("usable");
        tx.commit(None).expect("commit");
        db.close().expect("close");
    });
    run(&mut sim, |rt| {
        let db = open(rt, &[schema]).expect("reopen");
        assert_eq!(
            vec![r#""usable""#],
            sorted(result(&db, "from i in items.Named select i.text"))
        );
        db.close().expect("close");
    });
}

#[test]
fn oversized_append_aborts_the_whole_load() {
    use adtb::batch::BatchBuilder;
    use adtb::key::Value;
    use adtb::{LoadOptions, RecordBatch};

    let schema = format!("{BASE}  index (text)\n");
    let mut sim = Sim::new(7, Config::default());
    run(&mut sim, |rt| {
        let db = open(rt, &[&schema]).expect("create");
        db.query_text(&insert(0, "before"), &[], &CallOptions::default())
            .expect("before");
        let mut load = db.load(&LoadOptions::default()).expect("load");
        let mut appender = load.appender("items").expect("appender");
        let arrow = appender.schema().clone();
        let mut builder = BatchBuilder::new(arrow.batch_schema());
        builder.push_row(&[Value::I64(1), Value::Text("small".into())]);
        builder.push_row(&[Value::I64(2), Value::Text("x".repeat(20_000))]);
        appender
            .push(&RecordBatch::new(arrow, builder.finish()).expect("batch"))
            .expect("stage");
        let error = load.commit().expect_err("oversized append");
        assert_key_error(&error, "Item", "index (text)", 20_002, 1607);
        assert_eq!(
            vec!["0"],
            sorted(result(&db, "from i in items select i.id"))
        );
        db.query_text(&insert(3, "usable"), &[], &CallOptions::default())
            .expect("usable");
        db.close().expect("close");
    });
    run(&mut sim, |rt| {
        let db = open(rt, &[&schema]).expect("reopen");
        assert_eq!(
            vec!["0", "3"],
            sorted(result(&db, "from i in items select i.id"))
        );
        db.close().expect("close");
    });
}

#[test]
fn oversized_keys_precede_key_conflicts() {
    let schema = format!("{BASE}  index (text)\n");
    let mut sim = Sim::new(8, Config::default());
    run(&mut sim, |rt| {
        let db = open(rt, &[&schema]).expect("create");
        let error = db
            .query_text(
                &format!(
                    "({}, {})",
                    insert(1, "small"),
                    insert(1, &"x".repeat(20_000))
                ),
                &[],
                &CallOptions::default(),
            )
            .expect_err("size validation precedes duplicate keys");
        assert_key_error(&error, "Item", "index (text)", 20_002, 1607);
        assert!(sorted(result(&db, "from i in items select i")).is_empty());
        db.close().expect("close");
    });
}

#[test]
fn index_expression_fault_remains_a_fault() {
    let schema = format!("{BASE}  index (1 / id)\n");
    let mut sim = Sim::new(9, Config::default());
    run(&mut sim, |rt| {
        let db = open(rt, &[&schema]).expect("create");
        let mut tx = db.begin(&TxnOptions::default()).expect("begin");
        let error = tx
            .query_text(&insert(0, "fault"), &[], &TxnCallOptions::default())
            .expect_err("index expression faults");
        assert_eq!(
            Error::Rejected {
                reason: RejectReason::Faulted {
                    fault: adtb::exec::Fault::DivisionByZero
                }
            },
            error
        );
        assert!(api::in_txn(&mut tx, "from i in items select i").is_empty());
        tx.query_text(&insert(1, "usable"), &[], &TxnCallOptions::default())
            .expect("usable");
        tx.commit(None).expect("commit");
        db.close().expect("close");
    });
}
