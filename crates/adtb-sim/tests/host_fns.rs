//! Host functions in process over the simulator: bodies the program supplies at open, checked
//! against their declarations, called once per batch of rows from queries, writes and
//! constraints.

#[expect(dead_code, reason = "this binary uses part of the API helpers")]
#[path = "support/api.rs"]
mod api;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use adtb::batch::{self, Batch, BatchBuilder, Type};
use adtb::key::{Scalar, Value as Stored};
use adtb::{
    CallOptions, Database, Determinism, Error, FatalReason, Fault, HostError, HostFns, Log,
    OpenError, OpenOptions, Position, RejectReason, TxnCallOptions, TxnOptions,
};
use adtb_sim::{Config, Sim, SimRuntime};

use api::{once, options, printed, result, run};

const NOTES: &str = r#"module shop
add module shop
add type Note = { id: Key notes, body: Text }
add relation notes : Note key id generate sequence
define fn slugify (t: Text) -> Text = host "slugify" version "1.0" deterministic
define fn stamp (t: Text) -> Text = host "stamp" version "1.0"
"#;

const SLUGGED: &str = r"module shop
add constraint slugged =
  from n in notes
  where slugify(n.body) != n.body
  select n
";

const REDEFINE: &str = r#"module shop
redefine fn slugify (t: Text) -> Text = host "slugify" version "2.0" deterministic
"#;

/// Returns `f` over the text of each row of the batch's one column, as a batch of one text
/// column.
fn texts(b: &Batch, f: impl Fn(&str) -> String) -> Batch {
    let mut out = BatchBuilder::new(batch::Schema::new(vec![Type::Scalar(Scalar::Text)]));
    for i in 0..b.len() {
        let Stored::Text(t) = b.value(0, i) else {
            panic!("a text argument");
        };
        out.push_row(&[Stored::Text(f(&t))]);
    }
    out.finish()
}

fn slug(t: &str) -> String {
    t.to_lowercase().replace(' ', "-")
}

/// Returns the host functions with `slugify` at `version` and `det`, counting its calls in
/// `calls`.
fn slugify(version: &str, det: Determinism, calls: &Arc<AtomicUsize>) -> HostFns {
    let mut fns = HostFns::new();
    let calls = calls.clone();
    fns.register("slugify", version, det, move |b: &Batch| {
        calls.fetch_add(1, Ordering::Relaxed);
        Ok(texts(b, slug))
    });
    fns
}

fn with(fns: HostFns) -> OpenOptions {
    OpenOptions {
        host_fns: fns,
        ..options(4)
    }
}

fn open(rt: &SimRuntime, log: &[&str], fns: HostFns) -> Result<Database<SimRuntime>, OpenError> {
    Database::open_in(
        rt.clone(),
        "app.adt",
        &Log::new(log.iter().copied()),
        &with(fns),
    )
}

fn opened(rt: &SimRuntime, log: &[&str], fns: HostFns) -> Database<SimRuntime> {
    open(rt, log, fns).unwrap_or_else(|e| panic!("the database opens: {e:?}"))
}

fn deterministic() -> HostFns {
    slugify("1.0", Determinism::Deterministic, &Arc::default())
}

fn query(db: &Database<SimRuntime>, text: &str) -> Vec<String> {
    let mut out = printed(result(db, text));
    out.sort();
    out
}

fn write(db: &Database<SimRuntime>, text: &str) -> Result<(), Error> {
    db.query_text(text, &[], &CallOptions::default()).map(drop)
}

#[test]
fn a_query_calls_a_host_function_once_per_batch_of_rows() {
    once(0, |rt| {
        let calls = Arc::new(AtomicUsize::new(0));
        let db = opened(
            &rt,
            &[NOTES],
            slugify("1.0", Determinism::Deterministic, &calls),
        );
        write(
            &db,
            r#"insert notes from (from b in ["A B", "C D", "E F"] select { body: b })"#,
        )
        .expect("the notes");
        calls.store(0, Ordering::Relaxed);
        assert_eq!(
            vec![r#""a-b""#, r#""c-d""#, r#""e-f""#],
            query(&db, "from n in notes select slugify(n.body)")
        );
        assert_eq!(
            1,
            calls.load(Ordering::Relaxed),
            "three rows are one batch, one call"
        );
    });
}

#[test]
fn a_write_takes_values_a_host_function_computes() {
    once(1, |rt| {
        let db = opened(&rt, &[NOTES], deterministic());
        write(&db, r#"insert notes { body: slugify("Hello World") }"#).expect("the insert");
        assert_eq!(
            vec![r#""hello-world""#],
            query(&db, "from n in notes select n.body")
        );
    });
}

#[test]
fn a_constraint_reaching_a_host_function_holds_at_every_write() {
    once(2, |rt| {
        let db = opened(&rt, &[NOTES, SLUGGED], deterministic());
        write(&db, r#"insert notes { body: "ok-slug" }"#).expect("a slugged note");
        let err =
            write(&db, r#"insert notes { body: "Not A Slug" }"#).expect_err("the constraint holds");
        assert!(
            matches!(
                &err,
                Error::Rejected {
                    reason: RejectReason::Violated { constraint, .. }
                } if constraint == "shop::slugged"
            ),
            "{err:?}"
        );
    });
}

#[test]
fn a_missing_host_function_a_constraint_reaches_fails_open() {
    once(3, |rt| {
        let got = open(&rt, &[NOTES, SLUGGED], HostFns::new()).map(drop);
        assert!(
            matches!(&got, Err(OpenError::HostFnMissing { name }) if name == "slugify"),
            "{got:?}"
        );
    });
}

#[test]
fn a_missing_host_function_nothing_stored_reaches_fails_only_the_call_that_reaches_it() {
    once(4, |rt| {
        let db = opened(&rt, &[NOTES], deterministic());
        write(&db, r#"insert notes { body: "a" }"#).expect("a note");
        let got = db.query_text(
            "from n in notes select stamp(n.body)",
            &[],
            &CallOptions::default(),
        );
        assert!(
            matches!(
                &got,
                Err(Error::Fatal {
                    reason: FatalReason::MissingFunction { function }
                }) if function == "stamp"
            ),
            "{got:?}"
        );
        assert_eq!(vec![r#""a""#], query(&db, "from n in notes select n.body"));
    });
}

#[test]
fn a_host_function_supplied_at_another_version_fails_open() {
    once(5, |rt| {
        let got = open(
            &rt,
            &[NOTES],
            slugify("2.0", Determinism::Deterministic, &Arc::default()),
        )
        .map(drop);
        assert!(
            matches!(&got, Err(OpenError::HostFnMismatch { name }) if name == "slugify"),
            "{got:?}"
        );
    });
}

#[test]
fn a_host_function_supplied_at_another_determinism_fails_open() {
    once(6, |rt| {
        let got = open(
            &rt,
            &[NOTES],
            slugify("1.0", Determinism::Volatile, &Arc::default()),
        )
        .map(drop);
        assert!(
            matches!(&got, Err(OpenError::HostFnMismatch { name }) if name == "slugify"),
            "{got:?}"
        );
    });
}

#[test]
fn a_shipped_redefinition_lets_open_bind_the_new_version() {
    let mut sim = Sim::new(7, Config::default());
    run(&mut sim, |rt| drop(opened(&rt, &[NOTES], deterministic())));
    run(&mut sim, |rt| {
        let db = opened(
            &rt,
            &[NOTES, REDEFINE],
            slugify("2.0", Determinism::Deterministic, &Arc::default()),
        );
        assert_eq!(
            vec![r#""x-y""#],
            query(&db, r#"from b in ["X Y"] select slugify(b)"#)
        );
    });
    run(&mut sim, |rt| {
        let got = open(&rt, &[NOTES, REDEFINE], deterministic()).map(drop);
        assert!(
            matches!(&got, Err(OpenError::HostFnMismatch { name }) if name == "slugify"),
            "the catalog records 2.0 once the redefinition applied: {got:?}"
        );
    });
}

/// Returns the host functions with `slugify` failing as `body` does.
fn failing(body: impl Fn(&Batch) -> Result<Batch, HostError> + Send + Sync + 'static) -> HostFns {
    let mut fns = HostFns::new();
    fns.register("slugify", "1.0", Determinism::Deterministic, body);
    fns
}

fn faulted(got: Result<Vec<adtb::DynRows>, Error>) -> bool {
    matches!(
        got,
        Err(Error::Rejected {
            reason: RejectReason::Faulted {
                fault: Fault::FunctionFailed
            }
        })
    )
}

#[test]
fn a_host_function_that_errs_panics_or_returns_another_shape_faults_its_statement() {
    let bodies: Vec<(&str, HostFns)> = vec![
        ("an error", failing(|_| Err(HostError("no".into())))),
        ("a panic", failing(|_| panic!("a host function panics"))),
        (
            "another type",
            failing(|b| {
                let mut out =
                    BatchBuilder::new(batch::Schema::new(vec![Type::Scalar(Scalar::I64)]));
                for _ in 0..b.len() {
                    out.push_row(&[Stored::I64(1)]);
                }
                Ok(out.finish())
            }),
        ),
        ("another length", failing(|_| Ok(texts(&empty(), slug)))),
    ];
    for (what, fns) in bodies {
        once(8, |rt| {
            let db = opened(&rt, &[NOTES], fns);
            write(&db, r#"insert notes { body: "a" }"#).expect("a note");
            let got = db.query_text(
                "from n in notes select slugify(n.body)",
                &[],
                &CallOptions::default(),
            );
            assert!(faulted(got), "{what}");
            assert_eq!(
                vec!["Err { error: FunctionFailed }"],
                query(&db, "from n in notes select try slugify(n.body)"),
                "{what}: try catches the fault"
            );
        });
    }
}

fn empty() -> Batch {
    BatchBuilder::new(batch::Schema::new(vec![Type::Scalar(Scalar::Text)])).finish()
}

const SLUG_UNIQUE: &str = "module shop\nadd unique (slugify(body)) in notes\n";

fn is_violated(e: &Error) -> bool {
    matches!(
        e,
        Error::Rejected {
            reason: RejectReason::Violated { .. }
        }
    )
}

#[test]
fn an_index_whose_key_calls_a_host_function_is_built_and_kept() {
    let mut sim = Sim::new(9, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[NOTES], deterministic());
        write(&db, r#"insert notes { body: "A B" }"#).expect("a note");
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[NOTES, SLUG_UNIQUE], deterministic());
        let got = write(&db, r#"insert notes { body: "a b" }"#).expect_err("the unique holds");
        assert!(is_violated(&got), "{got:?}");
        write(&db, r#"insert notes { body: "C D" }"#).expect("another slug");
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[NOTES, SLUG_UNIQUE], deterministic());
        let got = write(&db, r#"insert notes { body: "c d" }"#).expect_err("the unique holds");
        assert!(is_violated(&got), "{got:?}");
    });
}

#[test]
fn an_index_build_over_rows_its_host_function_keys_alike_is_refused() {
    let mut sim = Sim::new(10, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[NOTES], deterministic());
        write(&db, r#"insert notes { body: "A B" }"#).expect("a note");
        write(&db, r#"insert notes { body: "a b" }"#).expect("a note of the same slug");
    });
    run(&mut sim, |rt| {
        match open(&rt, &[NOTES, SLUG_UNIQUE], deterministic()).map(drop) {
            Err(OpenError::MigrationRefused {
                version: 2,
                reason:
                    Error::Rejected {
                        reason: RejectReason::SchemaRefused { .. },
                    },
            }) => {}
            other => panic!("not refused: {other:?}"),
        }
    });
}

#[test]
fn a_migration_validates_a_constraint_reaching_a_host_function_over_existing_rows() {
    let mut sim = Sim::new(11, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[NOTES], deterministic());
        write(&db, r#"insert notes { body: "Not A Slug" }"#).expect("a note");
    });
    run(&mut sim, |rt| {
        match open(&rt, &[NOTES, SLUGGED], deterministic()).map(drop) {
            Err(OpenError::MigrationRefused {
                version: 2,
                reason:
                    Error::Rejected {
                        reason: RejectReason::SchemaRefused { .. },
                    },
            }) => {}
            other => panic!("not refused: {other:?}"),
        }
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[NOTES], deterministic());
        write(&db, "delete n in notes").expect("the delete");
        write(&db, r#"insert notes { body: "a-slug" }"#).expect("a slugged note");
    });
    run(&mut sim, |rt| {
        drop(opened(&rt, &[NOTES, SLUGGED], deterministic()));
    });
}

#[test]
fn a_staged_job_reaching_a_host_function_before_its_redefinition_fails_open() {
    let mut sim = Sim::new(12, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[NOTES], deterministic());
        write(&db, r#"insert notes { body: "a-slug" }"#).expect("a note");
    });
    run(&mut sim, |rt| {
        let got = open(
            &rt,
            &[NOTES, SLUGGED, REDEFINE],
            slugify("2.0", Determinism::Deterministic, &Arc::default()),
        )
        .map(drop);
        assert!(
            matches!(&got, Err(OpenError::HostFnMismatch { name }) if name == "slugify"),
            "{got:?}"
        );
    });
    run(&mut sim, |rt| {
        let got = open(
            &rt,
            &[NOTES, SLUG_UNIQUE, REDEFINE],
            slugify("2.0", Determinism::Deterministic, &Arc::default()),
        )
        .map(drop);
        assert!(
            matches!(&got, Err(OpenError::HostFnMismatch { name }) if name == "slugify"),
            "{got:?}"
        );
    });
    run(&mut sim, |rt| {
        let db = opened(
            &rt,
            &[NOTES, REDEFINE, SLUGGED],
            slugify("2.0", Determinism::Deterministic, &Arc::default()),
        );
        assert_eq!(
            vec![r#""a-slug""#],
            query(&db, "from n in notes select n.body")
        );
    });
}

/// Returns the host functions with `stamp` volatile, its result a number counting its rows.
fn stamping() -> HostFns {
    let mut fns = deterministic();
    let next = Arc::new(AtomicUsize::new(0));
    fns.register("stamp", "1.0", Determinism::Volatile, move |b: &Batch| {
        Ok(texts(b, |_| {
            next.fetch_add(1, Ordering::Relaxed).to_string()
        }))
    });
    fns
}

#[test]
fn a_volatile_host_function_is_evaluated_once_per_call() {
    once(13, |rt| {
        let db = opened(&rt, &[NOTES], stamping());
        assert_eq!(
            vec!["True"],
            query(
                &db,
                r#"from b in ["x", "y"], s in [stamp(b)] select s == s"#
            )
        );
        assert_eq!(
            vec!["True"],
            query(
                &db,
                r#"from b in ["x", "y"], s in [try stamp(b)] select s == s"#
            )
        );
        assert_eq!(
            2,
            query(&db, r#"from b in ["x", "y"], s in [stamp(b)] select s"#).len()
        );
    });
}

const STAMPED: &str = r"module shop
define fn stamped (t: Text) -> Text = stamp(t)
define fn twice (t: Text) -> Bool = stamped(t) == stamped(t)
";

#[test]
fn two_call_sites_of_a_volatile_host_function_with_the_same_text_draw_apart() {
    once(15, |rt| {
        let db = opened(&rt, &[NOTES, STAMPED], stamping());
        write(&db, r#"insert notes { body: "n" }"#).expect("a note");
        let cases = [
            r#"from b in ["x", "y"] select stamp(b) == stamp(b)"#,
            r#"from b in ["x", "y"] select stamped(b) == stamped(b)"#,
            "from n in notes select twice(n.body)",
            r#"from b in ["x", "y"], s in [stamp(b)], t in [stamp(b)] select s == t"#,
            r#"from b in ["x", "y"] select [stamp(b), stamp(b)] == [stamp(b), stamp(b)]"#,
            r#"from b in ["x", "y"] where stamp(b) != stamp(b) select b"#,
        ];
        let expected = [
            vec!["False"],
            vec!["False"],
            vec!["False"],
            vec!["False"],
            vec!["False"],
            vec![r#""x""#, r#""y""#],
        ];
        for (text, want) in cases.iter().zip(expected) {
            assert_eq!(want, query(&db, text), "`{text}`");
        }
    });
}

/// Commits `text` as one transaction. Returns its commit position.
fn commit(db: &Database<SimRuntime>, text: &str) -> Position {
    let mut t = db
        .begin(&TxnOptions::default())
        .expect("a transaction begins");
    t.query_text(text, &[], &TxnCallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"));
    t.commit(None).expect("the transaction commits")
}

#[test]
fn a_host_function_bound_at_open_is_called_inside_an_at_block() {
    once(14, |rt| {
        let db = opened(&rt, &[NOTES], deterministic());
        write(&db, "set setting RetentionWindow = { length: 1h }").expect("the window");
        let then = commit(&db, r#"insert notes { body: "A B" }"#);
        query(&db, "from n in notes select key n");
        commit(&db, r#"update n in notes set { n with body: "C D" }"#);
        query(&db, "from n in notes select key n");
        let at = format!(
            "at commit Position {{ epoch: {}, lsn: {} }} {{ from n in notes select slugify(n.body) }}",
            then.epoch, then.lsn
        );
        assert_eq!(vec![r#""a-b""#], query(&db, &at));
    });
}
