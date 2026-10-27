//! File scans of CSV and NDJSON files, loads from them, and temp relations, through the
//! in-process API of the engine.

#[expect(dead_code, reason = "this binary uses part of the API helpers")]
#[path = "support/api.rs"]
mod api;

use std::path::{Path, PathBuf};

use adtb::{
    CallOptions, Database, Error, Log, OpenOptions, RejectReason, RetryReason, TxnCallOptions,
    TxnOptions,
};
use adtb_sim::SimRuntime;

use api::{in_txn, once, options, printed, result, sorted};

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
        run(
            &db,
            r#"insert users { id: users.key(1), name: "ann" }
               insert users { id: users.key(2), name: "bo" }"#,
        );
        body(&db)
    })
}

fn run(db: &Database<SimRuntime>, text: &str) {
    db.query_text(text, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"));
}

/// Returns the rows of the one result of `text`, printed and sorted.
fn rows(db: &Database<SimRuntime>, text: &str) -> Vec<String> {
    sorted(result(db, text))
}

/// Returns the rows of the last result of `text`, printed in order.
fn last(db: &Database<SimRuntime>, text: &str) -> Vec<String> {
    let mut out = db
        .query_text(text, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"));
    printed(out.pop().expect("the text has a result"))
}

fn failure(db: &Database<SimRuntime>, text: &str) -> Error {
    match db.query_text(text, &[], &CallOptions::default()) {
        Ok(_) => panic!("`{text}` succeeds"),
        Err(e) => e,
    }
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
fn a_csv_scan_reads_columns_by_name_in_any_order_and_ignores_others() {
    let root = dir("by-name", &[("s.csv", "extra,name,id\nx,ann,1\ny,bo,2\n")]);
    with_db(&root, |db| {
        assert_eq!(
            strings(&[r#""ann""#, r#""bo""#]),
            rows(db, r#"from r in read_csv("s.csv", Small) select r.name"#)
        );
        assert_eq!(
            strings(&["1", "2"]),
            rows(db, r#"from r in read_csv("s.csv", Small) select r.id"#)
        );
    });
}

#[test]
fn a_csv_scan_reads_every_type_mapping_of_a_cell() {
    let root = dir(
        "wide",
        &[(
            "w.csv",
            "id,name,nick,owner,level,pos.x,pos.y,via.tag,via.Email.addr,via.Phone.num,amount,ratio\n\
             1,ann,,1,High,3,4,Email,a@b.example,,12.50,0.5\n",
        )],
    );
    with_db(&root, |db| {
        assert_eq!(
            strings(&[
                r#"Wide { id: 1, name: "ann", nick: None, owner: users.key(1), level: High, pos: Point { x: 3, y: 4 }, via: Email { addr: "a@b.example" }, amount: 12.50, ratio: 0.5 }"#
            ]),
            rows(db, r#"read_csv("w.csv", Wide)"#)
        );
    });
}

#[test]
fn an_empty_unquoted_cell_is_none_and_a_quoted_one_is_some_empty_text() {
    let root = dir("nulls", &[("n.csv", "id,nick\n1,\n2,\"\"\n3,cy\n")]);
    with_db(&root, |db| {
        assert_eq!(
            strings(&["None", r#"Some { value: "" }"#, r#"Some { value: "cy" }"#]),
            rows(db, r#"from r in read_csv("n.csv", Nick) select r.nick"#)
        );
    });
}

#[test]
fn a_missing_column_of_an_option_field_reads_none() {
    let root = dir("missing-option", &[("n.csv", "id\n1\n")]);
    with_db(&root, |db| {
        assert_eq!(
            strings(&["None"]),
            rows(db, r#"from r in read_csv("n.csv", Nick) select r.nick"#)
        );
    });
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
fn a_value_level_sum_reads_from_a_text_cell_or_from_tag_columns() {
    let root = dir(
        "sums",
        &[
            ("t.csv", "id,via\n1,\"Phone { num: \"\"5\"\" }\"\n"),
            (
                "c.csv",
                "id,via.tag,via.Email.addr,via.Phone.num\n1,Email,a@b,\n",
            ),
        ],
    );
    with_db(&root, |db| {
        assert_eq!(
            strings(&[r#"Phone { num: "5" }"#]),
            rows(db, r#"from c in read_csv("t.csv", Contact) select c.via"#)
        );
        assert_eq!(
            strings(&[r#"Email { addr: "a@b" }"#]),
            rows(db, r#"from c in read_csv("c.csv", Contact) select c.via"#)
        );
    });
}

#[test]
fn a_sum_row_type_reads_its_tag_header_and_constructor_columns() {
    let root = dir(
        "sum-rows",
        &[(
            "i.csv",
            "tag,id,user,Google.gid,Apple.sub\nGoogle,7,1,g-1,\nApple,8,2,,s-2\n",
        )],
    );
    with_db(&root, |db| {
        assert_eq!(
            strings(&[
                r#"Apple { id: identities.key(8), user: users.key(2), sub: "s-2" }"#,
                r#"Google { id: identities.key(7), user: users.key(1), gid: "g-1" }"#,
            ]),
            rows(db, r#"read_csv("i.csv", Identity)"#)
        );
    });
}

#[test]
fn a_glob_reads_every_file_it_matches_as_one_relation() {
    let root = dir(
        "glob",
        &[
            ("logs/a-1.csv", "id,name\n1,ann\n"),
            ("logs/a-2.csv", "id,name\n2,bo\n"),
            ("logs/b-1.csv", "id,name\n3,cy\n"),
        ],
    );
    with_db(&root, |db| {
        assert_eq!(
            strings(&["1", "2"]),
            rows(
                db,
                r#"from r in read_csv("logs/a-*.csv", Small) select r.id"#
            )
        );
        assert_eq!(
            strings(&["0"]),
            rows(db, r#"(count(r for r in read_csv("logs/z-*.csv", Small)))"#)
        );
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
fn a_file_scan_is_a_set_so_equal_records_are_one_row() {
    let root = dir("set", &[("s.csv", "id,name\n1,ann\n1,ann\n")]);
    with_db(&root, |db| {
        assert_eq!(
            strings(&["1"]),
            rows(db, r#"(count(r for r in read_csv("s.csv", Small)))"#)
        );
    });
}

#[test]
fn a_scan_joins_relations() {
    let root = dir("join", &[("o.csv", "user,n\n1,10\n1,5\n2,7\n")]);
    with_db(&root, |db| {
        assert_eq!(
            strings(&[r#"{ n: 15, name: "ann" }"#, r#"{ n: 7, name: "bo" }"#]),
            rows(
                db,
                r#"from o in read_csv("o.csv", Order), u in users
                   where o.user == key u
                   group o by u.name into g
                   select { name: g.key, n: sum(x.n for x in g) }"#
            )
        );
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
fn a_load_inserts_the_rows_of_a_file() {
    let root = dir("load", &[("s.csv", "id,name\n1,ann\n2,bo\n")]);
    with_db(&root, |db| {
        run(
            db,
            r#"insert outs from (from r in read_csv("s.csv", Small) select r)"#,
        );
        assert_eq!(
            strings(&[r#""ann""#, r#""bo""#]),
            rows(db, "from o in outs select o.name")
        );
    });
}

#[test]
fn a_load_with_rejects_routes_failed_records_to_the_rejects_relation() {
    let root = dir("rejects", &[("s.csv", "id,name\n1,ann\nx,bo\n3,\n4,dee\n")]);
    with_db(&root, |db| {
        run(
            db,
            r#"insert outs from (
                 from r in read_csv("s.csv", Small)
                 select Small { id: r.id, name: if r.name == "" then reject "no name" else r.name })
               rejects into out_rejects"#,
        );
        assert_eq!(strings(&["1", "4"]), rows(db, "from o in outs select o.id"));
        assert_eq!(
            strings(&[
                r#"{ line: 2, raw: "x,bo", source: "s.csv" }"#,
                r#"{ line: 3, raw: "3,", source: "s.csv" }"#,
            ]),
            rows(
                db,
                "from j in out_rejects select { line: j.line, source: j.source, raw: j.raw }"
            )
        );
        assert_eq!(
            strings(&[r#""no name""#]),
            rows(
                db,
                "from j in out_rejects where j.line == 3 select j.reason"
            )
        );
        assert!(
            rows(
                db,
                "from j in out_rejects where j.line == 2 select j.reason"
            )[0]
            .contains("`id`"),
            "the reason of a conversion failure names its column"
        );
        assert_eq!(
            strings(&["1"]),
            rows(
                db,
                "(count(l for l in (from j in out_rejects select j.load)))"
            ),
            "every reject of one statement has the same load"
        );
    });
}

#[test]
fn each_load_with_rejects_has_a_load_of_its_own() {
    let root = dir("loads", &[("s.csv", "id,name\nx,ann\n")]);
    with_db(&root, |db| {
        let load = r#"insert outs from (from r in read_csv("s.csv", Small) select r)
                      rejects into out_rejects"#;
        run(db, load);
        run(db, load);
        assert_eq!(
            strings(&["2"]),
            rows(
                db,
                "(count(l for l in (from j in out_rejects select j.load)))"
            )
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
fn a_constraint_violation_aborts_the_whole_load_and_keeps_no_rejects() {
    let root = dir("violation", &[("s.csv", "id,name\n1,ann\n1,bo\nx,cy\n")]);
    with_db(&root, |db| {
        let e = failure(
            db,
            r#"insert outs from (from r in read_csv("s.csv", Small) select r)
               rejects into out_rejects"#,
        );
        assert!(
            matches!(
                e,
                Error::Rejected {
                    reason: RejectReason::KeyViolation { .. }
                }
            ),
            "{e:?}"
        );
        assert_eq!(strings(&["0"]), rows(db, "(count(j for j in out_rejects))"));
    });
}

#[test]
fn an_ndjson_scan_reads_objects_with_members_in_any_order() {
    let root = dir(
        "ndjson",
        &[(
            "s.ndjson",
            "{\"name\": \"ann\", \"id\": \"1\"}\n\n{\"id\": \"2\", \"name\": \"bo\", \"x\": [1]}\n",
        )],
    );
    with_db(&root, |db| {
        assert_eq!(
            strings(&[r#""ann""#, r#""bo""#]),
            rows(
                db,
                r#"from r in read_ndjson("s.ndjson", Small) select r.name"#
            )
        );
    });
}

#[test]
fn an_option_is_none_by_name_or_by_a_missing_member() {
    let root = dir(
        "ndjson-option",
        &[(
            "n.ndjson",
            "{\"id\": \"1\", \"nick\": \"None\"}\n{\"id\": \"2\"}\n{\"id\": \"3\", \"nick\": {\"Some\": {\"value\": \"\"}}}\n",
        )],
    );
    with_db(&root, |db| {
        assert_eq!(
            strings(&["None", r#"Some { value: "" }"#]),
            rows(
                db,
                r#"from r in read_ndjson("n.ndjson", Nick) select r.nick"#
            )
        );
    });
}

#[test]
fn an_ndjson_sum_row_is_an_object_of_one_member_named_by_its_constructor() {
    let root = dir(
        "ndjson-sum",
        &[(
            "i.ndjson",
            r#"{"Google": {"id": "7", "user": "1", "gid": "g-1"}}"#,
        )],
    );
    with_db(&root, |db| {
        assert_eq!(
            strings(&[r#"Google { id: identities.key(7), user: users.key(1), gid: "g-1" }"#]),
            rows(db, r#"read_ndjson("i.ndjson", Identity)"#)
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

#[test]
fn a_temp_relation_holds_rows_for_the_rest_of_its_text() {
    let root = dir("temp", &[]);
    with_db(&root, |db| {
        assert_eq!(
            strings(&[r#""ann""#, r#""bo""#]),
            last(
                db,
                r#"temp relation stage : Small key id
                   insert stage { id: 1, name: "ann" }
                   insert stage { id: 2, name: "bo" }
                   from s in stage select s.name"#
            )
        );
    });
}

#[test]
fn a_temp_relation_is_gone_when_its_text_ends() {
    let root = dir("temp-gone", &[]);
    with_db(&root, |db| {
        run(
            db,
            r#"temp relation stage : Small key id
               insert stage { id: 1, name: "ann" }"#,
        );
        assert_eq!(
            strings(&["0"]),
            last(
                db,
                r"temp relation stage : Small key id
                  (count(s for s in stage))"
            )
        );
    });
}

#[test]
fn a_temp_relation_without_a_key_clause_is_keyed_by_its_whole_row() {
    let root = dir("temp-row-key", &[]);
    with_db(&root, |db| {
        assert_eq!(
            strings(&["2"]),
            last(
                db,
                r"temp relation pairs : Pair
                  insert pairs { a: 1, b: 1 }
                  insert pairs { a: 1, b: 2 }
                  (count(p for p in pairs))"
            )
        );
        let e = failure(
            db,
            r"temp relation pairs : Pair
              insert pairs { a: 1, b: 2 }
              insert pairs { a: 1, b: 2 }",
        );
        assert!(
            matches!(
                e,
                Error::Rejected {
                    reason: RejectReason::KeyViolation { .. }
                }
            ),
            "inserting a row whose key a kept row holds collides, as for any relation: {e:?}"
        );
    });
}

#[test]
fn a_temp_relation_of_an_anonymous_record_holds_its_rows() {
    let root = dir("temp-anon", &[]);
    with_db(&root, |db| {
        assert_eq!(
            strings(&["3"]),
            last(
                db,
                r"temp relation sums : { a: I64, b: I64 } key a
                  insert sums { a: 1, b: 2 }
                  from s in sums select s.a + s.b"
            )
        );
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
fn a_temp_relation_updates_and_deletes_like_any_relation() {
    let root = dir("temp-writes", &[]);
    with_db(&root, |db| {
        assert_eq!(
            strings(&[r#"Small { id: 2, name: "BO" }"#]),
            last(
                db,
                r#"temp relation stage : Small key id
                   insert stage { id: 1, name: "ann" }
                   insert stage { id: 2, name: "bo" }
                   update s in stage where s.id == 2 set Small { s with name: "BO" }
                   delete s in stage where s.id == 1
                   from s in stage select s"#
            )
        );
    });
}

#[test]
fn a_temp_relation_takes_no_name_a_module_of_the_text_declares() {
    let root = dir("temp-name", &[]);
    with_db(&root, |db| {
        let e = failure(
            db,
            r"temp relation users : User key id
              (count(u for u in users))",
        );
        assert!(
            matches!(
                e,
                Error::Rejected {
                    reason: RejectReason::Invalid { .. }
                }
            ),
            "{e:?}"
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
        in_txn_run(&mut t, "temp relation stage : Small key id");
        in_txn_run(&mut t, r#"insert stage { id: 1, name: "ann" }"#);
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
        in_txn_run(
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

#[test]
fn a_staged_load_lands_in_a_temp_relation_then_moves_with_rejects() {
    let root = dir(
        "staged",
        &[("s.csv", "line,id,name\n1,1,ann\n2,2,\n3,3,cy\n")],
    );
    with_db(&root, |db| {
        run(
            db,
            r#"temp relation stage : Staged key line
               temp relation stage_rejects : RejectedRow key (load, line)
               insert stage from (from r in read_csv("s.csv", Staged) select r)
               delete s in stage where s.line == 3
               insert outs from (
                 from s in stage
                 select Small { id: s.id, name: if s.name == "" then reject "no name" else s.name })
               rejects into stage_rejects
               insert out_rejects from (from j in stage_rejects select j)"#,
        );
        assert_eq!(
            strings(&[r#""ann""#]),
            rows(db, "from o in outs select o.name")
        );
        assert_eq!(
            strings(&[r#"{ line: 2, reason: "no name", source: "stage" }"#]),
            rows(
                db,
                "from j in out_rejects select { line: j.line, source: j.source, reason: j.reason }"
            )
        );
    });
}

fn in_txn_run(t: &mut adtb::AdHocTransaction<'_, SimRuntime>, text: &str) {
    t.query_text(text, &[], &TxnCallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"));
}
