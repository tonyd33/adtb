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
