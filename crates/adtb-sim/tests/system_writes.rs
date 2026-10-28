#[expect(
    dead_code,
    reason = "split API helpers into modules imported by every consumer"
)]
#[path = "support/api.rs"]
mod api;

use adtb::runtime::{Clock, Done, Io, Op, Runtime, Threads};
use adtb::{
    BundleDef, CallOptions, Database, Error, LoadOptions, Log, OpenError, OpenOptions,
    Registration, RejectReason, TxnCallOptions, TxnOptions,
};
use adtb_lang::schema::ValueDef;
use adtb_sim::SimRuntime;
use std::fmt::Debug;
use std::time::Duration;

const SCHEMA: &str = "module example
add module example
add type Item = { id: I64 }
add relation items : Item key id
define fn invoke (g: I64 -> I64, n: I64) -> I64 = g(n)
";
const INSERT_MAPPING: &str = r#"insert catalog::peer_mappings {
    listener: "local", ordinal: 1, pattern: "legacy", role: "web"
}"#;

#[test]
fn ordinary_program_insert_into_catalog_is_denied() {
    api::once(0, |rt| {
        let db = Database::open_in(rt, "system.adt", &Log::new([SCHEMA]), &api::options(2))
            .expect("open database");
        let result = db.query_text(INSERT_MAPPING, &[], &CallOptions::default());
        assert!(
            matches!(
                result,
                Err(Error::Rejected {
                    reason: RejectReason::Denied { .. }
                })
            ),
            "ordinary catalog insert must be Denied, got {result:?}"
        );
        assert!(
            api::printed(api::result(
                &db,
                "from m in catalog::peer_mappings select m"
            ))
            .is_empty()
        );
        db.close().expect("close database");
    });
}

fn open(rt: SimRuntime) -> Database<SimRuntime> {
    Database::open_in(rt, "system.adt", &Log::new([SCHEMA]), &api::options(2))
        .expect("open database")
}

fn denied<T: Debug>(result: Result<T, Error>, relation: &str) {
    let expected = format!("programs cannot write system relation `{relation}`");
    assert!(
        matches!(&result, Err(Error::Rejected { reason: RejectReason::Denied { detail } }) if detail == &expected),
        "expected Denied for {relation}, got {result:?}"
    );
}

fn mappings_empty(db: &Database<SimRuntime>) {
    assert!(api::printed(api::result(db, "from m in catalog::peer_mappings select m")).is_empty());
}

#[test]
fn every_system_relation_refuses_even_an_empty_delete_and_appender() {
    api::once(0, |rt| {
        let db = open(rt);
        let mut load = db.load(&LoadOptions::default()).expect("open load");
        let mut relations = 0;
        for module in &db.schema().modules {
            if !module.engine {
                continue;
            }
            for (name, value) in &module.values {
                let ValueDef::Relation(rel) = value else {
                    continue;
                };
                relations += 1;
                let name = format!("{}::{name}", module.name);
                denied(
                    db.query_text(
                        &format!("delete x in {name} where false"),
                        &[],
                        &CallOptions::default(),
                    ),
                    &name,
                );
                let sum = db.schema().ty(db.schema().relation(*rel).elem_def);
                for ctor in sum.ctors() {
                    denied(
                        load.appender(format!("{name}.{}", ctor.name).as_str()),
                        &name,
                    );
                }
            }
        }
        assert!(relations > 0, "system relations were checked");
        load.commit().expect("refused appenders staged no rows");
        mappings_empty(&db);
        db.close().expect("close database");
    });
}

#[test]
fn every_program_write_shape_is_checked_before_dead_code_elimination() {
    api::once(0, |rt| {
        let db = open(rt);
        let cases = [
            format!("if false then {INSERT_MAPPING} else skip"),
            format!("{INSERT_MAPPING} unless conflict on key else skip"),
            "update m in catalog::peer_mappings where false set catalog::PeerMapping { m with pattern: \"changed\" }".to_owned(),
            "delete m in catalog::peer_mappings where false".to_owned(),
            format!("transact\n insert items {{ id: 1 }}\n {INSERT_MAPPING}\n return {{}}"),
            "insert items { id: if (value(key (insert catalog::peer_mappings { listener: \"local\", ordinal: 1, pattern: \"legacy\", role: \"web\" }))).ordinal == 1 then 1 else 0 }".to_owned(),
            format!("explain analyze {INSERT_MAPPING}"),
            "insert items { id: invoke(fn (x: I64) -> x, if (value(key (insert catalog::peer_mappings { listener: \"local\", ordinal: 1, pattern: \"legacy\", role: \"web\" }))).ordinal == 1 then 1 else 0) }".to_owned(),
        ];
        for text in cases {
            denied(
                db.query_text(&text, &[], &CallOptions::default()),
                "catalog::peer_mappings",
            );
            mappings_empty(&db);
            assert!(
                api::printed(api::result(&db, "from i in items select i")).is_empty(),
                "{text}"
            );
        }
        denied(db.query_text("update t in catalog::types.SumType where false set catalog::TypeDef.Newtype { t with over: \"I64\" }",
                             &[], &CallOptions::default()), "catalog::types");
        db.close().expect("close database");
    });
}

#[test]
fn a_denied_interactive_call_keeps_prior_user_changes_and_transaction_open() {
    api::once(0, |rt| {
        let db = open(rt);
        let mut txn = db.begin(&TxnOptions::default()).expect("begin");
        txn.query_text("insert items { id: 1 }", &[], &TxnCallOptions::default())
            .expect("user insert");
        denied(
            txn.query_text(
                &format!("transact\n insert items {{ id: 2 }}\n {INSERT_MAPPING}\n return {{}}"),
                &[],
                &TxnCallOptions::default(),
            ),
            "catalog::peer_mappings",
        );
        txn.query_text("insert items { id: 3 }", &[], &TxnCallOptions::default())
            .expect("continue after denial");
        txn.commit(None).expect("commit unaffected calls");
        assert_eq!(
            vec!["1", "3"],
            api::sorted(api::result(&db, "from i in items select i.id"))
        );
        mappings_empty(&db);
        db.close().expect("close database");
    });
}

#[test]
fn registered_transactions_cannot_hide_a_system_write_in_an_unreached_branch() {
    api::once(0, |rt| {
        let body = format!(
            "import example\ntxn corrupt () -> {{}} =\n transact\n  if false then {INSERT_MAPPING} else skip\n  return {{}}"
        );
        let opts = OpenOptions {
            registrations: vec![Registration::new("bad", body)],
            ..api::options(2)
        };
        let got = Database::open_in(rt.clone(), "bad.adt", &Log::new([SCHEMA]), &opts);
        match got {
            Err(OpenError::RegistrationRefused { bundle, reason }) => {
                assert_eq!("bad", bundle);
                assert!(
                    matches!(reason, Error::Rejected { reason: RejectReason::Denied { ref detail } }
                    if detail == "txn `corrupt`: programs cannot write system relation `catalog::peer_mappings`"),
                    "{reason:?}"
                );
            }
            other => panic!("registration must fail with Denied: {other:?}"),
        }
        let db = open(rt);
        mappings_empty(&db);
        db.close().expect("close database");
    });
}

#[test]
fn native_setting_commands_and_temp_user_writes_remain_legal() {
    api::once(0, |rt| {
        let db = open(rt);
        db.query_text(
            "set setting RetryLimit = { retries: 3 }",
            &[],
            &CallOptions::default(),
        )
        .expect("native settings write");
        assert_eq!(
            vec!["3"],
            api::printed(api::result(
                &db,
                "from s in catalog::settings select match s | RetryLimit r -> r.retries | _ -> 0"
            ))
        );
        let mut txn = db.begin(&TxnOptions::default()).expect("begin");
        txn.query_text(
            "temp relation stage : Item key id\ninsert stage { id: 7 }",
            &[],
            &TxnCallOptions::default(),
        )
        .expect("temp relation insert");
        assert_eq!(
            vec!["7"],
            api::in_txn(&mut txn, "from s in stage select s.id")
        );
        txn.commit(None).expect("commit temp writes");
        db.query_text("insert items { id: 8 }", &[], &CallOptions::default())
            .expect("user insert");
        db.query_text("vacuum items", &[], &CallOptions::default())
            .expect("native maintenance");
        assert_eq!(
            vec!["8"],
            api::printed(api::result(&db, "from i in items select i.id"))
        );
        db.close().expect("close database");
    });
}

fn bundled(name: &str, body: &str) -> Registration {
    let source = adtb_lang::signature::replay([SCHEMA]).expect("replay fixture");
    let schema = source.schema().expect("build fixture schema");
    let hash: String = adtb::catalog::version_hash(&schema, &source)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Registration::bundle(format!(
        "bundle {name} pins 1 ({hash}) edition \"2026-10\" lease 8s\n{body}"
    ))
    .expect("parse bundle")
}

#[test]
fn denied_open_retains_unchecked_headers_without_registering_or_renewing_calls() {
    api::once(0, |rt| {
        let good = "import example\nquery ids () -> Set I64 = from i in items select i.id";
        let original = OpenOptions {
            registrations: vec![bundled("old-1.0", good)],
            ..api::options(2)
        };
        let db = Database::open_in(rt.clone(), "system.adt", &Log::new([SCHEMA]), &original)
            .expect("register original bundle");
        let renewed = api::printed(api::result(
            &db,
            "from b in catalog::bundles where b.name == \"old-1.0\" select b.renewed",
        ));
        db.close().expect("close original file");
        let mut timer = rt.io();
        let ticket = timer.submit(Op::Timer {
            at: rt.now() + Duration::from_secs(2),
        });
        loop {
            if let Some(done) = timer.poll() {
                assert_eq!(ticket, done.ticket);
                assert_eq!(Ok(Done::Fired), done.result);
                break;
            }
            rt.park();
        }
        let body = format!(
            "import example\ntxn corrupt () -> {{}} =\n transact\n  {INSERT_MAPPING}\n  return {{}}"
        );
        let opts = OpenOptions {
            registrations: vec![
                bundled("old-1.0", good),
                bundled("new-1.0", good),
                bundled("bad-1.0", &body),
            ],
            ..OpenOptions::default()
        };
        let refused = Database::open_in(rt.clone(), "system.adt", &Log::new([SCHEMA]), &opts);
        match refused {
            Err(OpenError::RegistrationRefused { bundle, reason }) => {
                assert_eq!("bad-1.0", bundle);
                assert!(
                    matches!(reason, Error::Rejected { reason: RejectReason::Denied { ref detail } }
                    if detail == "txn `corrupt`: programs cannot write system relation `catalog::peer_mappings`"),
                    "{reason:?}"
                );
            }
            other => panic!("expected denied bundle: {other:?}"),
        }
        let db = open(rt);
        mappings_empty(&db);
        assert_eq!(
            vec!["\"bad-1.0\"", "\"new-1.0\"", "\"old-1.0\""],
            api::sorted(api::result(&db, "from b in catalog::bundles select b.name"))
        );
        assert_eq!(
            vec!["1"],
            api::printed(api::result(
                &db,
                "count(r for r in catalog::registered where r.bundle.name == \"old-1.0\")"
            ))
        );
        assert_eq!(
            vec!["0"],
            api::printed(api::result(
                &db,
                "count(r for r in catalog::registered where r.bundle.name == \"new-1.0\" or r.bundle.name == \"bad-1.0\")"
            ))
        );
        assert_eq!(
            renewed,
            api::printed(api::result(
                &db,
                "from b in catalog::bundles where b.name == \"old-1.0\" select b.renewed"
            ))
        );
        db.close().expect("close database");
    });
}

struct SafeBundle;
impl BundleDef for SafeBundle {
    type Error = Error;
    const NAME: &'static str = "safe-1.0";
    const TEXT: &'static str = "";
    const PINS: u64 = 1;
}

struct SystemTarget;
impl adtb::CtorRelation for SystemTarget {
    fn target(&self) -> &str {
        "catalog::peer_mappings"
    }
}

#[test]
fn typed_wrappers_and_arrow_loads_share_the_program_boundary() {
    api::once(0, |rt| {
        let opts = OpenOptions {
            registrations: vec![bundled(
                "safe-1.0",
                "import example\nquery ids () -> Set I64 = from i in items select i.id",
            )],
            ..api::options(2)
        };
        let db = Database::open_in(rt, "typed.adt", &Log::new([SCHEMA]), &opts)
            .expect("open safe bundle");
        let mut load = db.load(&LoadOptions::default()).expect("open load");
        denied(load.appender(SystemTarget), "catalog::peer_mappings");
        {
            let mut appender = load.appender("items").expect("user target");
            let schema = appender.schema().clone();
            let mut builder = adtb::batch::BatchBuilder::new(schema.batch_schema());
            builder.push_row(&[adtb::key::Value::I64(11)]);
            let batch =
                adtb::RecordBatch::new(schema, builder.finish()).expect("batch of user rows");
            appender.push(&batch).expect("stage user row");
        }
        assert_eq!(1, load.commit().expect("commit user load").rows);
        let registered = db.registered(SafeBundle::NAME).expect("bind safe bundle");
        let rows: usize = registered
            .call_arrow(1, &[], &CallOptions::default())
            .expect("query as Arrow")
            .map(|batch| batch.expect("result batch").num_rows())
            .sum();
        assert_eq!(1, rows);
        let typed = db.bundle(&SafeBundle).expect("typed bundle");
        let mut txn = typed
            .begin(&TxnOptions::default())
            .expect("typed transaction");
        denied(
            txn.query_text(INSERT_MAPPING, &[], &TxnCallOptions::default()),
            "catalog::peer_mappings",
        );
        assert_eq!(
            vec!["11"],
            api::printed(
                txn.query_text(
                    "from i in items select i.id",
                    &[],
                    &TxnCallOptions::default()
                )
                .expect("user read")
                .pop()
                .expect("one result")
            )
        );
        txn.commit(None).expect("commit typed transaction");
        mappings_empty(&db);
        db.close().expect("close database");
    });
}

#[test]
fn system_reads_remain_queries_and_volatile_user_writes_still_have_their_own_error() {
    api::once(0, |rt| {
        let db = open(rt);
        db.query_text(
            "from c in sys::calls select c.id",
            &[],
            &CallOptions::default(),
        )
        .expect("system read");
        assert_eq!(
            vec!["\"items\""],
            api::printed(api::result(
                &db,
                "from r in catalog::relations where r.module == \"example\" select r.name"
            ))
        );
        let refused = db.query_text(
            "insert items { id: count(c for c in sys::calls) }",
            &[],
            &CallOptions::default(),
        );
        assert!(
            matches!(refused, Err(Error::Rejected { reason: RejectReason::Invalid { ref detail } })
            if detail == "a write must not depend on a volatile relation: not in its target, its predicate, its values, or a value or condition it depends on"),
            "{refused:?}"
        );
        assert!(api::printed(api::result(&db, "from i in items select i")).is_empty());
        db.close().expect("close database");
    });
}
