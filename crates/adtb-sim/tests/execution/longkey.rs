use adtb_simkit::api;
use adtb_simkit::open;

use adtb::{CallOptions, Error, Log, RejectReason};
use adtb_sim::{Config, Sim};

use adtb_simkit::open::options;
use api::{result, run, sorted};

const BASE: &str = "module app\nadd module app\n\
    add type Item = { id: I64, text: Text }\n\
    add relation items : Item key id\n";

fn insert(id: i64, text: &str) -> String {
    format!(r#"insert items {{ id: {id}, text: "{text}" }}"#)
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
fn oversized_append_aborts_the_whole_load() {
    use adtb::batch::BatchBuilder;
    use adtb::key::Value;
    use adtb::{LoadOptions, RecordBatch};

    let schema = format!("{BASE}  index (text)\n");
    let mut sim = Sim::new(7, Config::default());
    run(&mut sim, |rt| {
        let db = open::open(
            &rt,
            open::FILE,
            &Log::new([&schema].iter().copied()),
            &options(2),
        )
        .expect("create");
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
        let db = open::open(
            &rt,
            open::FILE,
            &Log::new([&schema].iter().copied()),
            &options(2),
        )
        .expect("reopen");
        assert_eq!(
            vec!["0", "3"],
            sorted(result(&db, "from i in items select i.id"))
        );
        db.close().expect("close");
    });
}
