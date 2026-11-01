//! Writes, delete actions, upserts, blocks and the statement boundary, driven through the front
//! end. The examples of the writes part of the spec whose outcome it states are here.

use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::check::check_adhoc;
use adtb_lang::kernel::Kind;
use adtb_lang::parse::{Options, parse_adhoc};

use adtb_reference::{Abort, Rejection, Value};
use adtb_testkit::fixture::*;
use adtb_testkit::writes::fixture;

#[path = "writes/mod.rs"]
mod cases;

#[test]
fn runtime_write_cases() {
    let (families, steps) = cases::run();
    eprintln!("runtime writes: {families} families, {steps} steps executed");
}

fn names(f: &Fixture) -> Value {
    f.value("from u in users select u.name")
}

fn violated(r: Rejection) -> (String, BTreeSet<Value>) {
    match r {
        Rejection::Violated { constraint, report } => (constraint, report),
        other => panic!("not a constraint violation: {other:?}"),
    }
}

fn key_violation(relation: &str, key: &str) -> Rejection {
    Rejection::KeyViolation {
        relation: relation.into(),
        key: key.into(),
    }
}

fn count(f: &Fixture, rel: &str) -> usize {
    f.table(rel).as_set().expect("a set").len()
}

// Change sets and combining writes

// Insert

#[test]
fn rejected_records_go_to_the_rejects_relation() {
    let mut f = fixture();
    let out = f
        .run(
            r#"insert users from (
                 from r in raws select User { name: if r.n > 0 then "ok" else reject "negative" })
               rejects into user_rejects"#,
        )
        .expect("loaded");
    assert_eq!(Value::unit(), out.result);
    assert_eq!(texts(["ann", "bo", "cy", "dee", "ok"]), names(&f));
    assert_eq!(6, count(&f, "users"));
    let j = f.one("from j in user_rejects select j");
    assert_eq!(Some(&int(2)), j.field("line"));
    assert_eq!(Some(&text("raws")), j.field("source"));
    assert_eq!(Some(&text("negative")), j.field("reason"));
    let Some(Value::Text(raw)) = j.field("raw") else {
        panic!("a raw record")
    };
    assert!(raw.contains("raws.key(2)"), "{raw}");
    let first = j.field("load").cloned();
    f.ok(r#"insert users from (
             from r in raws select User { name: if r.n > 0 then "ok" else reject "negative" })
           rejects into user_rejects"#);
    let loads = f.value("from j in user_rejects select j.load");
    assert_eq!(2, loads.as_set().expect("a set").len());
    assert!(
        loads
            .as_set()
            .expect("a set")
            .contains(&first.expect("a load"))
    );
}

// Update and retag

// Delete

// Delete actions

// Upsert

#[test]
fn an_upsert_conflict_outside_olds_type_inserts_without_evaluating_its_branch() {
    let mut f = Fixture::new(
        "type Note { id: Key notes } = Plain { body: Text } | Link { url: Text }\nrelation notes : Note key id generate sequence",
    );
    f.ok(r#"insert notes.Link { id: notes.key(1), url: "u" }"#);
    let text = r#"insert notes.Plain { id: notes.key(1), body: "x" }
      unless conflict on key else fail "the conflict branch ran""#;
    assert_eq!(
        Err(Abort::Reject(key_violation("notes", "notes.key(1)"))),
        run_with_restricted_old(&f, text)
    );
}

fn run_with_restricted_old(f: &Fixture, text: &str) -> Result<adtb_reference::Outcome, Abort> {
    let items = parse_adhoc(text, Options::default())
        .ok()
        .expect("the upsert parses");
    let mut checked =
        check_adhoc(&f.schema, &f.program, &items, &BTreeMap::new()).expect("the upsert checks");
    let olds: Vec<_> = checked
        .terms
        .nodes
        .iter()
        .filter_map(|n| match n.kind {
            Kind::Upsert { old, .. } => Some(old),
            _ => None,
        })
        .collect();
    assert_eq!(1, olds.len());
    checked.terms.vars[olds[0].0 as usize].ty = f.ty("Note.Plain");
    let mut supply = f.supply.clone();
    f.interp()
        .run(&checked.terms, checked.items[0], &[], &mut supply)
}

#[test]
fn a_scoped_unique_conflict_obeys_olds_constructor_type() {
    let mut f = Fixture::new(
        "type Note { id: Key notes, slug: Text } = Plain { body: Text } | Other { other: Text } | Link { url: Text }\nrelation notes : Note key id generate sequence unique (slug) in (Plain | Link)",
    );
    f.ok(r#"insert notes.Link { id: notes.key(9), slug: "same", url: "u" }"#);
    let text = r#"insert notes.Plain { id: notes.key(1), slug: "same", body: "x" }
      unless conflict on (slug) in (Plain | Link) else fail "the conflict branch ran""#;
    let plain = Value::Ctor(
        0,
        vec![
            ("id".into(), key(1)),
            ("slug".into(), Value::text("same")),
            ("body".into(), Value::text("x")),
        ],
    );
    let link = Value::Ctor(
        2,
        vec![
            ("id".into(), key(9)),
            ("slug".into(), Value::text("same")),
            ("url".into(), Value::text("u")),
        ],
    );
    assert_eq!(
        Err(Abort::Reject(Rejection::Violated {
            constraint: "app::notes.unique(slug) in (Link | Plain)".into(),
            report: BTreeSet::from([
                Value::record([("a", plain.clone()), ("b", link.clone())]),
                Value::record([("a", link), ("b", plain)]),
            ]),
        })),
        run_with_restricted_old(&f, text)
    );
    f.ok("delete n in notes");
    f.ok(r#"insert notes.Plain { id: notes.key(9), slug: "same", body: "a" }"#);
    assert_eq!(
        Err(Abort::Reject(Rejection::UserError {
            message: "the conflict branch ran".into(),
        })),
        run_with_restricted_old(&f, text)
    );
}

#[test]
fn an_upsert_on_a_unique_over_expressions() {
    let mut f = fixture();
    f.run_with(
        "insert identities.EmailPass { user: u, email: e, hash: h }
         unless conflict on (lower(email)) in EmailPass
         else update old set old",
        &[
            ("u", "Key users", key(3)),
            ("e", "Text", text("ANN@example.com")),
            ("h", "Bytes", Value::Bytes(b"h".to_vec())),
        ],
    )
    .expect("upserted");
    assert_eq!(5, count(&f, "identities"));
    let (name, _) = violated(f.rejected(
        r#"insert identities.EmailPass { user: users.key(3), email: "ANN@example.com", hash: bytes("h") }"#,
    ));
    let r = f.rel("identities");
    let by_id = f.schema.ids.indexes.iter().any(|(&(at, _), id)| {
        at == r && *id >= 1024 && name == format!("app::identities.unique#{id}")
    });
    assert!(by_id, "{name} is named by the unique's catalog id");
}

// The statement boundary

// Blocks and transactions

#[test]
fn a_unique_over_a_value_level_path_constrains_only_rows_holding_the_constructor() {
    let mut f = Fixture::new(
        "type Status = Draft | Paid { ref: Text }
         type Invoice = { id: Key invoices, status: Status }
         relation invoices : Invoice key id generate sequence unique (status.Paid.ref)",
    );
    f.ok("(insert invoices { status: Draft }, insert invoices { status: Draft })");
    f.ok(r#"insert invoices { status: Paid { ref: "r1" } }"#);
    let (name, report) = violated(f.rejected(r#"insert invoices { status: Paid { ref: "r1" } }"#));
    assert_eq!("app::invoices.unique(status.Paid.ref)", name);
    assert_eq!(2, report.len());
}

#[test]
fn an_upsert_on_a_value_level_path_meets_only_rows_holding_the_constructor() {
    let mut f = Fixture::new(
        "type Status = Draft | Paid { ref: Text }
         type Invoice = { id: Key invoices, status: Status, n: I64 }
         relation invoices : Invoice key id generate sequence unique (status.Paid.ref)",
    );
    f.ok(r#"insert invoices { status: Paid { ref: "r1" }, n: 1 }"#);
    let upsert = |n: i64| {
        format!(
            r#"insert invoices {{ status: Paid {{ ref: "r1" }}, n: {n} }}
               unless conflict on (status.Paid.ref)
               else update old set Invoice {{ old with n: old.n + 10 }}"#
        )
    };
    f.ok(&upsert(2));
    assert_eq!(1, count(&f, "invoices"));
    assert_eq!(int(11), f.one(r"from i in invoices select i.n"));
    f.ok(r"insert invoices { status: Draft, n: 5 }
            unless conflict on (status.Paid.ref)
            else update old set Invoice { old with n: 0 }");
    assert_eq!(2, count(&f, "invoices"));
}

// Rows too large

/// Runs the one statement of `text` over the fixture with a row limit of `limit` bytes, a row's
/// size the length of its textual syntax.
fn run_limited(
    f: &mut Fixture,
    text: &str,
    limit: usize,
) -> Result<adtb_reference::Outcome, Abort> {
    use adtb_lang::check::check_adhoc;
    use adtb_lang::parse::{Options, parse_adhoc};
    let items = parse_adhoc(text, Options::default())
        .ok()
        .expect("the text parses");
    let adhoc =
        check_adhoc(&f.schema, &f.program, &items, &Default::default()).expect("the text checks");
    let schema = &f.schema;
    let measure = |r: adtb_lang::types::RelId, _: u32, v: &Value| {
        let ty = schema.relation(r).elem.clone();
        let size = adtb_reference::textual::print(schema, v, &ty)
            .expect("a row prints")
            .len();
        (size, limit)
    };
    let out = f.interp().with_row_limit(&measure).run(
        &adhoc.terms,
        adhoc.items[0],
        &[],
        &mut f.supply.clone(),
    );
    if let Ok(o) = &out {
        f.db = o.db.clone();
    }
    out
}

fn too_large(r: Result<adtb_reference::Outcome, Abort>) -> (String, String, i64, i64) {
    match r {
        Err(Abort::Reject(Rejection::RowTooLarge {
            relation,
            ctor,
            size,
            limit,
        })) => (relation, ctor, size, limit),
        other => panic!("not too large: {other:?}"),
    }
}

#[test]
fn a_new_row_over_the_row_limit_is_row_too_large() {
    let mut f = fixture();
    let long = "y".repeat(60);
    let (relation, ctor, size, limit) = too_large(run_limited(
        &mut f,
        &format!(r#"insert users {{ id: users.key(9), name: "{long}" }}"#),
        60,
    ));
    assert_eq!(
        ("users", "User", 60),
        (relation.as_str(), ctor.as_str(), limit)
    );
    assert!(size > limit, "{size} over {limit}");
    run_limited(
        &mut f,
        r#"insert users { id: users.key(9), name: "z" }"#,
        60,
    )
    .expect("a short row fits");
}

#[test]
fn an_update_or_retag_to_a_row_over_the_limit_is_row_too_large() {
    let mut f = fixture();
    let long = "y".repeat(60);
    too_large(run_limited(
        &mut f,
        &format!(
            r#"update u in users where key u == users.key(1) set User {{ u with name: "{long}" }}"#
        ),
        60,
    ));
    let (relation, ctor, _, _) = too_large(run_limited(
        &mut f,
        &format!(
            r#"update i in identities where match i | Google g -> g.gid == "g-ann" | EmailPass _ -> false | Apple _ -> false
               set match i
                 | Google g -> Apple {{ sub: "{long}" }}
                 | EmailPass e -> e
                 | Apple a -> a"#
        ),
        120,
    ));
    assert_eq!(("identities", "Apple"), (relation.as_str(), ctor.as_str()));
}

#[test]
fn a_row_too_large_takes_precedence_over_keys_and_constraints() {
    let mut f = fixture();
    let long = "y".repeat(60);
    too_large(run_limited(
        &mut f,
        &format!(r#"insert users {{ id: users.key(1), name: "{long}" }}"#),
        60,
    ));
    too_large(run_limited(
        &mut f,
        &format!(r#"insert badges {{ owner: users.key(99), label: "{long}" }}"#),
        60,
    ));
}

#[test]
fn rows_the_statement_does_not_write_are_not_measured() {
    let mut f = fixture();
    run_limited(
        &mut f,
        r#"insert users { id: users.key(9), name: "z" }"#,
        40,
    )
    .expect("only the new row is measured");
}
