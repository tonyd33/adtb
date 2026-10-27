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

fn normalises(what: &str, text: &str) {
    let (s, a) = adhoc(&[], text).unwrap_or_else(|e| panic!("{what} does not check: {e:?}"));
    let p = check_schema(&s).expect("the schema checks");
    crate::norm::adhoc(&s, &p, &a).unwrap_or_else(|e| panic!("{what} has no normal form: {e:?}"));
}

#[test]
fn a_long_list_literal_parses_checks_and_normalises() {
    let rows = long_list(10_000, |i| format!("Raw {{ id: raws.key({i}), n: {i} }}"));
    normalises("rows", &format!("insert raws from {rows}"));
    let records = long_list(10_000, |i| format!("{{ n: {i}, m: {i} + 1 }}"));
    normalises("records", &format!("from r in {records} select r.n"));
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
        normalises(form, &text);
    }
}

/// The associative operators, each with an operand of its type.
const ASSOCIATIVE: [(&str, &str); 3] = [("and", "true"), ("or", "true"), ("union", "[1]")];

/// Returns `n` copies of `operand` joined by `op`, inside `m` parentheses.
fn chain_in_parens(op: &str, operand: &str, n: usize, m: usize) -> String {
    format!(
        "{}{}{}",
        "(".repeat(m),
        vec![operand; n].join(&format!(" {op} ")),
        ")".repeat(m)
    )
}

fn parses(text: &str) -> bool {
    parse_adhoc(text, Options::default()).ok().is_ok()
}

#[test]
fn associative_chains_count_the_height_of_a_balanced_tree() {
    for (op, operand) in ASSOCIATIVE {
        let limit = (0..)
            .take_while(|&m| parses(&chain_in_parens(op, operand, 2, m)))
            .last()
            .unwrap_or_else(|| panic!("{op}: a pair parses"));
        assert!(
            limit > 50,
            "{op}: a pair parses in only {limit} parentheses"
        );
        assert!(
            parses(&chain_in_parens(op, operand, 1024, limit - 9)),
            "{op}: 1024 terms are ten levels"
        );
        assert!(
            !parses(&chain_in_parens(op, operand, 1024, limit - 8)),
            "{op}: 1024 terms are more than nine levels"
        );
        assert!(
            parses(&chain_in_parens(op, operand, 1025, limit - 10)),
            "{op}: 1025 terms are eleven levels"
        );
        assert!(
            !parses(&chain_in_parens(op, operand, 1025, limit - 9)),
            "{op}: 1025 terms are more than ten levels"
        );
    }
}

#[test]
fn a_chain_past_the_limit_names_the_limit() {
    for (op, operand) in ASSOCIATIVE {
        let text = chain_in_parens(op, operand, 1 << 10, 60);
        let msg = match parse_adhoc(&text, Options::default()).ok() {
            Ok(_) => panic!("{op}: 70 levels parse"),
            Err(ds) => ds[0].message.clone(),
        };
        assert!(
            msg.starts_with("the text nests more than 64 levels deep"),
            "{op}: {msg}"
        );
    }
}

#[test]
fn a_thousand_term_filter_checks_and_normalises() {
    let terms = |op: &str, cmp: &str| {
        (0..1000)
            .map(|i| format!("r.n {cmp} {i}"))
            .collect::<Vec<_>>()
            .join(&format!(" {op} "))
    };
    let texts = [
        (
            "or",
            format!("from r in raws where {} select r", terms("or", "==")),
        ),
        (
            "and",
            format!("from r in raws where {} select r", terms("and", "!=")),
        ),
        (
            "union",
            (0..1000)
                .map(|i| format!("(from r in raws where r.n == {i} select r)"))
                .collect::<Vec<_>>()
                .join(" union "),
        ),
    ];
    on_small_stack(|| {
        for (op, text) in &texts {
            normalises(op, text);
        }
    });
}

#[test]
fn a_mixed_chain_keeps_precedence() {
    let (_, a) = adhoc(&[], "true or false and false").expect("checks");
    let (_, b) = adhoc(&[], "true or (false and false)").expect("checks");
    assert_eq!(
        format!("{:?}", a.terms.kind(a.items[0])),
        format!("{:?}", b.terms.kind(b.items[0]))
    );
}

#[test]
fn ten_thousand_transact_items_check_and_normalise() {
    let lets: String = (0..10_000)
        .map(|i| format!("  let u{i} = insert users {{ name: \"{i}\" }}\n"))
        .collect();
    let inserts: String = (0..10_000)
        .map(|i| format!("  insert users {{ name: \"{i}\" }}\n"))
        .collect();
    let texts = [
        ("lets", format!("transact\n{lets}")),
        ("inserts", format!("transact\n{inserts}")),
    ];
    on_small_stack(|| {
        for (what, text) in &texts {
            normalises(what, text);
        }
    });
}

#[test]
fn two_thousand_match_arms_check_and_normalise() {
    let arms: String = (0..2000).map(|i| format!(" | {i} -> {i}")).collect();
    let write_arms: String = (0..2000)
        .map(|i| format!(" | {i} -> insert users {{ name: \"{i}\" }}"))
        .collect();
    let texts = [
        (
            "a query match",
            format!("from r in raws select match r.n{arms} | _ -> 0"),
        ),
        (
            "a write match",
            format!("transact let n = 1 match n{write_arms} | _ -> skip"),
        ),
    ];
    on_small_stack(|| {
        for (what, text) in &texts {
            normalises(what, text);
        }
    });
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

const LOGINS: &str = r"
type Login { id: Key logins, user: Key users }
  = Password { email: Text }
  | Sso      { issuer: Text, subject: Text }
  | Passkey  { credential: Bytes }
relation logins : Login
  key id generate sequence
  unique (lower(email)) in Password
  index (user) in (Sso | Passkey)
  index (issuer, subject) in Sso
  index (user)
set Federated of logins = Sso | Passkey

type Settled = Open | Paid { receipt: Text }
type Bill = { id: Key bills, state: Settled }
relation bills : Bill key id generate sequence unique (state.Paid.receipt)
";

fn reindexed(text: &str) -> Result<Option<usize>, Vec<String>> {
    let a = adhoc_over(LOGINS, text)?;
    match a.commands.as_slice() {
        [
            AdhocCommand {
                command: Command::Reindex { index, .. },
                ..
            },
        ] => Ok(*index),
        cs => panic!("{text}\nis not one reindex: {cs:?}"),
    }
}

fn reindexes(text: &str, index: usize) {
    match reindexed(text) {
        Ok(found) => assert_eq!(Some(index), found, "{text}"),
        Err(errs) => panic!("{text}\nrefused: {errs:?}"),
    }
}

fn reindex_refused(text: &str, needle: &str) {
    match reindexed(text) {
        Ok(found) => panic!("{text}\naccepted as index {found:?}; expected {needle:?}"),
        Err(errs) => assert!(
            errs.iter().any(|e| e.contains(needle)),
            "{text}\nno error containing {needle:?} in {errs:?}"
        ),
    }
}

const NO_INDEX: &str = "declares no index or unique with these fields";

#[test]
fn reindex_names_a_declared_index_by_its_clause() {
    reindexes("reindex logins index (lower(email)) in Password", 0);
    reindexes("reindex logins index (user) in (Sso | Passkey)", 1);
    reindexes("reindex logins index (issuer, subject) in Sso", 2);
    reindexes(
        "reindex logins index (user) in (Password | Sso | Passkey)",
        3,
    );
    reindexes("reindex bills index (state.Paid.receipt) in Bill", 0);
}

#[test]
fn reindex_matches_the_clause_however_it_is_spaced_or_grouped() {
    reindexes("reindex logins index ( lower( email ) ) in Password", 0);
    reindexes("reindex logins index ((user)) in (Passkey | Sso)", 1);
    reindexes("reindex logins index (user) in Federated", 1);
    reindexes(
        "reindex logins\n  index (issuer,subject)\n  in Sso offline",
        2,
    );
}

#[test]
fn reindex_refuses_an_index_that_is_not_declared() {
    reindex_refused("reindex logins index (email) in Password", NO_INDEX);
    reindex_refused("reindex logins index (upper(email)) in Password", NO_INDEX);
    reindex_refused("reindex logins index (user) in Sso", NO_INDEX);
    reindex_refused("reindex logins index (user) in (Password | Sso)", NO_INDEX);
    reindex_refused("reindex logins index (subject, issuer) in Sso", NO_INDEX);
    reindex_refused("reindex logins index (issuer) in Sso", NO_INDEX);
    reindex_refused("reindex logins index (id) in Sso", NO_INDEX);
    reindex_refused("reindex logins index (subject) in Sso", NO_INDEX);
}

#[test]
fn reindex_still_type_checks_its_fields_and_scope() {
    reindex_refused("reindex logins index (email) in Sso", "email");
    reindex_refused(
        "reindex logins index (user) in Nope",
        "neither a constructor",
    );
    reindex_refused("reindex logins index (nosuch) in Sso", "nosuch");
}

#[test]
fn reindex_without_a_clause_rebuilds_every_index() {
    let a = adhoc_over(LOGINS, "reindex logins").expect("checks");
    let [AdhocCommand { command, .. }] = a.commands.as_slice() else {
        panic!("one command: {:?}", a.commands);
    };
    let s = schema(LOGINS).expect("the schema builds");
    let logins = rel_id(&s, "logins");
    assert_eq!(
        &Command::Reindex {
            relation: logins,
            index: None,
            online: true,
        },
        command
    );
}

#[test]
fn reindex_is_online_unless_offline_is_written() {
    let online = |text: &str| match adhoc_over(LOGINS, text)
        .expect("checks")
        .commands
        .as_slice()
    {
        [
            AdhocCommand {
                command: Command::Reindex { online, .. },
                ..
            },
        ] => *online,
        cs => panic!("{text}\nis not one reindex: {cs:?}"),
    };
    assert!(online("reindex logins index (user) in Federated"));
    assert!(online("reindex logins online"));
    assert!(!online("reindex logins index (user) in Federated offline"));
}

const LOGIN_KEY: RegisteredQuery = ("web", "2.4", "login_key", &["Key users"]);

fn explained(params: &[(&str, &str)], text: &str) -> (Adhoc, Explained) {
    match adhoc_in(&[LOGIN_KEY], params, text) {
        Ok((_, a)) => match a.commands.as_slice() {
            [
                AdhocCommand {
                    command: Command::Explain { target, .. },
                    ..
                },
            ] => {
                let target = target.clone();
                (a, target)
            }
            cs => panic!("{text}\nis not one explain: {cs:?}"),
        },
        Err(errs) => panic!("{text}\nrefused: {errs:?}"),
    }
}

fn explain_refused(bundles: &[RegisteredQuery], text: &str, needle: &str) {
    match adhoc_in(bundles, &[], text) {
        Ok(_) => panic!("{text}\naccepted; expected an error containing {needle:?}"),
        Err(errs) => assert!(
            errs.iter().any(|e| e.contains(needle)),
            "{text}\nno error containing {needle:?} in {errs:?}"
        ),
    }
}

#[test]
fn explain_of_a_registered_call_checks_against_its_bundle() {
    let (a, target) = explained(&[], "explain analyze web-2.4::login_key(users.key(3))");
    let Explained::Registered {
        bundle,
        query,
        args,
    } = target
    else {
        panic!("a registered call: {target:?}");
    };
    assert_eq!(
        ("web", "2.4"),
        (bundle.family.as_str(), bundle.version.as_str())
    );
    assert_eq!("login_key", query);
    let [arg] = args.as_slice() else {
        panic!("one argument: {args:?}");
    };
    assert!(matches!(a.terms.ty(*arg), Ty::Key { .. }));
    assert!(a.items.is_empty());
    explained(&[("u", "Key users")], "explain web-2.4::login_key(u)");
}

#[test]
fn explain_of_a_call_of_an_unknown_bundle_is_refused() {
    explain_refused(
        &[],
        "explain web-2.4::login_key(users.key(3))",
        "bundle `web-2.4` is not registered",
    );
    explain_refused(
        &[LOGIN_KEY],
        "explain web-2.5::login_key(users.key(3))",
        "bundle `web-2.5` is not registered",
    );
    explain_refused(
        &[LOGIN_KEY],
        "explain web-2.4::logout(users.key(3))",
        "bundle `web-2.4` registers no query `logout`",
    );
}

#[test]
fn explain_of_a_registered_call_checks_its_arguments() {
    explain_refused(
        &[LOGIN_KEY],
        "explain web-2.4::login_key(3)",
        "expected Key users",
    );
    explain_refused(
        &[LOGIN_KEY],
        "explain web-2.4::login_key(users.key(3), 4)",
        "`web-2.4::login_key` takes 1 argument, given 2",
    );
    explain_refused(
        &[LOGIN_KEY],
        "explain web-2.4::login_key()",
        "`web-2.4::login_key` takes 1 argument, given 0",
    );
    explain_refused(
        &[LOGIN_KEY],
        "explain web-2.4::login_key(nosuch)",
        "supplies no value",
    );
}

#[test]
fn explain_keeps_the_term_its_statement_would_have() {
    for text in [
        "from u in users select u.name",
        "insert users { name: \"a\" }",
        "transact insert users { name: \"a\" }",
    ] {
        let (_, statement) = adhoc(&[], text).expect("checks");
        let (a, target) = explained(&[], &format!("explain {text}"));
        let Explained::Statement(t) = target else {
            panic!("{text}\nis not a statement: {target:?}");
        };
        assert_eq!(
            statement.terms.ty(statement.items[0]),
            a.terms.ty(t),
            "{text}"
        );
        assert!(a.items.is_empty(), "{text}");
    }
}

#[test]
fn explain_analyze_of_a_write_is_held_to_the_rules_of_writes() {
    adhoc_refuses(
        &[],
        "explain analyze insert users from (from s in sys::sessions select { name: s.principal })",
        "a write must not depend on a volatile relation",
    );
    adhoc_accepts(&[], "explain from s in sys::sessions select s.principal");
}

fn command_of(params: &[(&str, &str)], text: &str) -> (Schema, Adhoc, Command) {
    let (s, a) = adhoc(params, text).unwrap_or_else(|e| panic!("{text}\nrefused: {e:?}"));
    let [AdhocCommand { command, .. }] = a.commands.as_slice() else {
        panic!("{text}\nis not one command: {:?}", a.commands);
    };
    let command = command.clone();
    (s, a, command)
}

#[test]
fn every_command_keeps_the_terms_of_its_expressions() {
    type Case<'a> = (&'a [(&'a str, &'a str)], &'a str, &'a str);
    let cases: &[Case] = &[
        (
            &[],
            "pin plan catalog::plans.key(8812)",
            "Key catalog::plans",
        ),
        (
            &[("p", "Key catalog::plans")],
            "unpin plan p",
            "Key catalog::plans",
        ),
        (&[], "cancel call sys::calls.key(812)", "Key sys::calls"),
        (
            &[("s", "Key sys::sessions")],
            "terminate session s",
            "Key sys::sessions",
        ),
        (
            &[],
            "set setting VacuumBudget = { io: 0.2, cpu: 0.5 }",
            "{ cpu: F64, io: F64 }",
        ),
        (
            &[],
            "export (from i in identities.Google select i) to \"google.parquet\"",
            "Set Identity.Google",
        ),
        (
            &[],
            "archive audit through 991204 to \"audit.arrow\"",
            "I64",
        ),
        (&[("k", "Text")], "dump to \"d\" encrypt k", "Text"),
        (
            &[("t", "Instant")],
            "restore from \"d\" to \"f\" at instant t",
            "Instant",
        ),
        (
            &[("p", "Position")],
            "restore from \"d\" to \"f\" at position p",
            "Position",
        ),
        (&[], "explain from u in users select u.name", "Set Text"),
    ];
    for (params, text, ty) in cases {
        let (s, a, command) = command_of(params, text);
        let terms = command.terms();
        let [t] = terms.as_slice() else {
            panic!("{text}\nhas not one term: {terms:?}");
        };
        assert_eq!(*ty, s.show_ty(a.terms.ty(*t)), "{text}");
        assert!(a.items.is_empty(), "{text}");
    }
}

#[test]
fn commands_with_no_expression_keep_what_they_name() {
    let (s, _, c) = command_of(&[], "vacuum users");
    assert_eq!(Command::Vacuum(rel_id(&s, "users")), c);
    let (s, _, c) = command_of(&[], "verify identities");
    assert_eq!(Command::Verify(Some(rel_id(&s, "identities"))), c);
    assert_eq!(Command::Verify(None), command_of(&[], "verify").2);
    assert_eq!(Command::Compact, command_of(&[], "compact").2);
    assert_eq!(
        Command::FinalizeUpgrade,
        command_of(&[], "finalize upgrade").2
    );
    assert_eq!(Command::PrintSchema, command_of(&[], "print schema").2);
    assert_eq!(Command::RotateDek, command_of(&[], "rotate key dek").2);
    assert_eq!(
        Command::RotateKek("kms://k".into()),
        command_of(&[], "rotate key kek \"kms://k\"").2
    );
    assert_eq!(
        Command::DestroyDek(3),
        command_of(&[], "destroy key dek 3").2
    );
    assert_eq!(
        Command::Backup {
            path: "b".into(),
            incremental: true
        },
        command_of(&[], "backup to \"b\" incremental").2
    );
    assert_eq!(
        Command::RegisterBundle("web-2.4.adt".into()),
        command_of(&[], "register bundle \"web-2.4.adt\"").2
    );
    assert_eq!(
        Command::ApplyMigrations(vec!["0013.adt".into(), "0014.adt".into()]),
        command_of(&[], "apply migrations \"0013.adt\" \"0014.adt\"").2
    );
    assert!(matches!(
        command_of(&[], "retire bundle web-2.4").2,
        Command::RetireBundle(b) if b.family == "web" && b.version == "2.4"
    ));
    assert!(matches!(
        command_of(&[], "supersede bundle web-2.3 by web-2.4").2,
        Command::SupersedeBundle(a, b) if a.version == "2.3" && b.version == "2.4"
    ));
    assert_eq!(
        Command::Restore {
            from: "d".into(),
            to: "f".into(),
            point: Some(RestorePoint::BeforeVersion(12)),
        },
        command_of(&[], "restore from \"d\" to \"f\" before version 12").2
    );
    assert_eq!(
        Command::Dump {
            path: "d".into(),
            encrypt: None
        },
        command_of(&[], "dump to \"d\"").2
    );
    assert!(matches!(
        command_of(&[], "set setting StatementLimit = { limit: None }").2,
        Command::SetSetting { .. }
    ));
}

#[test]
fn commands_keep_their_place_among_the_statements() {
    let (_, a) = adhoc(
        &[],
        "compact\nfrom u in users select u.name\nvacuum users\nexplain from p in posts select p\nfrom p in posts select p.title\nprint schema",
    )
    .expect("checks");
    assert_eq!(2, a.items.len());
    let places: Vec<usize> = a.commands.iter().map(|c| c.at).collect();
    assert_eq!(vec![0, 1, 1, 2], places);
}

#[test]
fn command_terms_normalise_with_the_statements() {
    let (s, a) = adhoc_in(
        &[LOGIN_KEY],
        &[("u", "Key users")],
        "from x in users select x.name
export (from i in identities where i.user == u select display_name(i)) to \"x\"
explain web-2.4::login_key(u)",
    )
    .expect("checks");
    let program = check_schema(&s).expect("the schema checks");
    let n = crate::norm::adhoc(&s, &program, &a).expect("normalises");
    assert_eq!(1, n.items.len());
    assert_eq!(2, n.commands.len());
    for (c, d) in a.commands.iter().zip(&n.commands) {
        assert_eq!(c.at, d.at);
        assert_eq!(c.command.terms().len(), d.command.terms().len());
        for (t, u) in c.command.terms().into_iter().zip(d.command.terms()) {
            assert_eq!(a.terms.ty(t), n.terms.ty(u));
        }
    }
    let Command::Export { source, .. } = n.commands[0].command else {
        panic!("an export: {:?}", n.commands[0]);
    };
    assert!(
        !n.terms.any(source, |k| matches!(k, Kind::Call { .. })),
        "a normal export source inlines its calls"
    );
}
