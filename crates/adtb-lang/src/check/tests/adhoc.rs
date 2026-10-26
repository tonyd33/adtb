use super::*;

#[test]
fn deployment_grants_and_revokes_check() {
    adhoc_accepts(&[], "grant adhoc to analyst");
    adhoc_accepts(&[], "grant monitor, cancel to oncall");
    adhoc_accepts(&[], "grant schema to deployer with grant option");
    adhoc_accepts(&[], "grant call web-2.4 to web");
    adhoc_accepts(&[], "grant call web-2.4::login_key to support");
    adhoc_accepts(&[], "grant call web::login_key to app");
    adhoc_accepts(&[], "revoke adhoc from analyst");
    adhoc_accepts(&[], "revoke call web-2.4 from web");
}

#[test]
fn operator_grants_are_refused_in_adhoc_text() {
    adhoc_refuses(
        &[],
        "grant read identities.EmailPass (user, email) to support",
        "schema operator",
    );
    adhoc_refuses(
        &[],
        "grant insert identities.Google to web_owner",
        "schema operator",
    );
    adhoc_refuses(
        &[],
        "grant update users (name) to web_owner",
        "schema operator",
    );
    adhoc_refuses(&[], "grant delete posts to web_owner", "schema operator");
    adhoc_refuses(
        &[],
        "grant retag identities.Google -> identities.Apple to web_owner",
        "schema operator",
    );
    adhoc_refuses(&[], "grant call fn display_name to web", "schema operator");
    adhoc_refuses(
        &[],
        "revoke insert identities.Google from web_owner",
        "schema operator",
    );
    adhoc_refuses(&[], "grant adhoc, delete posts to web", "schema operator");
}

#[test]
fn grant_targets_resolve() {
    adhoc_refuses(&[], "grant read nosuch to support", "no `nosuch` in scope");
    adhoc_refuses(
        &[],
        "grant read display_name to support",
        "is not a relation",
    );
    adhoc_refuses(
        &[],
        "grant read identities.Nope to support",
        "neither a constructor",
    );
    adhoc_refuses(&[], "grant call fn nosuch to web", "no `nosuch` in scope");
    adhoc_refuses(&[], "grant call fn users to web", "is not a function");
    adhoc_refuses(
        &[],
        "grant read catalog::nosuch to oncall",
        "module `catalog` declares no `nosuch`",
    );
    adhoc_refuses(
        &[],
        "grant read nosuch::plans to oncall",
        "no module `nosuch`",
    );
}

#[test]
fn grants_on_engine_relations_are_deployment_state() {
    adhoc_accepts(&[], "grant read catalog::plans to oncall");
    adhoc_accepts(&[], "grant read catalog::constructors to analyst");
    adhoc_accepts(&[], "revoke read sys::sessions from analyst");
    adhoc_accepts(&[], "grant read catalog::bundles.(Bundle) to analyst");
}

#[test]
fn role_statements_check() {
    adhoc_accepts(&[], "create role web");
    adhoc_accepts(&[], "drop role web");
    adhoc_accepts(&[("p", "Text")], "alter role web password p");
    adhoc_refuses(
        &[],
        "alter role web password \"hunter2\"",
        "the password is a parameter",
    );
    adhoc_refuses(
        &[("p", "I64")],
        "alter role web password p",
        "expected Text",
    );
    adhoc_refuses(&[], "alter role web password p", "supplies no value");
}

#[test]
fn peer_statements_check() {
    adhoc_accepts(&[], "map peer \"CN=*.web.internal\" on tls_main to web");
    adhoc_accepts(&[], "unmap peer 1 on tls_main");
}

#[test]
fn temp_relations_check() {
    adhoc_accepts(
        &[],
        "temp relation raw : { kind: Text, user: I64, email: Text }",
    );
    adhoc_accepts(&[], "temp relation rejects : RejectedRow key (load, line)");
    adhoc_accepts(&[], "temp relation copies : User key id");
    adhoc_accepts(&[], "temp relation ids : Identity key id");
    adhoc_refuses(&[], "temp relation r : Nope", "no type `Nope`");
    adhoc_refuses(&[], "temp relation r : { a: I64 } key b", "key field `b`");
    adhoc_refuses(
        &[],
        "temp relation r : Identity key email",
        "key field `email`",
    );
    adhoc_refuses(
        &[],
        "temp relation r : { a: I64 } key (a, a)",
        "named twice",
    );
    adhoc_refuses(&[], "temp relation r : I64", "a temp relation holds");
    adhoc_refuses(&[], "temp relation r : Set User", "a temp relation holds");
    adhoc_refuses(
        &[],
        "temp relation r : { a: I64 } key a generate sequence",
        "a generated key is one field of type `Key` of its own relation",
    );
    adhoc_refuses(
        &[],
        "temp relation r : { a: I64 } key a generate nope",
        "not a key generator",
    );
    adhoc_refuses(
        &[],
        "temp relation r : { a: I64 }\ntemp relation r : { b: I64 }",
        "declared twice",
    );
}

#[test]
fn commands_naming_relations_check() {
    adhoc_accepts(&[], "vacuum users");
    adhoc_accepts(&[], "verify");
    adhoc_accepts(&[], "verify identities");
    adhoc_accepts(&[], "reindex users");
    adhoc_accepts(&[], "reindex identities index (email) in EmailPass online");
    adhoc_refuses(&[], "vacuum nosuch", "no `nosuch` in scope");
    adhoc_refuses(&[], "verify display_name", "is not a relation");
    adhoc_refuses(&[], "reindex nosuch", "no `nosuch` in scope");
    adhoc_refuses(
        &[],
        "reindex identities index (email) in Nope",
        "neither a constructor",
    );
    adhoc_refuses(
        &[],
        "reindex identities index (nosuch) in EmailPass",
        "no `nosuch`",
    );
}

#[test]
fn commands_with_expressions_check() {
    adhoc_accepts(
        &[],
        "export (from i in identities.Google select i) to \"google.parquet\"",
    );
    adhoc_accepts(
        &[],
        "export (from u in users select u) to \"users.csv\" sums text",
    );
    adhoc_refuses(
        &[],
        "export (from u in nosuch select u) to \"x\"",
        "`nosuch` is a parameter of this text, but the call supplies no value",
    );
    adhoc_refuses(&[], "export 1 to \"x\"", "`export` takes a set");
    adhoc_accepts(&[], "archive audit through 991204 to \"audit.arrow\"");
    adhoc_refuses(
        &[],
        "archive audit through \"x\" to \"audit.arrow\"",
        "expected I64",
    );
    adhoc_accepts(&[("k", "Text")], "dump to \"d\" encrypt k");
    adhoc_accepts(&[], "dump to \"d\"");
    adhoc_refuses(&[], "dump to \"d\" encrypt k", "supplies no value");
    adhoc_accepts(&[], "restore from \"d\" to \"f\" before version 12");
    adhoc_accepts(
        &[("t", "Instant")],
        "restore from \"d\" to \"f\" at instant t",
    );
    adhoc_accepts(
        &[("p", "Position")],
        "restore from \"d\" to \"f\" at position p",
    );
    adhoc_refuses(
        &[("t", "I64")],
        "restore from \"d\" to \"f\" at instant t",
        "expected Instant",
    );
    adhoc_refuses(
        &[("t", "Instant")],
        "restore from \"d\" to \"f\" at position t",
        "expected Position",
    );
}

#[test]
fn explain_checks_its_statement() {
    adhoc_accepts(&[], "explain from u in users select u");
    adhoc_accepts(&[], "explain analyze insert users { name: \"a\" }");
    adhoc_accepts(&[], "explain transact insert users { name: \"a\" }");
    adhoc_accepts(&[], "explain web-2.4::login_key(1)");
    adhoc_refuses(
        &[],
        "explain from u in nosuch select u",
        "`nosuch` is a parameter of this text, but the call supplies no value",
    );
    adhoc_refuses(&[], "explain insert users { nope: 1 }", "nope");
}

#[test]
fn commands_with_nothing_to_resolve_check() {
    adhoc_accepts(&[], "compact");
    adhoc_accepts(&[], "finalize upgrade");
    adhoc_accepts(&[], "print schema");
    adhoc_accepts(&[], "rotate key dek");
    adhoc_accepts(&[], "rotate key kek \"kms://k\"");
    adhoc_accepts(&[], "destroy key dek 3");
    adhoc_accepts(&[], "backup to \"b\" incremental");
    adhoc_accepts(&[], "register bundle \"web-2.4.adt\"");
    adhoc_accepts(&[], "apply migrations \"0013.adt\" \"0014.adt\"");
    adhoc_accepts(&[], "retire bundle web-2.4");
    adhoc_accepts(&[], "supersede bundle web-2.3 by web-2.4");
}

#[test]
fn commands_over_the_catalog_take_its_keys() {
    adhoc_accepts(&[], "pin plan catalog::plans.key(8812)");
    adhoc_accepts(&[("p", "Key catalog::plans")], "unpin plan p");
    adhoc_accepts(&[], "cancel call sys::calls.key(812)");
    adhoc_accepts(&[("s", "Key sys::sessions")], "terminate session s");
    adhoc_refuses(&[], "pin plan users.key(1)", "expected Key catalog::plans");
    adhoc_refuses(
        &[("c", "Key sys::sessions")],
        "cancel call c",
        "expected Key sys::calls",
    );
    adhoc_refuses(
        &[("s", "I64")],
        "terminate session s",
        "expected Key sys::sessions",
    );
}

#[test]
fn set_setting_takes_the_payload_of_its_constructor() {
    adhoc_accepts(&[], "set setting StatementLimit = { limit: None }");
    adhoc_accepts(
        &[],
        "set setting StatementLimit = { limit: Some { value: 5s } }",
    );
    adhoc_accepts(&[], "set setting VacuumBudget = { io: 0.2, cpu: 0.5 }");
    adhoc_refuses(
        &[],
        "set setting Nope = { limit: None }",
        "`Nope` is not a constructor of `catalog::Setting`",
    );
    adhoc_refuses(&[], "set setting VacuumBudget = { io: 0.2 }", "cpu");
    adhoc_refuses(
        &[],
        "set setting ConflictLog = { rows: \"many\" }",
        "expected I64",
    );
}

#[test]
fn engine_names_are_reached_only_qualified() {
    adhoc_accepts(&[], "from c in catalog::constructors select c.name");
    adhoc_accepts(&[], "from s in sys::sessions select s.principal");
    adhoc_refuses(&[], "from c in constructors select c.name", "constructors");
    adhoc_refuses(&[], "from w in wait_samples select w.at", "wait_samples");
}

#[test]
fn engine_constructors_resolve_by_expected_type() {
    adhoc_accepts(
        &[],
        "from s in catalog::ctor_states where s.stage == Enabled select s.ctor",
    );
    adhoc_accepts(
        &[],
        "from s in catalog::ctor_states where s.stage == catalog::CtorStage.Enabled select s.ctor",
    );
}

#[test]
fn adhoc_items_without_terms_leave_the_others_in_order() {
    let (_, a) = adhoc(
        &[],
        "temp relation raw : { a: I64 }\nfrom u in users select u.name\ncompact\nfrom p in posts select p.title",
    )
    .unwrap();
    assert_eq!(2, a.items.len());
}

#[test]
fn a_temp_relation_is_in_scope_after_its_declaration() {
    adhoc_accepts(
        &[],
        "temp relation raw : { kind: Text, user: I64 }\nfrom r in raw select r.kind",
    );
    adhoc_accepts(
        &[],
        "temp relation stage : RejectedRow key (load, line)
insert stage from (from t in read_csv(\"x.csv\", RejectedRow) select t)
update s in stage where s.line == 3 set RejectedRow { s with reason: \"fixed\" }
delete s in stage where s.line == 4
from s in stage select s.raw",
    );
    adhoc_accepts(
        &[],
        "temp relation raw : { kind: Text, n: I64 }
insert raw from (from u in users select { kind: u.name, n: 1 })
from r in raw where r.n > 0 select r.kind",
    );
    adhoc_accepts(
        &[],
        "temp relation copies : User key id\nfrom c in copies select c.name",
    );
}

#[test]
fn a_name_before_its_temp_declaration_is_a_parameter() {
    adhoc_refuses(
        &[],
        "from r in raw select r\ntemp relation raw : { a: I64 }",
        "`raw` is a parameter of this text",
    );
}

#[test]
fn a_temp_relation_takes_a_name_no_relation_variable_has() {
    adhoc_refuses(
        &[],
        "temp relation users : { a: I64 }",
        "a temp relation takes a name no module the text reads declares",
    );
    adhoc_refuses(
        &[],
        "temp relation display_name : { a: I64 }",
        "module `auth` declares `display_name`",
    );
}

#[test]
fn a_temp_relation_of_a_sum_of_several_constructors_declares_its_key() {
    adhoc_refuses(
        &[],
        "temp relation ids : Identity\nfrom i in ids select i",
        "a temp relation of a sum of several constructors declares its key",
    );
    adhoc_accepts(
        &[],
        "temp relation ids : Identity key id\nfrom i in ids select i",
    );
}

#[test]
fn file_scans_read_at_the_declared_row_type() {
    adhoc_accepts(&[], "from r in read_csv(\"users.csv\", User) select r.name");
    adhoc_accepts(
        &[],
        "from r in read_parquet(\"logs/*.parquet\", User) select r.name",
    );
    adhoc_accepts(
        &[],
        "from r in read_ndjson(\"u.ndjson\", Identity) select r",
    );
    adhoc_accepts(&[], "from r in read_arrow(\"u.arrow\", User) select r");
    adhoc_accepts(
        &[],
        "from r in read_csv(\"users-old.csv\", User) pins 3 (c41ec41ec41ec41ec41ec41ec41ec41ec41ec41ec41ec41ec41ec41ec41ec41e) select r.name",
    );
    adhoc_refuses(&[], "read_csv(\"x.csv\", Nope)", "no type `Nope`");
    adhoc_refuses(
        &[],
        "read_csv(\"x.csv\", Text)",
        "a declared record or sum type",
    );
    adhoc_refuses(
        &[],
        "read_csv(\"x.csv\", Option)",
        "a declared record or sum type",
    );
    adhoc_refuses(&[], "read_csv(3, User)", "Text");
    adhoc_refuses(&[], "read_csv(\"x.csv\")", "takes 2 arguments");
    adhoc_refuses(
        &[],
        "display_name(i) pins 3 (c41ec41ec41ec41ec41ec41ec41ec41ec41ec41ec41ec41ec41ec41ec41ec41e)",
        "only a file scan takes `pins`",
    );
}

#[test]
fn a_file_scan_is_the_source_of_rejects() {
    adhoc_accepts(
        &[],
        "temp relation rejects : RejectedRow key (load, line)
insert users from (from r in read_csv(\"u.csv\", User) select User { id: r.id, name: r.name })
rejects into rejects",
    );
}

fn long_list(n: i64, row: impl Fn(i64) -> String) -> String {
    let rows: Vec<String> = (1..=n).map(row).collect();
    format!("[{}]", rows.join(", "))
}

fn normalises(text: &str) {
    let (s, a) = adhoc(&[], text).unwrap_or_else(|e| panic!("does not check: {e:?}"));
    let p = check_schema(&s).expect("the schema checks");
    crate::norm::adhoc(&s, &p, &a).unwrap_or_else(|e| panic!("has no normal form: {e:?}"));
}

#[test]
fn a_long_list_literal_parses_checks_and_normalises() {
    let rows = long_list(10_000, |i| format!("Raw {{ id: raws.key({i}), n: {i} }}"));
    normalises(&format!("insert raws from {rows}"));
    let records = long_list(10_000, |i| format!("{{ n: {i}, m: {i} + 1 }}"));
    normalises(&format!("from r in {records} select r.n"));
}

/// Returns texts of every nesting form, each `n` levels deep.
fn nested_texts(n: usize) -> Vec<(&'static str, String)> {
    vec![
        (
            "parentheses",
            format!("{}1{}", "(".repeat(n), ")".repeat(n)),
        ),
        ("sets", format!("{}1{}", "[".repeat(n), "]".repeat(n))),
        ("a sum chain", vec!["1"; n].join(" + ")),
        ("an or chain", vec!["true"; n].join(" or ")),
        ("negations", format!("{}true", "not ".repeat(n))),
        (
            "conditionals",
            format!("{}1", "if true then 1 else ".repeat(n)),
        ),
        (
            "records",
            format!("{}1{}", "{ a: ".repeat(n), " }".repeat(n)),
        ),
        (
            "generators",
            format!(
                "from {} select 1",
                (0..n)
                    .map(|i| format!("x{i} in raws"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        (
            "where clauses",
            format!("from r in raws {}select r.n", "where true ".repeat(n - 2)),
        ),
        (
            "a field chain",
            format!(
                "{}1{}{}",
                "{ a: ".repeat(n / 2),
                " }".repeat(n / 2),
                ".a".repeat(n / 2)
            ),
        ),
    ]
}

#[test]
fn nesting_past_the_limit_is_refused() {
    let n = crate::parse::MAX_DEPTH as usize + 1;
    for (form, text) in nested_texts(n) {
        let msg = match parse_adhoc(&text, Options::default()).ok() {
            Ok(_) => panic!("{form}: {n} levels parse"),
            Err(ds) => ds[0].message.clone(),
        };
        assert!(
            msg.starts_with("the text nests more than 64 levels deep"),
            "{form}: {msg}"
        );
    }
}

#[test]
fn nesting_within_the_limit_checks_and_normalises() {
    let n = crate::parse::MAX_DEPTH as usize - 2;
    for (form, text) in nested_texts(n) {
        let (s, a) = adhoc(&[], &text).unwrap_or_else(|e| panic!("{form}: {e:?}"));
        let p = check_schema(&s).expect("the schema checks");
        crate::norm::adhoc(&s, &p, &a).unwrap_or_else(|e| panic!("{form}: {e:?}"));
    }
}

#[test]
fn a_chain_of_nested_chains_counts_its_height() {
    let mut e = "1".to_string();
    for _ in 0..2 {
        e = format!("({e}{})", " + 1".repeat(40));
    }
    let msg = match parse_adhoc(&e, Options::default()).ok() {
        Ok(_) => panic!("{e} parses"),
        Err(ds) => ds[0].message.clone(),
    };
    assert!(msg.starts_with("the text nests more than"), "{msg}");
}

#[test]
fn nested_parenthesised_writes_parse_in_linear_attempts() {
    let n = 18;
    let text = format!(
        "{}true{}",
        "(if ".repeat(n),
        " then 1 else 2) == 1".repeat(n)
    );
    assert!(parse_adhoc(&text, Options::default()).ok().is_ok());
}
