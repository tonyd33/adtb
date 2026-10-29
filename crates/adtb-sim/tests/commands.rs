//! Commands of ad hoc text run in process: explain, explain analyze, vacuum, reindex and set
//! setting.

use adtb_simkit::api;

use adtb::{Database, DynRows, Error, Log, RejectReason, TxnCallOptions, TxnOptions, Value};
use adtb_sim::SimRuntime;

use api::{failure, in_txn, is_invalid, once, options, printed, result, results, sorted};

const SCHEMA: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
add type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text }
  | Google    { gid: Text }
  | Apple     { sub: Text }
add relation identities : Identity key id generate sequence
  unique (user) in Google
  index (user) in Google
  index (user)
";

fn with_db<T>(body: impl FnOnce(&Database<SimRuntime>) -> T) -> T {
    once(0, |rt| {
        let db = Database::open_in(rt, "commands.adt", &Log::new([SCHEMA]), &options(2))
            .expect("the database opens");
        results(
            &db,
            r#"insert users { id: users.key(1), name: "ann" }
               insert users { id: users.key(2), name: "bo" }
               insert identities.EmailPass { id: identities.key(1), user: users.key(1), email: "a@x" }
               insert identities.Google { id: identities.key(2), user: users.key(2), gid: "g-bo" }"#,
        );
        body(&db)
    })
}

/// Returns the `text` and `document` of the one row of an explain result.
fn explained(rows: DynRows) -> (String, String) {
    let got: Vec<Value> = rows.map(|r| r.expect("a row")).collect();
    let [Value::Record(fs)] = got.as_slice() else {
        panic!("explain returns one record: {got:?}")
    };
    match fs.as_ref() {
        [Value::Text(document), Value::Text(text)] => (text.to_string(), document.to_string()),
        other => panic!("an explain row is {{ document: Text, text: Text }}: {other:?}"),
    }
}

fn names(db: &Database<SimRuntime>) -> Vec<String> {
    sorted(result(db, "from u in users select u.name"))
}

#[test]
fn explain_returns_the_plan_of_a_query_as_text_and_a_document() {
    with_db(|db| {
        let mut out = results(db, "explain from u in users select u.name");
        assert_eq!(1, out.len(), "explain returns one result");
        let (text, document) = explained(out.remove(0));
        assert!(text.contains("Scan users.User"), "{text}");
        assert!(document.starts_with('{'), "{document}");
        for member in [
            "\"format\": 1",
            "\"release\": ",
            "\"root\": ",
            "\"kind\": ",
            "\"estimate\": ",
            "\"children\": ",
        ] {
            assert!(document.contains(member), "{member} in {document}");
        }
    });
}

#[test]
fn explain_of_an_index_probe_routes_by_constructor() {
    with_db(|db| {
        let (text, document) = explained(
            results(
                db,
                r"explain from i in identities
                      where i.user == users.key(2)
                      select match i
                        | EmailPass e -> e.email
                        | Google g    -> g.gid
                        | Apple a     -> a.sub",
            )
            .remove(0),
        );
        assert!(text.contains("Probe identities (user)"), "{text}");
        for name in ["EmailPass", "Google", "Apple"] {
            assert!(text.contains(name), "{name} in {text}");
            assert!(
                document.contains(&format!("\"name\": \"{name}\"")),
                "{name} in {document}"
            );
        }
        assert!(document.contains("\"arms\": "), "{document}");
    });
}

#[test]
fn explain_of_a_read_of_some_constructors_names_the_pruned_ones() {
    with_db(|db| {
        let (_, document) = explained(
            results(
                db,
                "explain from g in identities.Google where g.user == users.key(2) select g.gid",
            )
            .remove(0),
        );
        assert!(
            document.contains("\"pruned\": [{"),
            "a read of one constructor of a sum names the others pruned: {document}"
        );
        assert!(document.contains("\"stage\": \"compile\""), "{document}");
    });
}

#[test]
fn explain_of_a_write_does_not_run_it() {
    with_db(|db| {
        let (text, _) = explained(results(db, r#"explain insert users { name: "cy" }"#).remove(0));
        assert!(text.contains("Insert users"), "{text}");
        assert_eq!(vec![r#""ann""#, r#""bo""#], names(db));
    });
}

#[test]
fn explain_of_a_read_of_a_temp_relation_plans_it() {
    with_db(|db| {
        let (text, _) = explained(
            results(
                db,
                r"temp relation stage : User key id
                  explain from s in stage select s.name",
            )
            .remove(0),
        );
        assert!(text.contains("Unnest"), "{text}");
    });
}

#[test]
fn explain_of_a_registered_call_names_a_registered_bundle() {
    with_db(|db| {
        let e = failure(db, "explain web-2::names()");
        assert!(is_invalid(&e), "no bundle is registered: {e:?}");
    });
}

#[test]
fn explain_results_take_their_place_among_the_items() {
    with_db(|db| {
        let out = results(
            db,
            r"(count(u for u in users))
              explain from u in users select u.name
              (count(i for i in identities))",
        );
        assert_eq!(3, out.len());
        let shown: Vec<Vec<String>> = out.into_iter().map(printed).collect();
        assert_eq!(vec!["2".to_owned()], shown[0]);
        assert!(shown[1][0].contains("Scan users.User"), "{:?}", shown[1]);
        assert_eq!(vec!["2".to_owned()], shown[2]);
    });
}

#[test]
fn explain_analyze_runs_a_query_and_adds_its_actual_rows() {
    with_db(|db| {
        let (text, document) =
            explained(results(db, "explain analyze from u in users select u.name").remove(0));
        assert!(text.contains("actual rows 2"), "{text}");
        assert!(document.contains("\"actual\": {\"rows\": 2"), "{document}");
        for member in ["\"loops\": ", "\"time_us\": ", "\"storage\": "] {
            assert!(document.contains(member), "{member} in {document}");
        }
    });
}

#[test]
fn explain_analyze_of_a_write_reports_its_change_set_and_applies_nothing() {
    with_db(|db| {
        let (text, document) = explained(
            results(
                db,
                r#"explain analyze update g in identities.Google where g.gid == "g-bo"
                   set Google { g with gid: "g-2" }"#,
            )
            .remove(0),
        );
        assert!(text.contains("identities.Google: -1 +1"), "{text}");
        assert!(text.contains("constraints checked"), "{text}");
        assert!(
            document
                .contains("\"change\": {\"identities.Google\": {\"deleted\": 1, \"inserted\": 1}}"),
            "{document}"
        );
        assert_eq!(
            vec![r#""g-bo""#.to_owned()],
            printed(result(db, "from g in identities.Google select g.gid"))
        );
    });
}

#[test]
fn explain_analyze_of_a_write_to_a_temp_relation_reports_its_change() {
    with_db(|db| {
        let got = results(
            db,
            "temp relation t : { a: I64 } key a\nexplain analyze insert t { a: 1 }",
        );
        let (text, document) = explained(got.into_iter().next().expect("a result"));
        assert!(text.contains("t.t: -0 +1"), "{text}");
        assert!(
            document.contains("\"change\": {\"t.t\": {\"deleted\": 0, \"inserted\": 1}}"),
            "{document}"
        );
    });
}

#[test]
fn explain_analyze_of_a_write_counts_no_actual_rows() {
    with_db(|db| {
        let (text, document) =
            explained(results(db, r#"explain analyze insert users { name: "cy" }"#).remove(0));
        assert!(text.contains("actual rows 0"), "{text}");
        assert!(document.contains("\"actual\": {\"rows\": 0"), "{document}");
    });
}

#[test]
fn explain_analyze_of_a_write_checks_its_constraints() {
    with_db(|db| {
        let e = failure(
            db,
            "explain analyze insert identities.Google { user: users.key(2), gid: \"again\" }",
        );
        assert!(
            matches!(
                e,
                Error::Rejected {
                    reason: RejectReason::Violated { .. } | RejectReason::KeyViolation { .. }
                }
            ),
            "{e:?}"
        );
    });
}

#[test]
fn explain_analyze_in_a_transaction_sees_its_writes_and_leaves_no_trace() {
    with_db(|db| {
        let mut t = db
            .begin(&TxnOptions::default())
            .expect("a transaction begins");
        t.query_text(
            r#"insert users { name: "cy" }"#,
            &[],
            &TxnCallOptions::default(),
        )
        .expect("the insert runs");
        let got = t
            .query_text(
                r#"explain analyze delete u in users where u.name == "cy""#,
                &[],
                &TxnCallOptions::default(),
            )
            .expect("explain analyze runs");
        let (text, _) = explained(got.into_iter().next().expect("a result"));
        assert!(text.contains("users.User: -1 +0"), "{text}");
        assert_eq!(
            vec!["3".to_owned()],
            in_txn(&mut t, "(count(u for u in users))"),
            "the explained delete is undone"
        );
        t.commit(None).expect("the transaction commits");
        assert_eq!(vec![r#""ann""#, r#""bo""#, r#""cy""#], names(db));
    });
}

#[test]
fn explain_analyze_of_a_write_needs_a_transaction_that_writes() {
    with_db(|db| {
        let mut t = db
            .begin(&TxnOptions {
                read_only: true,
                ..TxnOptions::default()
            })
            .expect("a transaction begins");
        let e = t
            .query_text(
                r#"explain analyze insert users { name: "cy" }"#,
                &[],
                &TxnCallOptions::default(),
            )
            .expect_err("a read-only transaction does not write");
        assert!(is_invalid(&e), "{e:?}");
    });
}

#[test]
fn vacuum_returns_one_row_per_constructor_relation() {
    with_db(|db| {
        results(db, "delete i in identities where value(key i) == 1");
        let got = printed(results(db, "vacuum identities").remove(0));
        assert_eq!(3, got.len(), "{got:?}");
        assert!(
            got.iter()
                .all(|r| r.contains("ctor: ") && r.contains("removed: ")),
            "{got:?}"
        );
        assert_eq!(
            vec!["2".to_owned()],
            printed(result(db, "from i in identities select value(key i)")),
            "vacuum removes no live row"
        );
    });
}

#[test]
fn vacuum_removes_versions_no_snapshot_sees() {
    with_db(|db| {
        results(db, "delete i in identities where value(key i) == 1");
        let got = printed(results(db, "vacuum identities").remove(0));
        assert!(
            got.iter().any(|r| !r.contains("removed: 0")),
            "the deleted EmailPass version is removed: {got:?}"
        );
    });
}

#[test]
#[ignore = "the engine does not rebuild indexes yet"]
fn reindex_rebuilds_every_index_of_a_relation_as_a_job() {
    with_db(|db| {
        let got = printed(results(db, "reindex identities").remove(0));
        assert_eq!(1, got.len(), "{got:?}");
        assert!(got[0].starts_with("{ job: "), "{got:?}");
    });
}

#[test]
#[ignore = "the engine does not rebuild indexes yet"]
fn reindex_of_one_index_clause_rebuilds_it_as_a_job() {
    with_db(|db| {
        let got =
            printed(results(db, "reindex identities index (user) in Google offline").remove(0));
        assert_eq!(1, got.len(), "{got:?}");
        assert!(got[0].starts_with("{ job: "), "{got:?}");
        assert_eq!(
            vec![r#""g-bo""#.to_owned()],
            printed(result(
                db,
                "from g in identities.Google where g.user == users.key(2) select g.gid"
            )),
            "the rebuilt index finds the row"
        );
    });
}

#[test]
fn reindex_of_an_undeclared_index_is_invalid() {
    with_db(|db| {
        let e = failure(db, "reindex identities index (gid) in Google");
        assert!(is_invalid(&e), "{e:?}");
    });
}

#[test]
fn vacuum_of_a_temp_relation_is_invalid_and_names_it() {
    with_db(|db| {
        let e = failure(db, "temp relation t : User key id\nvacuum t");
        assert!(is_invalid(&e), "{e:?}");
        let shown = format!("{e:?}");
        assert!(shown.contains("`t`") && shown.contains("temp"), "{shown}");
    });
}

#[test]
fn reindex_of_a_temp_relation_is_invalid_and_names_it() {
    with_db(|db| {
        let e = failure(db, "temp relation t : User key id\nreindex t");
        assert!(is_invalid(&e), "{e:?}");
        let shown = format!("{e:?}");
        assert!(shown.contains("`t`") && shown.contains("temp"), "{shown}");
    });
}

#[test]
fn vacuum_of_a_temp_relation_an_earlier_statement_declared_is_invalid() {
    with_db(|db| {
        let mut t = db
            .begin(&TxnOptions::default())
            .expect("a transaction begins");
        t.query_text(
            "temp relation t : User key id",
            &[],
            &TxnCallOptions::default(),
        )
        .expect("the temp relation is declared");
        let Err(e) = t.query_text("vacuum t", &[], &TxnCallOptions::default()) else {
            panic!("`vacuum t` succeeds")
        };
        assert!(is_invalid(&e), "{e:?}");
    });
}

#[test]
fn set_setting_replaces_the_row_of_its_constructor() {
    with_db(|db| {
        let got = printed(results(db, "set setting RetryLimit = { retries: 3 }").remove(0));
        assert_eq!(1, got.len(), "{got:?}");
        assert!(got[0].starts_with("RetryLimit {"), "{got:?}");
        assert!(got[0].contains("retries: 3"), "{got:?}");
        results(db, "set setting RetryLimit = { retries: 7 }");
        assert_eq!(
            vec!["7".to_owned()],
            printed(result(
                db,
                "from s in catalog::settings select match s
                   | RetryLimit r -> r.retries
                   | _ -> 0"
            ))
            .into_iter()
            .filter(|r| r != "0")
            .collect::<Vec<_>>()
        );
    });
}

#[test]
fn set_setting_holds_whatever_the_transaction_does() {
    with_db(|db| {
        let mut t = db
            .begin(&TxnOptions::default())
            .expect("a transaction begins");
        t.query_text(
            "set setting ConflictLog = { rows: 5 }",
            &[],
            &TxnCallOptions::default(),
        )
        .expect("the command runs");
        t.rollback().expect("the transaction rolls back");
        assert_eq!(
            vec!["1".to_owned()],
            printed(result(db, "(count(s for s in catalog::settings))")),
            "a command is not undone by an abort"
        );
    });
}

#[test]
fn a_command_the_engine_does_not_run_is_refused_whole() {
    with_db(|db| {
        for text in ["compact", "verify", "print schema", "finalize upgrade"] {
            let e = failure(db, text);
            assert!(is_invalid(&e), "`{text}`: {e:?}");
        }
    });
}
