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
        "grant read catalog::plans to oncall",
        "no module `catalog`",
    );
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
fn commands_over_the_catalog_are_refused_until_it_exists() {
    for text in [
        "pin plan p",
        "unpin plan p",
        "cancel call c",
        "terminate session s",
        "set setting StatementLimit = { limit: None }",
    ] {
        adhoc_refuses(&[], text, "not checked yet");
    }
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
