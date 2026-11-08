//! File scans of CSV and NDJSON files, loads from them, and temp relations, through the
//! in-process API of the engine.

use adtb_harness::process::once;
use adtb_simkit::api;

use std::path::{Path, PathBuf};

use adtb::{
    CallOptions, Database, Error, Log, OpenOptions, RejectReason, RetryReason, TxnCallOptions,
    TxnOptions,
};
use adtb_sim::SimRuntime;

use adtb_simkit::open::options;
use api::{failure, in_txn, in_txn_results, printed, result, results, sorted};

const SCHEMA: &str = r"module test
add module test
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
add type Via = Email { addr: Text } | Phone { num: Text }
add type Level = Low | High
add type Point = { x: I32, y: I32 }
add type Wide = { id: I64, name: Text, nick: Option Text, owner: Key users, level: Level,
              pos: Point, via: Via, amount: Decimal 2, ratio: F64 }
add type Small = { id: I64, name: Text }
add type Named = { name: Text }
add type Identity { id: Key identities, user: Key users } = Google { gid: Text } | Apple { sub: Text }
add relation identities : Identity key id generate sequence
add relation outs : Small key id
add type Nick = { id: I64, nick: Option Text }
add type Contact = { id: I64, via: Via }
add type Staged = { line: I64, id: I64, name: Text }
add type Measure = { x: F64 }
add type Pair = { a: I64, b: I64 }
add type Order = { user: Key users, n: I64 }
add relation out_rejects : RejectedRow key (load, line)
";

/// Returns a fresh directory named `name` under the test binary's scratch directory, holding
/// each file of `files` by its relative name.
fn dir(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let at = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("files")
        .join(name);
    if at.exists() {
        std::fs::remove_dir_all(&at).expect("the old directory is removed");
    }
    for (file, text) in files {
        let path = at.join(file);
        std::fs::create_dir_all(path.parent().expect("a file has a directory"))
            .expect("the directory is made");
        std::fs::write(&path, text).expect("the file is written");
    }
    at
}

/// Runs `body` over a fresh database whose file scans read under `root`, holding users 1 `ann`
/// and 2 `bo`.
fn with_db<T>(root: &Path, body: impl FnOnce(&Database<SimRuntime>) -> T) -> T {
    once(0, |rt| {
        let opts = OpenOptions {
            file_root: Some(root.to_path_buf()),
            ..options(2)
        };
        let db = Database::open_in(rt, "files.adt", &Log::new([SCHEMA]), &opts)
            .expect("the database opens");
        results(
            &db,
            r#"insert users { id: users.key(1), name: "ann" }
               insert users { id: users.key(2), name: "bo" }"#,
        );
        body(&db)
    })
}

/// Returns the rows of the one result of `text`, printed and sorted.
fn rows(db: &Database<SimRuntime>, text: &str) -> Vec<String> {
    sorted(result(db, text))
}

/// Returns the file, line and detail of the `BadInput` rejection `e`.
fn bad_input(e: Error) -> (String, i64, String) {
    match e {
        Error::Rejected {
            reason:
                RejectReason::BadInput {
                    source,
                    line,
                    detail,
                },
        } => (source, line, detail),
        other => panic!("not bad input: {other:?}"),
    }
}

fn strings(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|x| (*x).to_owned()).collect()
}

#[test]
fn a_missing_column_is_bad_input_naming_the_file_row_and_column() {
    let root = dir("missing", &[("s.csv", "id\n1\n")]);
    with_db(&root, |db| {
        let (source, line, detail) = bad_input(failure(db, r#"read_csv("s.csv", Small)"#));
        assert_eq!(("s.csv", 1), (source.as_str(), line));
        assert!(detail.contains("`name`"), "{detail}");
    });
}

#[test]
fn a_cell_that_does_not_convert_is_bad_input_naming_its_row_and_column() {
    let root = dir("bad-cell", &[("s.csv", "id,name\n1,ann\nx,bo\n")]);
    with_db(&root, |db| {
        let (source, line, detail) = bad_input(failure(db, r#"read_csv("s.csv", Small)"#));
        assert_eq!(("s.csv", 2), (source.as_str(), line));
        assert!(detail.contains("`id`"), "{detail}");
    });
}

#[test]
fn a_glob_numbers_records_across_its_files_in_name_order() {
    let root = dir(
        "glob-lines",
        &[
            ("logs/a-1.csv", "id,name\n1,ann\n"),
            ("logs/a-2.csv", "id,name\n2,bo\nx,cy\n"),
        ],
    );
    with_db(&root, |db| {
        let (source, line, _) = bad_input(failure(db, r#"read_csv("logs/a-*.csv", Small)"#));
        assert_eq!(("logs/a-2.csv", 3), (source.as_str(), line));
    });
}

#[test]
fn a_file_that_cannot_be_read_is_bad_input_naming_it_at_line_zero() {
    let root = dir("unreadable", &[]);
    with_db(&root, |db| {
        let (source, line, _) = bad_input(failure(db, r#"read_csv("nope.csv", Small)"#));
        assert_eq!(("nope.csv", 0), (source.as_str(), line));
    });
}

#[test]
fn a_scan_whose_path_is_a_parameter_reads_that_file() {
    let root = dir("param", &[("s.csv", "id,name\n1,ann\n")]);
    with_db(&root, |db| {
        let got = db
            .query_text(
                "from r in read_csv(p, Small) select r.name",
                &[(
                    "p",
                    adtb_lang::types::Ty::Base(adtb_lang::types::Base::Text),
                    adtb::Value::text("s.csv"),
                )],
                &CallOptions::default(),
            )
            .expect("the scan runs");
        assert_eq!(
            strings(&[r#""ann""#]),
            printed(got.into_iter().next().expect("a result"))
        );
    });
}

#[test]
fn a_load_without_rejects_fails_with_bad_input_on_a_failed_record() {
    let root = dir("no-rejects", &[("s.csv", "id,name\n1,ann\nx,bo\n")]);
    with_db(&root, |db| {
        let (source, line, _) = bad_input(failure(
            db,
            r#"insert outs from (from r in read_csv("s.csv", Small) select r)"#,
        ));
        assert_eq!(("s.csv", 2), (source.as_str(), line));
        assert_eq!(strings(&["0"]), rows(db, "(count(o for o in outs))"));
    });
}

#[test]
fn a_reject_in_a_load_without_rejects_fails_with_bad_input_naming_its_line() {
    let root = dir("reject-line", &[("s.csv", "id,name\n1,ann\n2,\n")]);
    with_db(&root, |db| {
        let (source, line, detail) = bad_input(failure(
            db,
            r#"insert outs from (
                 from r in read_csv("s.csv", Small)
                 select Small { id: r.id, name: if r.name == "" then reject "no name" else r.name })"#,
        ));
        assert_eq!(
            ("s.csv", 2, "no name"),
            (source.as_str(), line, detail.as_str())
        );
    });
}

#[test]
fn a_line_that_is_not_json_does_not_convert() {
    let root = dir(
        "ndjson-bad",
        &[(
            "s.ndjson",
            "{\"id\": \"1\", \"name\": \"ann\"}\n{\"id\": \n",
        )],
    );
    with_db(&root, |db| {
        let (source, line, _) = bad_input(failure(db, r#"read_ndjson("s.ndjson", Small)"#));
        assert_eq!(("s.ndjson", 2), (source.as_str(), line));
    });
}

#[test]
fn a_file_that_changes_within_a_transaction_aborts_it_with_file_changed() {
    let root = dir("changed", &[("s.csv", "id,name\n1,ann\n")]);
    with_db(&root, |db| {
        let mut t = db
            .begin(&TxnOptions::default())
            .expect("a transaction begins");
        let scan = r#"from r in read_csv("s.csv", Small) select r.name"#;
        assert_eq!(strings(&[r#""ann""#]), in_txn(&mut t, scan));
        assert_eq!(
            strings(&[r#""ann""#]),
            in_txn(&mut t, scan),
            "the same files read again"
        );
        std::fs::write(root.join("s.csv"), "id,name\n1,bo\n").expect("the file is rewritten");
        let e = t
            .query_text(scan, &[], &TxnCallOptions::default())
            .expect_err("a changed file aborts the transaction");
        assert!(
            matches!(
                &e,
                Error::Retry {
                    reason: RetryReason::FileChanged { path }
                } if path == "s.csv"
            ),
            "{e:?}"
        );
        assert!(
            t.commit(None).is_err(),
            "the aborted transaction does not commit"
        );
        assert_eq!(
            strings(&[r#""bo""#]),
            rows(db, scan),
            "a later transaction reads the new file"
        );
    });
}

/// Assert that `e` is a `FileChanged` retry naming `path`.
fn assert_file_changed(e: &Error, path: &str) {
    assert!(
        matches!(
            e,
            Error::Retry {
                reason: RetryReason::FileChanged { path: p }
            } if p == path
        ),
        "{e:?}"
    );
}

#[test]
fn a_file_a_failed_statement_read_stays_pinned_for_its_transaction() {
    let root = dir("pinned-on-failure", &[("s.csv", "id,name\nx,ann\n")]);
    with_db(&root, |db| {
        let mut t = db
            .begin(&TxnOptions::default())
            .expect("a transaction begins");
        let scan = r#"from r in read_csv("s.csv", Small) select r.name"#;
        let e = t
            .query_text(scan, &[], &TxnCallOptions::default())
            .expect_err("a record that does not convert fails the statement");
        bad_input(e);
        std::fs::write(root.join("s.csv"), "id,name\n1,bo\n").expect("the file is rewritten");
        let e = t
            .query_text(scan, &[], &TxnCallOptions::default())
            .expect_err("the file changed since the failed statement read it");
        assert_file_changed(&e, "s.csv");
    });
}

#[test]
fn a_file_an_explain_analyze_read_stays_pinned_for_its_transaction() {
    let root = dir("pinned-on-trial", &[("s.csv", "id,name\n1,ann\n")]);
    with_db(&root, |db| {
        let mut t = db
            .begin(&TxnOptions::default())
            .expect("a transaction begins");
        let scan = r#"from r in read_csv("s.csv", Small) select r.name"#;
        t.query_text(
            &format!("explain analyze {scan}"),
            &[],
            &TxnCallOptions::default(),
        )
        .expect("explain analyze runs the scan");
        std::fs::write(root.join("s.csv"), "id,name\n1,bo\n").expect("the file is rewritten");
        let e = t
            .query_text(scan, &[], &TxnCallOptions::default())
            .expect_err("the file changed since explain analyze read it");
        assert_file_changed(&e, "s.csv");
    });
}

#[test]
fn a_temp_relation_key_is_checked() {
    let root = dir("temp-key", &[]);
    with_db(&root, |db| {
        let e = failure(
            db,
            r#"temp relation stage : Small key id
               insert stage { id: 1, name: "ann" }
               insert stage { id: 1, name: "bo" }"#,
        );
        match e {
            Error::Rejected {
                reason: RejectReason::KeyViolation { relation, key },
            } => assert_eq!(
                ("temp::stage", "stage.key(1)"),
                (relation.as_str(), key.as_str())
            ),
            other => panic!("{other:?}"),
        }
        let e = failure(
            db,
            r"temp relation stage : Small key id
              insert stage from (from u in users select Small { id: 1, name: u.name })",
        );
        assert!(
            matches!(
                e,
                Error::Rejected {
                    reason: RejectReason::KeyViolation { .. }
                }
            ),
            "two rows of one insert sharing a key: {e:?}"
        );
    });
}

#[test]
fn a_temp_relation_belongs_to_its_interactive_transaction() {
    let root = dir("temp-txn", &[]);
    with_db(&root, |db| {
        let mut t = db
            .begin(&TxnOptions::default())
            .expect("a transaction begins");
        in_txn_results(&mut t, "temp relation stage : Small key id");
        in_txn_results(&mut t, r#"insert stage { id: 1, name: "ann" }"#);
        assert_eq!(
            strings(&[r#""ann""#]),
            in_txn(&mut t, "from s in stage select s.name"),
            "a later call reads the rows of an earlier one"
        );
        let e = t
            .query_text(
                "temp relation stage : Small key id",
                &[],
                &TxnCallOptions::default(),
            )
            .expect_err("the transaction has a temp relation of that name");
        assert!(
            matches!(
                e,
                Error::Rejected {
                    reason: RejectReason::Invalid { .. }
                }
            ),
            "{e:?}"
        );
        t.commit(None).expect("the transaction commits");
        let e = failure(db, "(count(s for s in stage))");
        assert!(
            matches!(
                e,
                Error::Rejected {
                    reason: RejectReason::Invalid { .. }
                }
            ),
            "a temp relation is gone when its transaction ends: {e:?}"
        );
    });
}

#[test]
fn a_failed_call_keeps_none_of_its_writes_to_temp_relations() {
    let root = dir("temp-failed", &[]);
    with_db(&root, |db| {
        let mut t = db
            .begin(&TxnOptions::default())
            .expect("a transaction begins");
        in_txn_results(
            &mut t,
            r#"temp relation stage : Small key id
               insert stage { id: 1, name: "ann" }"#,
        );
        let e = t
            .query_text(
                r#"insert stage { id: 2, name: "bo" }
                   insert users { id: users.key(1), name: "dup" }"#,
                &[],
                &TxnCallOptions::default(),
            )
            .expect_err("the second item collides on the key of users");
        assert!(
            matches!(
                e,
                Error::Rejected {
                    reason: RejectReason::KeyViolation { .. }
                }
            ),
            "{e:?}"
        );
        assert_eq!(
            strings(&["1"]),
            in_txn(&mut t, "from s in stage select s.id")
        );
    });
}
