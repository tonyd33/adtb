//! Writes, delete actions, upserts, blocks and the statement boundary, driven through the front
//! end. The examples of the writes part of the spec whose outcome it states are here.

mod common;

use std::collections::BTreeSet;

use adtb_reference::{Abort, Fault, Rejection, Stage, Value};
use common::*;

const SCHEMA: &str = r#"
type User = { id: Key users, name: Text }
type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text, hash: Bytes }
  | Google    { gid: Text }
  | Apple     { sub: Text }
relation users : User key id generate sequence
relation identities : Identity
  key id generate uuidv7
  unique (email) in EmailPass
  unique (gid) in Google
  unique (sub) in Apple
  unique (lower(email)) in EmailPass
set OAuth of identities = Google | Apple
constraint login_present =
  from i in identities
  where match i
    | EmailPass e -> scalar_len(e.email) == 0
    | Google g    -> scalar_len(g.gid) == 0
    | Apple a     -> scalar_len(a.sub) == 0
  select i

type Login { id: Key logins, user: Key users on delete cascade }
  = Active  { identity: Key identities on delete retag Revoked {} }
  | Revoked { at: Instant }
relation logins : Login
  key id generate sequence
  default Revoked.at = generate clock

type Reset = { id: Key resets, identity: Key identities.EmailPass on delete cascade, token: Bytes }
relation resets : Reset key id generate sequence

type Badge = { id: Key badges, owner: Key users on delete restrict, label: Text }
relation badges : Badge key id generate sequence

type Import { id: Key imports } = Email { user: Key users, addr: Text, h: Bytes }
                                | Oauth { user: Key users, sub: Text }
relation imports : Import key id generate sequence

type Note = { id: Key notes, author: Option (Key users) on delete clear, body: Text }
relation notes : Note key id generate sequence

fn lookup (email: Text) -> Text = lower(email)
fn rehash (h: Bytes) -> Bytes = h

type Address = US { zip: Text, street: Text } | Intl { country: Text, postcode: Text }
type Customer = { id: Key customers, name: Text, address: Address }
relation customers : Customer key id generate sequence

type Ticket { id: Key tickets, owner: Option (Key users) on delete clear }
  = Open { identity: Key identities on delete retag Closed {} }
  | Closed
relation tickets : Ticket key id generate sequence

type Raw = { id: Key raws, n: I64 }
relation raws : Raw key id generate sequence
relation user_rejects : RejectedRow key (load, line)

type Email = new Text where scalar_len(self) > 3
type Contact = { id: Key contacts, email: Email }
relation contacts : Contact key id generate sequence

constraint aa_names = from u in users where u.name == "aa" select u
constraint zz_names = from u in users where u.name == "zz" select u
constraint frozen_notes = from n in deleted(notes) where n.body == "frozen" select n
check name_short on users (scalar_len(name) < 20)
check divides on raws (10 / n != 0)
"#;

fn uuid(n: u8) -> String {
    format!("identities.key(uuid(\"00000000-0000-7000-8000-0000000000{n:02}\"))")
}

fn fixture() -> Fixture {
    let mut f = Fixture::new(SCHEMA);
    f.rows(
        "users",
        r#"[User { id: users.key(1), name: "ann" },
            User { id: users.key(2), name: "bo" },
            User { id: users.key(3), name: "cy" },
            User { id: users.key(4), name: "dee" }]"#,
    );
    f.rows(
        "identities",
        &format!(
            r#"[EmailPass {{ id: {}, user: users.key(1), email: "ann@example.com", hash: bytes("h1") }},
                Google {{ id: {}, user: users.key(1), gid: "g-ann" }},
                Google {{ id: {}, user: users.key(2), gid: "g-bo" }},
                Google {{ id: {}, user: users.key(1), gid: "s-bo" }},
                EmailPass {{ id: {}, user: users.key(2), email: "Bo@Example.com", hash: bytes("h2") }}]"#,
            uuid(1),
            uuid(2),
            uuid(3),
            uuid(4),
            uuid(5)
        ),
    );
    f.rows_with(
        "logins",
        &format!(
            r#"[Active {{ id: logins.key(1), user: users.key(1), identity: {} }},
                Active {{ id: logins.key(2), user: users.key(2), identity: {} }},
                Revoked {{ id: logins.key(3), user: users.key(1), at: t }}]"#,
            uuid(1),
            uuid(3)
        ),
        &[("t", "Instant", Value::Instant(5))],
    );
    f.rows(
        "resets",
        r#"from e in identities.EmailPass where e.email == "ann@example.com"
           select Reset { id: resets.key(1), identity: key e, token: bytes("t") }"#,
    );
    f.rows(
        "badges",
        r#"[Badge { id: badges.key(1), owner: users.key(3), label: "b3" },
            Badge { id: badges.key(2), owner: users.key(4), label: "b4" }]"#,
    );
    f.rows(
        "imports",
        r#"[Oauth { id: imports.key(1), user: users.key(2), sub: "s-bo" },
            Email { id: imports.key(2), user: users.key(3), addr: "cy@x.org", h: bytes("h3") }]"#,
    );
    f.rows(
        "notes",
        r#"[Note { id: notes.key(1), author: Some { value: users.key(4) }, body: "n" },
            Note { id: notes.key(2), author: None, body: "plain" },
            Note { id: notes.key(3), author: None, body: "frozen" }]"#,
    );
    f.rows(
        "customers",
        r#"[Customer { id: customers.key(1), name: "c", address: US { zip: "10001", street: "s" } },
            Customer { id: customers.key(2), name: "d", address: Intl { country: "fr", postcode: "75001" } }]"#,
    );
    f.rows(
        "tickets",
        &format!(
            r#"[Open {{ id: tickets.key(1), owner: Some {{ value: users.key(4) }}, identity: {} }}]"#,
            uuid(2)
        ),
    );
    f.rows(
        "raws",
        "[Raw { id: raws.key(1), n: 5 }, Raw { id: raws.key(2), n: -1 }, Raw { id: raws.key(3), n: 7 }]",
    );
    f
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

#[test]
fn an_insert_of_an_existing_key_is_a_key_violation_even_when_equal() {
    let mut f = fixture();
    assert_eq!(
        key_violation("users", "users.key(1)"),
        f.rejected(r#"insert users { id: users.key(1), name: "ann" }"#)
    );
}

#[test]
fn an_insert_fills_its_generated_key_with_an_unused_value() {
    let mut f = fixture();
    let out = f
        .run_with("insert users { name: n }", &[("n", "Text", text("eve"))])
        .expect("inserted");
    assert_eq!(Value::unit(), out.result);
    let k = f.one(r#"from u in users where u.name == "eve" select key u"#);
    assert!(![key(1), key(2), key(3), key(4)].contains(&k));
    assert_eq!(5, count(&f, "users"));
}

#[test]
fn an_insert_into_a_constructor_relation() {
    let mut f = fixture();
    f.run_with(
        "insert identities.Google { user: u, gid: g }",
        &[("u", "Key users", key(3)), ("g", "Text", text("g-cy"))],
    )
    .expect("inserted");
    assert_eq!(
        key(3),
        f.one(r#"from g in identities.Google where g.gid == "g-cy" select g.user"#)
    );
}

#[test]
fn an_insert_from_a_query_into_a_constructor_relation() {
    let mut f = fixture();
    f.ok("delete i in identities.Google where i.gid == \"s-bo\"");
    f.ok("insert identities.Google from (
            from x in imports.Oauth select { user: x.user, gid: x.sub })");
    assert_eq!(
        key(2),
        f.one(r#"from g in identities.Google where g.gid == "s-bo" select g.user"#)
    );
}

#[test]
fn an_insert_into_a_sum_takes_constructors_from_the_query() {
    let mut f = fixture();
    f.ok("delete i in identities.Google where i.gid == \"s-bo\"");
    f.ok("insert identities from (
            from x in imports
            select match x
              | Email e -> EmailPass { user: e.user, email: e.addr, hash: e.h }
              | Oauth o -> Google { user: o.user, gid: o.sub })");
    assert_eq!(
        key(3),
        f.one(r#"from e in identities.EmailPass where e.email == "cy@x.org" select e.user"#)
    );
    assert_eq!(
        key(2),
        f.one(r#"from g in identities.Google where g.gid == "s-bo" select g.user"#)
    );
}

#[test]
fn two_rows_inserted_with_one_key_are_a_key_violation() {
    let mut f = fixture();
    assert_eq!(
        key_violation("users", "users.key(9)"),
        f.rejected(
            r#"(insert users { id: users.key(9), name: "a" }, insert users { id: users.key(9), name: "b" })"#
        )
    );
}

#[test]
fn two_updates_of_one_row_to_different_values_are_a_key_violation() {
    let mut f = fixture();
    assert_eq!(
        key_violation("users", "users.key(1)"),
        f.rejected(
            r#"(update u in users where key u == users.key(1) set User { u with name: "x" },
                update u in users where key u == users.key(1) set User { u with name: "y" })"#
        )
    );
}

#[test]
fn two_equal_updates_of_one_row_are_one_update() {
    let mut f = fixture();
    f.ok(
        r#"(update u in users where key u == users.key(1) set User { u with name: "x" },
            update u in users where key u == users.key(1) set User { u with name: "x" })"#,
    );
    assert_eq!(texts(["x", "bo", "cy", "dee"]), names(&f));
}

#[test]
fn a_delete_and_an_insert_of_one_key_are_an_update() {
    let mut f = fixture();
    let out = f
        .run(
            r#"(delete u in users where key u == users.key(1),
                insert users { id: users.key(1), name: "new" })"#,
        )
        .expect("an update");
    let users = f.rel("users");
    let rows = |db: &adtb_reference::Database| db.get(&(users, 0)).cloned().unwrap_or_default();
    assert_eq!(1, rows(&out.change.inserted).len());
    assert_eq!(1, rows(&out.change.deleted).len());
    assert_eq!(texts(["new", "bo", "cy", "dee"]), names(&f));
}

#[test]
fn an_update_that_changes_nothing_has_an_empty_change_set() {
    let mut f = fixture();
    let out = f
        .run(
            "update i in identities where i.user == users.key(1)
             set match i
               | EmailPass e -> EmailPass { e with hash: rehash(e.hash) }
               | Google g    -> g
               | Apple a     -> a",
        )
        .expect("ran");
    assert!(out.change.inserted.is_empty());
    assert!(out.change.deleted.is_empty());
}

#[test]
fn writes_of_a_statement_read_one_snapshot() {
    let mut f = fixture();
    f.ok(r#"(insert users { name: "eve" },
            update u in users set User { u with name: u.name ++ "!" })"#);
    assert_eq!(texts(["ann!", "bo!", "cy!", "dee!", "eve"]), names(&f));
}

#[test]
fn a_fault_while_computing_a_change_set_aborts_the_statement() {
    let mut f = fixture();
    let before = f.db.clone();
    assert_eq!(
        Err(Abort::Fault(Fault::DivisionByZero)),
        f.run_with(
            r#"(insert users { name: "eve" }, if 1 / z > 0 then skip else skip)"#,
            &[("z", "I64", int(0))]
        )
        .map(|o| o.result)
    );
    assert_eq!(before, f.db);
}

#[test]
fn fail_rejects_the_statement_with_a_user_error() {
    let mut f = fixture();
    assert_eq!(
        Rejection::UserError {
            message: "no".into()
        },
        f.rejected(r#"(insert users { name: "eve" }, fail "no")"#)
    );
    f.ok(r#"if false then fail "no" else insert users { name: "eve" }"#);
    assert_eq!(5, count(&f, "users"));
}

#[test]
fn try_does_not_catch_fail() {
    let mut f = fixture();
    assert!(matches!(
        f.rejected(r#"transact let x = try(1) fail "stop" return x"#),
        Rejection::UserError { .. }
    ));
}

// Insert

#[test]
fn a_block_binds_inserted_rows_with_their_generated_keys() {
    let mut f = fixture();
    let out = f
        .run_with(
            "transact
               let u = insert users { name: n }
               let i = insert identities.Google { user: key u, gid: g }
               return key i",
            &[("n", "Text", text("eve")), ("g", "Text", text("g-eve"))],
        )
        .expect("inserted");
    let i = f.one(r#"from g in identities.Google where g.gid == "g-eve" select key g"#);
    assert_eq!(i, out.result);
    let u = f.one(r#"from u in users where u.name == "eve" select key u"#);
    assert_eq!(
        u,
        f.one(r#"from g in identities.Google where g.gid == "g-eve" select g.user"#)
    );
}

#[test]
fn a_nested_insert_is_part_of_the_change_set() {
    let mut f = fixture();
    f.ok(r#"insert identities.Google { user: key (insert users { name: "nest" }), gid: "g-n" }"#);
    let u = f.one(r#"from u in users where u.name == "nest" select key u"#);
    assert_eq!(
        u,
        f.one(r#"from g in identities.Google where g.gid == "g-n" select g.user"#)
    );
}

#[test]
fn a_sequence_never_reissues_a_key_a_row_has_held() {
    let mut f = fixture();
    f.ok(r#"insert users { id: users.key(42), name: "imported" }"#);
    f.ok("delete u in users where key u == users.key(42)");
    f.ok(r#"insert users { name: "next" }"#);
    let k = f.one(r#"from u in users where u.name == "next" select key u"#);
    assert_ne!(key(42), k);
    assert!(![key(1), key(2), key(3), key(4)].contains(&k));
}

#[test]
fn generated_keys_are_distinct_within_a_statement() {
    let mut f = fixture();
    f.ok(r#"insert users from (from u in users select User { name: u.name ++ "2" })"#);
    assert_eq!(8, count(&f, "users"));
}

#[test]
fn every_clock_fill_of_a_statement_has_one_value() {
    let mut f = fixture();
    f.supply.clock = 777;
    f.ok("update s in logins where s.user == users.key(1) set Revoked { }");
    assert_eq!(
        set([Value::Instant(777)]),
        f.value("from s in logins.Revoked where s.user == users.key(1) select s.at")
    );
    assert_eq!(
        2,
        f.value("from s in logins.Revoked where s.user == users.key(1) select s")
            .as_set()
            .expect("a set")
            .len()
    );
}

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

#[test]
fn a_rejected_record_without_rejects_into_is_bad_input() {
    let mut f = fixture();
    assert_eq!(
        Rejection::BadInput {
            source: "raws".into(),
            line: 2,
            detail: "negative".into()
        },
        f.rejected(
            r#"insert users from (
                 from r in raws select User { name: if r.n > 0 then "ok" else reject "negative" })"#
        )
    );
}

#[test]
fn a_let_on_an_insert_with_rejects_binds_the_inserted_rows() {
    let mut f = fixture();
    let out = f
        .run(
            r#"transact
                 let x = insert users from (
                   from r in raws select User { name: if r.n > 0 then "ok" else reject "negative" })
                   rejects into user_rejects
                 return count(u for u in x)"#,
        )
        .expect("loaded");
    assert_eq!(int(2), out.result);
}

#[test]
fn a_constraint_violation_rejects_the_statement_with_its_rejects() {
    let mut f = fixture();
    let (name, _) = violated(f.rejected(
        r#"insert users from (
             from r in raws select User { name: if r.n > 0 then "aa" else reject "negative" })
           rejects into user_rejects"#,
    ));
    assert_eq!("app::aa_names", name);
    assert_eq!(0, count(&f, "user_rejects"));
}

// Update and retag

#[test]
fn an_update_of_a_constructor_relation() {
    let mut f = fixture();
    f.run_with(
        "update g in identities.Google where g.user == u set Google { g with gid: g2 }",
        &[("u", "Key users", key(2)), ("g2", "Text", text("g2"))],
    )
    .expect("updated");
    assert_eq!(
        texts(["g2"]),
        f.value("from g in identities.Google where g.user == users.key(2) select g.gid")
    );
}

#[test]
fn an_update_over_a_key_preserving_query() {
    let mut f = fixture();
    f.run_with(
        "update g in (from g in identities.Google, x in imports.Oauth
                     where g.gid == x.sub select g)
         set Google { g with user: u }",
        &[("u", "Key users", key(2))],
    )
    .expect("updated");
    assert_eq!(
        key(2),
        f.one(r#"from g in identities.Google where g.gid == "s-bo" select g.user"#)
    );
}

#[test]
fn an_update_rebuilds_a_value_level_sum() {
    let mut f = fixture();
    f.run_with(
        r#"update c in customers where c.id == k
           set Customer { c with address: match c.address
                                            | US a   -> US { a with zip: "02139" }
                                            | Intl i -> i }"#,
        &[("k", "Key customers", Value::key(int(1)))],
    )
    .expect("updated");
    assert_eq!(
        texts(["02139"]),
        f.value(
            "from c in customers where c.id == customers.key(1)
             select match c.address | US a -> a.zip | Intl i -> i.country"
        )
    );
}

#[test]
fn an_update_to_another_key_is_key_changed() {
    let mut f = fixture();
    let other = Value::Ctor(0, vec![("id".into(), key(2)), ("name".into(), text("x"))]);
    let before = f.db.clone();
    assert_eq!(
        Err(Abort::Reject(Rejection::KeyChanged {
            relation: "users".into()
        })),
        f.run_with(
            "update u in users where key u == users.key(1) set v",
            &[("v", "User", other)]
        )
        .map(|o| o.result)
    );
    assert_eq!(before, f.db);
}

#[test]
fn a_retag_moves_the_row_between_constructor_relations_under_its_key() {
    let mut f = fixture();
    let out = f
        .run(&format!(
            "update i in identities where i.id == {}
             set match i
               | EmailPass e -> Google {{ gid: lookup(e.email) }}
               | Google g    -> g
               | Apple a     -> a",
            uuid(5)
        ))
        .expect("retagged");
    let ids = f.rel("identities");
    assert!(out.change.deleted.contains_key(&(ids, 0)));
    assert!(out.change.inserted.contains_key(&(ids, 1)));
    assert_eq!(
        f.value(&format!("[{}]", uuid(5))),
        f.value(r#"from g in identities.Google where g.gid == "bo@example.com" select key g"#)
    );
    assert_eq!(
        key(2),
        f.one(r#"from g in identities.Google where g.gid == "bo@example.com" select g.user"#)
    );
}

#[test]
fn a_retag_fills_omitted_fields_from_the_relation_whatever_the_old_constructor() {
    let mut f = fixture();
    f.supply.clock = 99;
    f.ok("update s in logins where s.user == users.key(1) set Revoked { }");
    assert_eq!(
        set([Value::Instant(99)]),
        f.value("from s in logins.Revoked where s.user == users.key(1) select s.at")
    );
}

#[test]
fn a_retag_is_refused_while_a_reference_to_the_old_constructor_remains() {
    let mut f = fixture();
    let (name, report) = violated(f.rejected(&format!(
        r#"update i in identities.EmailPass where i.id == {} set Google {{ gid: "g" }}"#,
        uuid(1)
    )));
    assert_eq!("app::resets.Reset.identity", name);
    assert_eq!(1, report.len());
}

#[test]
fn a_retag_succeeds_when_the_statement_deletes_the_referrers() {
    let mut f = fixture();
    f.ok(&format!(
        r#"transact
             delete r in resets where r.identity == {k}
             update i in identities.EmailPass where i.id == {k} set Google {{ gid: "g" }}"#,
        k = uuid(1)
    ));
    assert_eq!(0, count(&f, "resets"));
    assert_eq!(
        texts(["Active"]),
        f.value("from s in logins where s.id == logins.key(1) select match s | Active a -> \"Active\" | Revoked r -> \"Revoked\"")
    );
}

#[test]
fn a_let_on_an_update_binds_the_new_rows() {
    let mut f = fixture();
    let out = f
        .run(
            r#"transact
                 let x = update u in users where key u == users.key(1) set User { u with name: "z" }
                 return from y in x select y.name"#,
        )
        .expect("updated");
    assert_eq!(texts(["z"]), out.result);
}

// Delete

#[test]
fn a_delete_over_a_constructor_relation() {
    let mut f = fixture();
    f.run_with(
        "delete s in logins.Revoked where s.user == u",
        &[("u", "Key users", key(1))],
    )
    .expect("deleted");
    assert_eq!(2, count(&f, "logins"));
}

#[test]
fn a_delete_over_a_key_preserving_query() {
    let mut f = fixture();
    f.run_with(
        "delete s in (from s in logins.Active, i in identities.Google
                      where s.identity == key i and i.gid == g select s)",
        &[("g", "Text", text("g-bo"))],
    )
    .expect("deleted");
    assert_eq!(
        keys_of(&f, "from s in logins select key s"),
        BTreeSet::from([1, 3])
    );
}

fn keys_of(f: &Fixture, q: &str) -> BTreeSet<i64> {
    f.value(q)
        .as_set()
        .expect("a set")
        .iter()
        .map(|k| match k {
            Value::Key(v) => match v.as_ref() {
                Value::Int(i) => *i,
                v => panic!("{v:?}"),
            },
            v => panic!("{v:?}"),
        })
        .collect()
}

#[test]
fn a_let_on_a_delete_binds_the_rows_it_names() {
    let mut f = fixture();
    let out = f
        .run(
            r#"transact
                 let x = delete n in notes where n.body == "plain"
                 return from y in x select y.body"#,
        )
        .expect("deleted");
    assert_eq!(texts(["plain"]), out.result);
    assert_eq!(2, count(&f, "notes"));
}

// Delete actions

#[test]
fn deleting_a_target_cascades_and_retags_its_referrers() {
    let mut f = fixture();
    f.supply.clock = 1234;
    f.ok(&format!("delete i in identities where i.id == {}", uuid(1)));
    assert_eq!(0, count(&f, "resets"));
    assert_eq!(
        set([Value::Instant(1234)]),
        f.value("from s in logins.Revoked where s.id == logins.key(1) select s.at")
    );
    assert_eq!(
        key(1),
        f.one("from s in logins.Revoked where s.id == logins.key(1) select s.user")
    );
    assert_eq!(3, count(&f, "logins"));
}

#[test]
fn restrict_names_the_least_reference_and_its_referrers() {
    let mut f = fixture();
    let (name, report) = violated(f.rejected("delete x in users where key x == users.key(2)"));
    assert_eq!("app::identities.user", name);
    assert_eq!(2, report.len());
}

#[test]
fn deleting_the_referrers_with_the_target_succeeds() {
    let mut f = fixture();
    f.run_with(
        "transact
           delete d in badges where d.owner == u
           delete x in users where key x == u",
        &[("u", "Key users", key(4))],
    )
    .expect("deleted");
    assert_eq!(3, count(&f, "users"));
    assert_eq!(
        set([Value::none()]),
        f.value(r#"from n in notes where n.body == "n" select n.author"#)
    );
}

#[test]
fn deleting_only_the_target_of_a_restrict_fails() {
    let mut f = fixture();
    let (name, report) = violated(f.rejected("delete x in users where key x == users.key(4)"));
    assert_eq!("app::badges.Badge.owner", name);
    assert_eq!(
        texts(["b4"]),
        Value::set(
            report
                .iter()
                .map(|b| b.field("label").expect("a label").clone())
        )
    );
}

#[test]
fn a_cascaded_row_is_deleted_whatever_other_action_reaches_it() {
    let mut f = fixture();
    f.ok("transact
            delete i in identities where i.user == users.key(1)
            delete t in tickets
            delete x in users where key x == users.key(1)");
    assert_eq!(
        keys_of(&f, "from s in logins select key s"),
        BTreeSet::from([2])
    );
    assert_eq!(0, count(&f, "resets"));
}

#[test]
fn a_reference_repointed_in_the_statement_is_judged_by_its_new_value() {
    let mut f = fixture();
    f.ok(&format!(
        "transact
           update s in logins.Active where s.id == logins.key(1) set Active {{ s with identity: {} }}
           delete i in identities where i.id == {}",
        uuid(2),
        uuid(1)
    ));
    assert_eq!(
        f.value(&format!("[{}]", uuid(2))),
        f.value("from s in logins.Active where s.id == logins.key(1) select s.identity")
    );
}

#[test]
fn an_inserted_row_referencing_a_gone_key_violates_its_reference() {
    let mut f = fixture();
    let (name, report) = violated(f.rejected(
        r#"transact
             insert notes { author: Some { value: users.key(4) }, body: "new" }
             delete d in badges where d.owner == users.key(4)
             delete t in tickets
             delete x in users where key x == users.key(4)"#,
    ));
    assert_eq!("app::notes.Note.author", name);
    assert_eq!(
        texts(["new"]),
        Value::set(
            report
                .iter()
                .map(|n| n.field("body").expect("a body").clone())
        )
    );
}

#[test]
fn a_retag_and_a_clear_on_one_row_give_it_one_new_value() {
    let mut f = fixture();
    f.ok(&format!(
        "transact
           delete d in badges where d.owner == users.key(4)
           delete i in identities where i.id == {}
           delete x in users where key x == users.key(4)",
        uuid(2)
    ));
    let t = f.one("from t in tickets select t");
    assert_eq!(Some(1), t.tag());
    assert_eq!(Some(&Value::none()), t.field("owner"));
}

#[test]
fn a_retag_of_the_target_fires_no_action() {
    let mut f = fixture();
    f.ok(&format!(
        r#"update i in identities.Google where i.id == {} set Apple {{ sub: "a" }}"#,
        uuid(3)
    ));
    assert_eq!(
        texts(["Active"]),
        f.value("from s in logins where s.id == logins.key(2) select match s | Active a -> \"Active\" | Revoked r -> \"Revoked\"")
    );
}

// Upsert

#[test]
fn an_upsert_on_a_unique_updates_the_conflicting_row() {
    let mut f = fixture();
    f.run_with(
        "insert identities.Google { user: u, gid: g }
         unless conflict on (gid) in Google
         else update old set Google { old with user: u }",
        &[("u", "Key users", key(2)), ("g", "Text", text("g-ann"))],
    )
    .expect("upserted");
    assert_eq!(
        key(2),
        f.one(r#"from g in identities.Google where g.gid == "g-ann" select g.user"#)
    );
    assert_eq!(5, count(&f, "identities"));
}

#[test]
fn an_upsert_with_no_conflict_inserts() {
    let mut f = fixture();
    f.run_with(
        "insert identities.Google { user: u, gid: g }
         unless conflict on (gid) in Google
         else update old set Google { old with user: u }",
        &[("u", "Key users", key(2)), ("g", "Text", text("fresh"))],
    )
    .expect("upserted");
    assert_eq!(6, count(&f, "identities"));
}

const ON_KEY: &str = r#"insert identities.Google { id: k, user: u, gid: g }
unless conflict on key
else match old
  | Google o    -> update o set Google { o with gid: g }
  | EmailPass e -> fail "identity is email"
  | Apple a     -> fail "identity is apple""#;

fn identity_key(n: u8) -> Value {
    let f = Fixture::new(SCHEMA);
    f.one(&format!("[{}]", uuid(n)))
}

#[test]
fn an_upsert_on_the_key_matches_the_old_row() {
    let mut f = fixture();
    f.run_with(
        ON_KEY,
        &[
            ("k", "Key identities", identity_key(2)),
            ("u", "Key users", key(1)),
            ("g", "Text", text("g-new")),
        ],
    )
    .expect("upserted");
    assert_eq!(
        set([identity_key(2)]),
        f.value(r#"from g in identities.Google where g.gid == "g-new" select key g"#)
    );
    let before = f.db.clone();
    assert_eq!(
        Err(Abort::Reject(Rejection::UserError {
            message: "identity is email".into()
        })),
        f.run_with(
            ON_KEY,
            &[
                ("k", "Key identities", identity_key(1)),
                ("u", "Key users", key(1)),
                ("g", "Text", text("g-x")),
            ],
        )
        .map(|o| o.result)
    );
    assert_eq!(before, f.db);
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
    assert!(name.starts_with("app::identities.unique#"), "{name}");
}

#[test]
fn a_bulk_upsert_reads_each_proposed_row() {
    let mut f = fixture();
    f.ok("insert identities.Google from (
            from x in imports.Oauth select { user: x.user, gid: x.sub })
          unless conflict on (gid) in Google
          else update old set Google { old with user: new.user }");
    assert_eq!(
        key(2),
        f.one(r#"from g in identities.Google where g.gid == "s-bo" select g.user"#)
    );
}

const GET_OR_CREATE: &str = "transact
  let g = insert identities.Google { user: u, gid: x }
          unless conflict on (gid) in Google
          else update old set old
  return key g";

#[test]
fn a_get_or_create_returns_the_existing_row() {
    let mut f = fixture();
    let out = f
        .run_with(
            GET_OR_CREATE,
            &[("u", "Key users", key(2)), ("x", "Text", text("g-ann"))],
        )
        .expect("ran");
    assert_eq!(identity_key(2), out.result);
    assert!(out.change.inserted.is_empty());
}

#[test]
fn a_get_or_create_returns_the_created_row() {
    let mut f = fixture();
    let out = f
        .run_with(
            GET_OR_CREATE,
            &[("u", "Key users", key(2)), ("x", "Text", text("g-new"))],
        )
        .expect("ran");
    assert_eq!(
        out.result,
        f.one(r#"from g in identities.Google where g.gid == "g-new" select key g"#)
    );
}

#[test]
fn an_upsert_binding_is_the_new_value_its_branch_writes() {
    let mut f = fixture();
    let out = f
        .run_with(
            r#"transact
                 let b = insert identities.Google { id: k, user: users.key(2), gid: "gg" }
                         unless conflict on key
                         else match old
                           | Google o    -> update o set o
                           | EmailPass e -> update e set Google { gid: "from-email" }
                           | Apple a     -> update a set a
                 return match b | Google g -> g.gid | Apple a -> a.sub"#,
            &[("k", "Key identities", identity_key(5))],
        )
        .expect("ran");
    assert_eq!(text("from-email"), out.result);
}

#[test]
fn two_proposed_rows_with_one_unique_value_violate_the_unique() {
    let mut f = fixture();
    let (name, _) = violated(f.rejected(
        r#"insert identities.Google from (
             from p in [{ u: users.key(1), g: "dup" }, { u: users.key(2), g: "dup" }]
             select { user: p.u, gid: p.g })
           unless conflict on (gid) in Google
           else update old set old"#,
    ));
    assert_eq!("app::identities.unique(gid) in Google", name);
}

#[test]
fn an_upsert_does_not_see_a_row_another_write_inserts() {
    let mut f = fixture();
    let (name, _) = violated(f.rejected(
        r#"(insert identities.Google { user: users.key(1), gid: "z" },
            insert identities.Google { user: users.key(2), gid: "z" }
            unless conflict on (gid) in Google
            else update old set old)"#,
    ));
    assert_eq!("app::identities.unique(gid) in Google", name);
}

#[test]
fn an_upsert_on_the_key_that_meets_another_insert_is_a_key_violation() {
    let mut f = fixture();
    assert_eq!(
        key_violation("users", "users.key(9)"),
        f.rejected(
            r#"(insert users { id: users.key(9), name: "a" },
                insert users { id: users.key(9), name: "b" }
                unless conflict on key
                else update old set old)"#
        )
    );
}

// The statement boundary

#[test]
fn a_constraint_violation_reports_its_rows() {
    let mut f = fixture();
    let (name, report) =
        violated(f.rejected(r#"insert identities.Google { user: users.key(3), gid: "" }"#));
    assert_eq!("app::login_present", name);
    assert_eq!(1, report.len());
}

#[test]
fn the_violation_reported_is_the_least_name() {
    let mut f = fixture();
    let (name, _) = violated(f.rejected(
        r#"(insert identities.Google { user: users.key(3), gid: "" },
            insert identities.Google { user: users.key(4), gid: "" })"#,
    ));
    assert_eq!("app::identities.unique(gid) in Google", name);
    let (name, _) = violated(f.rejected(r#"insert users { id: users.key(1), name: "aa" }"#));
    assert_eq!("app::aa_names", name);
    assert_eq!(
        key_violation("users", "users.key(1)"),
        f.rejected(r#"insert users { id: users.key(1), name: "zz" }"#)
    );
}

#[test]
fn a_row_check_is_a_constraint() {
    let mut f = fixture();
    let (name, _) = violated(f.rejected(r#"insert users { name: "a name longer than twenty" }"#));
    assert_eq!("app::name_short", name);
}

#[test]
fn a_transition_constraint_reads_the_change() {
    let mut f = fixture();
    let (name, _) = violated(f.rejected(r#"delete n in notes where n.body == "frozen""#));
    assert_eq!("app::frozen_notes", name);
    f.ok(r#"delete n in notes where n.body == "plain""#);
}

#[test]
fn a_domain_is_checked_on_stored_values() {
    let mut f = fixture();
    let (name, _) = violated(f.rejected(r#"insert contacts { email: Email("ab") }"#));
    assert_eq!("app::Email.domain", name);
    f.ok(r#"insert contacts { email: Email("abcd") }"#);
    assert_eq!(text("ab"), f.value(r#"unwrap(Email("ab"))"#));
}

#[test]
fn a_fault_in_a_constraint_takes_precedence_over_every_violation() {
    let mut f = fixture();
    assert_eq!(
        Err(Abort::Fault(Fault::DivisionByZero)),
        f.run(r#"(insert raws { n: 0 }, insert users { id: users.key(1), name: "ann" })"#)
            .map(|o| o.result)
    );
}

#[test]
fn a_constructor_not_open_refuses_inserts_and_retags() {
    let mut f = fixture();
    f.ok(r#"insert identities.Apple { user: users.key(3), sub: "a-cy" }"#);
    f.closed
        .insert(f.ctor("Identity", "Apple"), Stage::Declared);
    assert_eq!(
        Rejection::NotOpen {
            ctor: "identities.Apple".into(),
            stage: Stage::Declared
        },
        f.rejected(r#"insert identities.Apple { user: users.key(4), sub: "a-dee" }"#)
    );
    assert!(matches!(
        f.rejected(&format!(
            r#"update i in identities.Google where i.id == {} set Apple {{ sub: "x" }}"#,
            uuid(3)
        )),
        Rejection::NotOpen { .. }
    ));
    f.ok(r#"update a in identities.Apple set Apple { a with sub: "a-cy2" }"#);
}

#[test]
fn a_value_level_constructor_not_open_refuses_new_values_only() {
    let mut f = fixture();
    f.closed
        .insert(f.ctor("Address", "Intl"), Stage::Deprecated);
    assert_eq!(
        Rejection::NotOpen {
            ctor: "Address.Intl".into(),
            stage: Stage::Deprecated
        },
        f.rejected(
            r#"update c in customers where c.id == customers.key(1)
               set Customer { c with address: Intl { country: "de", postcode: "1" } }"#
        )
    );
    f.ok(
        r#"update c in customers where c.id == customers.key(2) set Customer { c with name: "e" }"#,
    );
}

// Blocks and transactions

#[test]
fn a_block_without_return_returns_unit() {
    let mut f = fixture();
    assert_eq!(
        Value::unit(),
        f.ok(r#"transact insert users { name: "eve" }"#)
    );
}

#[test]
fn a_block_result_reads_the_snapshot() {
    let mut f = fixture();
    assert_eq!(
        int(4),
        f.ok(r#"transact insert users { name: "eve" } return count(u for u in users)"#)
    );
}

#[test]
fn a_block_with_options_runs_once() {
    let mut f = fixture();
    assert_eq!(
        int(4),
        f.ok("transact retry 3 read_only return count(u for u in users)")
    );
    assert_eq!(
        Value::unit(),
        f.ok(r#"transact durability quorum insert users { name: "q" }"#)
    );
    assert_eq!(5, count(&f, "users"));
}

#[test]
fn a_query_statement_changes_nothing() {
    let mut f = fixture();
    let out = f.run("count(u for u in users)").expect("ran");
    assert_eq!(int(4), out.result);
    assert_eq!(f.db, out.db);
}

#[test]
fn a_registered_transaction_runs_as_one_statement() {
    let mut f = fixture();
    let out = f
        .call_txn(
            "txn add_user (n: Text) -> Key users = transact
               let u = insert users { name: n }
               return key u",
            vec![text("eve")],
        )
        .expect("ran");
    assert_eq!(
        out.result,
        f.one(r#"from u in users where u.name == "eve" select key u"#)
    );
}

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
    assert_eq!("app::invoices.unique(status.Paid.ref) in Invoice", name);
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
    assert_eq!(int(11), f.one(r#"from i in invoices select i.n"#));
    f.ok(r#"insert invoices { status: Draft, n: 5 }
            unless conflict on (status.Paid.ref)
            else update old set Invoice { old with n: 0 }"#);
    assert_eq!(2, count(&f, "invoices"));
}
