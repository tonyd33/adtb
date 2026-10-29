use super::*;
use crate::kernel::{Conflict, Durability};
use crate::schema::Generator;

const WRITES: &str = r#"
type Login { id: Key logins, user: Key users on delete cascade }
  = Active  { identity: Key identities on delete retag Revoked {} }
  | Revoked { at: Instant }
relation logins : Login
  key id generate sequence
  default Revoked.at = generate clock
type Badge = { id: Key badges, owner: Key users on delete restrict, label: Text }
relation badges : Badge key id generate sequence
type Import { id: Key imports } = Email { user: Key users, addr: Text, h: Bytes }
                                | Oauth { user: Key users, sub: Text }
relation imports : Import key id generate sequence
type Note = { id: Key notes, author: Option (Key users) on delete clear, body: Text }
relation notes : Note key id generate sequence
relation google_rejects : RejectedRow key (load, line)
type Address = US { zip: Text, street: Text } | Intl { country: Text, postcode: Text }
type Customer = { id: Key customers, name: Text, address: Address }
relation customers : Customer key id generate sequence
type Counter = { id: Key counters, n: I64, label: Text }
relation counters : Counter key id generate sequence
  default n = 0
  default label = "x"
type Shape { id: Key shapes } = Circle { r: I64 } | Ring { r: I64, w: I64 }
relation shapes : Shape key id generate sequence
type Label = { id: Key tags, name: Text }
relation tags : Label key id generate sequence unique (lower(name))
fn lookup (email: Text) -> Text = lower(email)
fn rehash (h: Bytes) -> Bytes = h
"#;

fn wschema() -> Schema {
    schema(WRITES).expect("the write schema builds")
}

/// Checks ad hoc `text` against the running schema and `WRITES`.
fn run(params: &[(&str, &str)], text: &str) -> Result<(Schema, Adhoc), Vec<String>> {
    let s = wschema();
    let mut tys = BTreeMap::new();
    {
        let c = Checker::new(&s, Visible::all(&s), Context::Adhoc);
        for (name, t) in params {
            let parsed = parse_type(t, Options::default()).ok().expect("a type");
            tys.insert(
                name.to_string(),
                c.resolve_type(&parsed).expect("a known type"),
            );
        }
    }
    let parsed = parse_adhoc(text, Options::default());
    if !parsed.diags.is_empty() {
        panic!(
            "{text}\ndoes not parse: {:?}",
            parsed
                .diags
                .iter()
                .map(|d| d.render(text))
                .collect::<Vec<_>>()
        );
    }
    let items = parsed.ok().expect("parsed");
    let program = check_schema(&s).expect("the schema checks");
    match check_adhoc(&s, &program, &items, &tys) {
        Ok(a) => {
            adhoc_one_binding_site(&s, &program, &a);
            Ok((s, a))
        }
        Err(ds) => Err(ds.into_iter().map(|d| d.message).collect()),
    }
}

fn ok(params: &[(&str, &str)], text: &str) -> (Schema, Adhoc) {
    match run(params, text) {
        Ok(r) => r,
        Err(errs) => panic!("{text}\nrefused: {errs:?}"),
    }
}

fn no(params: &[(&str, &str)], text: &str, needle: &str) {
    match run(params, text) {
        Ok(_) => panic!("{text}\naccepted; expected an error containing {needle:?}"),
        Err(errs) => assert!(
            errs.iter().any(|e| e.contains(needle)),
            "{text}\nno error containing {needle:?} in {errs:?}"
        ),
    }
}

/// The type of the single item of ad hoc text.
fn ty_of(params: &[(&str, &str)], text: &str) -> String {
    let (s, a) = ok(params, text);
    s.show_ty(a.terms.ty(a.items[0]))
}

fn txn(text: &str) -> Result<(Schema, Txn), String> {
    let s = wschema();
    let items = parse_example(text, Options::default())
        .ok()
        .expect("parses");
    let Some(ExampleItem::BundleItem(BundleItem::Txn(t))) = items.into_iter().next() else {
        panic!("{text}\nis not a txn");
    };
    let module = s.module_named("auth").expect("module auth");
    let program = check_schema(&s).expect("the schema checks");
    match check_txn(&s, &program, module, &t) {
        Ok(t) => Ok((s, t)),
        Err(d) => Err(d.message),
    }
}

fn txn_ok(text: &str) {
    if let Err(e) = txn(text) {
        panic!("{text}\nrefused: {e}");
    }
}

fn txn_no(text: &str, needle: &str) {
    match txn(text) {
        Ok(_) => panic!("{text}\naccepted; expected an error containing {needle:?}"),
        Err(e) => assert!(e.contains(needle), "{text}\nerror {e:?} lacks {needle:?}"),
    }
}

fn schema_no(extra: &str, needle: &str) {
    refuses(&format!("{WRITES}\n{extra}"), needle);
}

/// The named field of a constructor term.
fn field(terms: &Terms, t: Tm, name: &str) -> Tm {
    let Kind::Ctor { fields, .. } = terms.kind(t) else {
        panic!("a constructor, found {:?}", terms.kind(t));
    };
    fields
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, t)| *t)
        .unwrap_or_else(|| panic!("no field `{name}` in {fields:?}"))
}

const NG: &[(&str, &str)] = &[("n", "Text"), ("g", "Text"), ("u", "Key users")];

// Inserts

#[test]
fn an_insert_fills_a_generated_key() {
    let (s, a) = ok(NG, "insert users { name: n }");
    let Kind::Insert(rel, rows) = a.terms.kind(a.items[0]) else {
        panic!("an insert, found {:?}", a.terms.kind(a.items[0]));
    };
    assert_eq!(*rel, rel_id(&s, "users"));
    let Kind::Singleton(row) = a.terms.kind(*rows) else {
        panic!("one row");
    };
    let id = field(&a.terms, *row, "id");
    assert_eq!(
        a.terms.kind(id),
        &Kind::Fresh(Generator::Sequence, rel_id(&s, "users"))
    );
    assert_eq!(s.show_ty(a.terms.ty(a.items[0])), "a write");
}

#[test]
fn an_insert_into_a_constructor_relation_fills_its_header_key() {
    let (s, a) = ok(NG, "insert identities.Google { user: u, gid: g }");
    let Kind::Insert(_, rows) = a.terms.kind(a.items[0]) else {
        panic!("an insert");
    };
    let Kind::Singleton(row) = a.terms.kind(*rows) else {
        panic!("one row");
    };
    assert!(matches!(a.terms.kind(*row), Kind::Ctor { ctor: 1, .. }));
    assert_eq!(
        a.terms.kind(field(&a.terms, *row, "id")),
        &Kind::Fresh(Generator::Uuidv7, rel_id(&s, "identities"))
    );
}

#[test]
fn a_generated_field_may_be_given() {
    ok(NG, "insert users { id: users.key(42), name: n }");
}

#[test]
fn inserts_from_a_query_take_records_or_constructor_values() {
    ok(
        &[],
        "insert identities.Google from (
           from x in imports.Oauth select { user: x.user, gid: x.sub })",
    );
    ok(
        &[],
        "insert identities from (
           from x in imports
           select match x
             | Email e -> EmailPass { user: e.user, email: e.addr, hash: e.h }
             | Oauth o -> Google { user: o.user, gid: o.sub })",
    );
    ok(
        &[],
        "insert identities.OAuth from (
           from x in imports.Oauth select Google { user: x.user, gid: x.sub })",
    );
}

#[test]
fn an_insert_into_a_constructor_set_refuses_another_constructor() {
    no(
        &[],
        "insert identities.OAuth from (
           from x in imports.Email select EmailPass { user: x.user, email: x.addr, hash: x.h })",
        "expected",
    );
    no(
        &[],
        "insert identities.Google from (
           from x in imports.Email select EmailPass { user: x.user, email: x.addr, hash: x.h })",
        "expected",
    );
}

#[test]
fn a_single_row_goes_into_one_constructor() {
    no(
        NG,
        "insert identities { user: u, gid: g }",
        "a single row is inserted into one constructor",
    );
    no(
        NG,
        "insert identities Google { user: u, gid: g }",
        "a single row is inserted into one constructor",
    );
    no(
        &[],
        "insert identities from (from x in imports.Oauth select { user: x.user, gid: x.sub })",
        "expected",
    );
}

#[test]
fn an_insert_gives_every_field_it_has_no_default_for() {
    no(
        &[("u", "Key users")],
        "insert identities.Google { user: u }",
        "field `gid` of `Google` is missing",
    );
    no(
        NG,
        "insert identities.Google { user: u, gid: g, email: n }",
        "has no field `email`",
    );
}

#[test]
fn defaults_are_per_constructor() {
    let (s, a) = ok(&[("u", "Key users")], "insert logins.Revoked { user: u }");
    let Kind::Insert(_, rows) = a.terms.kind(a.items[0]) else {
        panic!("an insert");
    };
    let Kind::Singleton(row) = a.terms.kind(*rows) else {
        panic!("one row");
    };
    assert_eq!(
        a.terms.kind(field(&a.terms, *row, "at")),
        &Kind::Fresh(Generator::Clock, rel_id(&s, "logins"))
    );
    no(
        &[("u", "Key users")],
        "insert logins.Active { user: u }",
        "field `identity` of `Active` is missing",
    );
}

#[test]
fn a_default_expression_is_elaborated_into_the_row() {
    let (_, a) = ok(&[], "insert counters { }");
    let Kind::Insert(_, rows) = a.terms.kind(a.items[0]) else {
        panic!("an insert");
    };
    let Kind::Singleton(row) = a.terms.kind(*rows) else {
        panic!("one row");
    };
    assert_eq!(
        a.terms.kind(field(&a.terms, *row, "n")),
        &Kind::Const(Const::Int(0))
    );
    assert_eq!(
        a.terms.kind(field(&a.terms, *row, "label")),
        &Kind::Const(Const::Text("x".into()))
    );
}

#[test]
fn defaults_reach_the_select_of_an_insert_query_and_its_match_arms() {
    ok(
        &[],
        "insert logins from (
           from x in users select match 1 | 1 -> Revoked { user: key x } | _ -> Revoked { user: key x })",
    );
}

#[test]
fn a_value_built_elsewhere_gives_every_field() {
    refuses(
        &format!("{WRITES}\nfn f () -> Set User = from u in users select User {{ name: u.name }}"),
        "field `id` of `User` is missing",
    );
}

// Updates

#[test]
fn an_update_replaces_rows_by_a_function_of_each() {
    let (s, a) = ok(
        &[("u", "Key users"), ("g2", "Text")],
        "update g in identities.Google where g.user == u set Google { g with gid: g2 }",
    );
    let Kind::Replace(rel, rows, f) = a.terms.kind(a.items[0]) else {
        panic!("a replace, found {:?}", a.terms.kind(a.items[0]));
    };
    assert_eq!(*rel, rel_id(&s, "identities"));
    assert!(matches!(a.terms.kind(*rows), Kind::For(..)));
    let Kind::Lam(x, body) = a.terms.kind(*f) else {
        panic!("a lambda");
    };
    let Kind::Field(base, _) = a.terms.kind(field(&a.terms, *body, "id")) else {
        panic!("the key is copied");
    };
    assert_eq!(a.terms.kind(*base), &Kind::Var(*x));
}

#[test]
fn update_forms_of_the_spec() {
    ok(
        &[("u", "Key users")],
        "update i in identities where i.user == u
         set match i
           | EmailPass e -> EmailPass { e with hash: rehash(e.hash) }
           | Google g    -> g
           | Apple a     -> a",
    );
    ok(
        &[("u", "Key users")],
        "update g in (from g in identities.Google, x in imports.Oauth
                      where g.gid == x.sub select g)
         set Google { g with user: u }",
    );
    ok(
        &[("k", "Key customers")],
        "update c in customers where c.id == k
         set Customer { c with address: match c.address
                                          | US a   -> US { a with zip: \"02139\" }
                                          | Intl i -> i }",
    );
}

#[test]
fn a_retag_keeps_the_key_and_header_and_fills_own_fields() {
    let (_, a) = ok(
        &[("k", "Key identities")],
        "update i in identities where i.id == k
         set match i
           | EmailPass e -> Google { gid: lookup(e.email) }
           | Google g    -> g
           | Apple a     -> a",
    );
    assert!(matches!(a.terms.kind(a.items[0]), Kind::Replace(..)));
    let (s, a) = ok(
        &[("u", "Key users")],
        "update s in logins where s.user == u set Revoked { }",
    );
    let Kind::Replace(_, _, f) = a.terms.kind(a.items[0]) else {
        panic!("a replace");
    };
    let Kind::Lam(x, body) = a.terms.kind(*f) else {
        panic!("a lambda");
    };
    assert_eq!(
        a.terms.kind(field(&a.terms, *body, "at")),
        &Kind::Fresh(Generator::Clock, rel_id(&s, "logins"))
    );
    for f in ["id", "user"] {
        let Kind::Field(base, _) = a.terms.kind(field(&a.terms, *body, f)) else {
            panic!("`{f}` is copied");
        };
        assert_eq!(a.terms.kind(*base), &Kind::Var(*x));
    }
}

#[test]
fn an_update_never_gives_a_key_field() {
    no(
        &[("u", "Key users"), ("k", "Key identities")],
        "update g in identities.Google where g.user == u set Google { g with id: k }",
        "keeps the row's key",
    );
    no(
        &[("k", "Key identities")],
        "update g in identities.Google set Google { id: k, gid: \"x\" }",
        "keeps the row's key",
    );
    no(
        &[("k", "Key users")],
        "update x in users set User { id: k, name: \"x\" }",
        "keeps the row's key",
    );
}

#[test]
fn an_updated_own_field_without_default_is_given() {
    no(
        &[("k", "Key identities")],
        "update i in identities where i.id == k set Apple { }",
        "field `sub` of `Apple` is missing",
    );
    ok(&[], "update x in users set User { name: \"x\" }");
}

#[test]
fn with_copies_only_what_the_old_value_has() {
    ok(
        &[],
        "update g in identities.Google set Google { g with user: g.user }",
    );
    no(
        &[],
        "update i in identities set Google { i with user: i.user }",
        "field `gid` of `Google` is missing",
    );
    no(
        &[],
        "update s in shapes.Circle set Ring { s with w: 1 }",
        "field `r` of `Ring` is missing",
    );
    ok(&[], "update s in shapes.Ring set Ring { s with w: 1 }");
}

#[test]
fn an_update_ranges_over_rows_of_one_relation() {
    no(
        &[],
        "update x in (from g in identities.Google select g.gid) set x",
        "ranges over rows of one relation variable",
    );
    no(
        &[],
        "delete g in (from u in users group u by u.name into g select g)",
        "ranges over rows of one relation variable",
    );
}

#[test]
fn the_new_value_is_a_value_of_the_relations_sum() {
    no(
        &[],
        "update g in identities.Google set User { id: users.key(1), name: \"x\" }",
        "expected Identity",
    );
}

#[test]
fn update_and_delete_name_snapshot_rows() {
    no(
        &[("n", "Text"), ("m", "Text")],
        "transact
           let u = insert users { name: n }
           update u set User { u with name: m }",
        "is not a row of the snapshot",
    );
    no(
        &[("n", "Text")],
        "transact
           let u = insert users { name: n }
           delete u",
        "is not a row of the snapshot",
    );
    no(
        &[("p", "User")],
        "update p set p",
        "is not a row of the snapshot",
    );
    no(&[("p", "User")], "delete p", "is not a row of the snapshot");
    no(
        NG,
        "insert identities.Google { user: u, gid: g }
         unless conflict on (gid) in Google
         else update new set new",
        "is not a row of the snapshot",
    );
    no(
        &[],
        "transact
           let us = from x in users select x
           delete us",
        "is not a row of the snapshot",
    );
}

#[test]
fn update_x_where_p_updates_when_p_holds() {
    let (_, a) = ok(
        &[],
        "insert users { name: \"a\" } unless conflict on key else update old where old.name == \"a\" set old",
    );
    let Kind::Upsert { otherwise, .. } = a.terms.kind(a.items[0]) else {
        panic!("an upsert");
    };
    let Kind::Replace(_, rows, _) = a.terms.kind(*otherwise) else {
        panic!("a replace, found {:?}", a.terms.kind(*otherwise));
    };
    assert!(matches!(a.terms.kind(*rows), Kind::If(..)));
    let (_, a) = ok(
        &[],
        "insert users { name: \"a\" } unless conflict on key else delete old where old.name == \"a\"",
    );
    let Kind::Upsert { otherwise, .. } = a.terms.kind(a.items[0]) else {
        panic!("an upsert");
    };
    let Kind::Delete(_, rows) = a.terms.kind(*otherwise) else {
        panic!("a delete, found {:?}", a.terms.kind(*otherwise));
    };
    assert!(matches!(a.terms.kind(*rows), Kind::If(..)));
}

#[test]
fn update_x_where_p_reads_the_row_in_p() {
    let (_, a) = ok(
        &[],
        "insert users { name: \"a\" } unless conflict on key else update old where old.name == \"a\" set old",
    );
    let Kind::Upsert { otherwise, .. } = a.terms.kind(a.items[0]) else {
        panic!("an upsert");
    };
    let Kind::Replace(_, rows, f) = a.terms.kind(*otherwise) else {
        panic!("a replace, found {:?}", a.terms.kind(*otherwise));
    };
    let Kind::If(p, ..) = a.terms.kind(*rows) else {
        panic!("an if, found {:?}", a.terms.kind(*rows));
    };
    let Kind::Lam(x, _) = a.terms.kind(*f) else {
        panic!("a lam, found {:?}", a.terms.kind(*f));
    };
    assert!(!mentions_var(&a.terms, *p, *x));
}

fn mentions_var(terms: &Terms, t: Tm, v: Var) -> bool {
    matches!(terms.kind(t), Kind::Var(w) if *w == v)
        || terms
            .children(t)
            .into_iter()
            .any(|c| mentions_var(terms, c, v))
}

#[test]
fn a_let_bound_to_a_match_over_a_key_is_a_snapshot_row() {
    ok(
        &[("k", "Key identities")],
        "transact
           let r = match k | EmailPass e -> e | Google g -> g | Apple a -> a
           update r set match r
             | EmailPass e -> e
             | Google g    -> Google { g with gid: \"x\" }
             | Apple a     -> a",
    );
}

#[test]
fn rows_read_from_the_snapshot_are_snapshot_rows() {
    ok(
        &[("k", "Key users")],
        "match k | User v -> update v set User { v with name: \"b\" }",
    );
    ok(
        &[("k", "Key users")],
        "transact
           let r = match k | User v -> v
           update r set User { r with name: \"b\" }",
    );
    ok(
        &[("k", "Key users")],
        "transact
           let r = match k | User v -> v
           delete r",
    );
    no(
        &[("n", "Text")],
        "transact
           let u = insert users { name: n }
           let r = match u | User v -> v
           delete r",
        "is not a row of the snapshot",
    );
    no(
        &[("n", "Text")],
        "transact
           let u = insert users { name: n }
           match u | User v -> delete v",
        "is not a row of the snapshot",
    );
    no(
        &[("u", "Key users"), ("g2", "Text")],
        "transact
           let s = update g in identities.Google where g.user == u set Google { g with gid: g2 }
           delete x in s",
        "a write proposes",
    );
}

// Deletes

#[test]
fn a_delete_names_rows_of_one_relation() {
    let (s, a) = ok(
        &[("u", "Key users")],
        "delete s in logins.Revoked where s.user == u",
    );
    let Kind::Delete(rel, rows) = a.terms.kind(a.items[0]) else {
        panic!("a delete");
    };
    assert_eq!(*rel, rel_id(&s, "logins"));
    assert!(matches!(a.terms.kind(*rows), Kind::For(..)));
    ok(&[], "delete s in logins.Revoked");
    ok(
        &[("g", "Text")],
        "delete s in (from s in logins.Active, i in identities.Google
                      where s.identity == key i and i.gid == g select s)",
    );
    ok(
        &[("u", "Key users")],
        "transact
           delete d in badges where d.owner == u
           delete x in users where key x == u",
    );
}

// Upserts

#[test]
fn upserts_of_the_spec() {
    let (_, a) = ok(
        NG,
        "insert identities.Google { user: u, gid: g }
         unless conflict on (gid) in Google
         else update old set Google { old with user: u }",
    );
    assert!(matches!(
        a.terms.kind(a.items[0]),
        Kind::Upsert {
            on: Conflict::Unique(_),
            ..
        }
    ));
    let (_, a) = ok(
        &[("u", "Key users"), ("g", "Text"), ("k", "Key identities")],
        "insert identities.Google { id: k, user: u, gid: g }
         unless conflict on key
         else match old
           | Google o    -> update o set Google { o with gid: g }
           | EmailPass e -> fail \"identity is email\"
           | Apple a     -> fail \"identity is apple\"",
    );
    assert!(matches!(
        a.terms.kind(a.items[0]),
        Kind::Upsert {
            on: Conflict::Key,
            ..
        }
    ));
    ok(
        &[],
        "insert identities.Google from (
           from x in imports.Oauth select { user: x.user, gid: x.sub })
         unless conflict on (gid) in Google
         else update old set Google { old with user: new.user }",
    );
    ok(
        &[("n", "Text")],
        "insert tags { name: n } unless conflict on (lower(name)) else update old set old",
    );
}

#[test]
fn an_upsert_names_a_declared_key_or_unique() {
    no(
        NG,
        "insert identities.Google { user: u, gid: g }
         unless conflict on (user) in Google
         else update old set old",
        "names no key or unique",
    );
    no(
        NG,
        "insert identities.Google { user: u, gid: g }
         unless conflict on (gid) in Apple
         else update old set old",
        "names no key or unique",
    );
}

#[test]
fn an_upsert_target_lies_within_the_unique() {
    no(
        &[("u", "Key users"), ("e", "Text"), ("h", "Bytes")],
        "insert identities.EmailPass { user: u, email: e, hash: h }
         unless conflict on (gid) in Google
         else update old set old",
        "lie outside",
    );
    no(
        &[],
        "insert identities from (from x in imports.Oauth select Google { user: x.user, gid: x.sub })
         unless conflict on (gid) in Google
         else update old set old",
        "lie outside",
    );
}

#[test]
fn old_has_the_row_type_of_the_conflict_scope() {
    ok(
        NG,
        "insert identities.Google { user: u, gid: g }
         unless conflict on (gid) in Google
         else match old | Google o -> update o set o",
    );
    no(
        &[("u", "Key users"), ("g", "Text"), ("k", "Key identities")],
        "transact
           insert identities.Google { id: k, user: u, gid: g }
           unless conflict on key
           else match old | Google o -> update o set o",
        "does not cover",
    );
}

#[test]
fn an_upsert_else_branch_may_delete_old_without_let() {
    ok(
        NG,
        "insert identities.Google { user: u, gid: g }
         unless conflict on (gid) in Google
         else delete old",
    );
}

#[test]
fn a_let_on_an_upsert_binds_one_row_of_the_joined_type() {
    assert_eq!(
        ty_of(
            &[("u", "Key users"), ("x", "Text")],
            "transact
               let g = insert identities.Google { user: u, gid: x }
                       unless conflict on (gid) in Google
                       else update old set old
               return key g",
        ),
        "Key identities.Google"
    );
    assert_eq!(
        ty_of(
            &[("u", "Key users"), ("g", "Text"), ("k", "Key identities")],
            "transact
               let r = insert identities.Google { id: k, user: u, gid: g }
                       unless conflict on key
                       else match old
                         | Google o    -> update o set o
                         | EmailPass e -> update e set Apple { sub: g }
                         | Apple a     -> fail \"apple\"
               return r",
        ),
        "Identity.(Google | Apple)"
    );
    assert_eq!(
        ty_of(
            &[],
            "transact
               let r = insert identities.Google from (
                         from x in imports.Oauth select { user: x.user, gid: x.sub })
                       unless conflict on (gid) in Google
                       else update old set Google { old with user: new.user }
               return r",
        ),
        "Set Identity.Google"
    );
}

#[test]
fn a_let_on_an_upsert_that_can_delete_its_row_is_refused() {
    no(
        NG,
        "transact
           let r = insert identities.Google { user: u, gid: g }
                   unless conflict on (gid) in Google
                   else delete old
           return r",
        "can delete it",
    );
    no(
        &[("u", "Key users"), ("g", "Text"), ("k", "Key identities")],
        "transact
           let r = insert identities.Google { id: k, user: u, gid: g }
                   unless conflict on key
                   else match old
                     | Google o    -> update o set o
                     | EmailPass e -> delete e
                     | Apple a     -> fail \"apple\"
           return r",
        "can delete it",
    );
}

#[test]
fn a_let_on_an_upsert_that_can_keep_old_binds_at_its_type_too() {
    assert_eq!(
        ty_of(
            &[("u", "Key users"), ("g", "Text"), ("k", "Key identities")],
            "transact
               let r = insert identities.Google { id: k, user: u, gid: g }
                       unless conflict on key
                       else match old
                         | Google o    -> update o set o
                         | EmailPass e -> skip
                         | Apple a     -> skip
               return r",
        ),
        "Identity"
    );
    assert_eq!(
        ty_of(
            &[("u", "Key users"), ("g", "Text")],
            "transact
               let r = insert identities.Google { user: u, gid: g }
                       unless conflict on (gid) in Google
                       else skip
               return r",
        ),
        "Identity.Google"
    );
}

#[test]
fn a_let_on_an_upsert_whose_branch_can_cascade_into_its_row_is_refused() {
    no(
        &[("u", "Key users"), ("k", "Key logins")],
        "transact
           let r = insert logins.Revoked { id: k, user: u }
                   unless conflict on key
                   else (update old set old, delete x in users where key x == u)
           return r",
        "can delete it",
    );
    ok(
        &[("u", "Key users"), ("k", "Key identities"), ("g", "Text")],
        "transact
           let r = insert identities.Google { id: k, user: u, gid: g }
                   unless conflict on key
                   else (update old set old, delete x in users where key x == u)
           return r",
    );
    ok(
        &[("u", "Key users"), ("k", "Key logins")],
        "transact
           let r = insert logins.Revoked { id: k, user: u }
                   unless conflict on key
                   else (update old set old, delete b in badges where b.owner == u)
           return r",
    );
}

// Nested inserts

#[test]
fn a_nested_insert_is_inserted_beside_its_write() {
    let (s, a) = ok(
        &[("n", "Text"), ("g", "Text")],
        "insert identities.Google { user: key (insert users { name: n }), gid: g }",
    );
    let Kind::Let(v, row, body) = a.terms.kind(a.items[0]) else {
        panic!("a let, found {:?}", a.terms.kind(a.items[0]));
    };
    assert!(matches!(a.terms.kind(*row), Kind::Ctor { .. }));
    let Kind::Par(ws) = a.terms.kind(*body) else {
        panic!("a par");
    };
    let Kind::Insert(rel, rows) = a.terms.kind(ws[0]) else {
        panic!("the nested insert");
    };
    assert_eq!(*rel, rel_id(&s, "users"));
    let Kind::Singleton(x) = a.terms.kind(*rows) else {
        panic!("one row");
    };
    assert_eq!(a.terms.kind(*x), &Kind::Var(*v));
    assert!(matches!(a.terms.kind(ws[1]), Kind::Insert(..)));
}

#[test]
fn a_nested_insert_in_a_block_let_and_return() {
    assert_eq!(
        ty_of(
            &[("n", "Text")],
            "transact
               let k = key (insert users { name: n })
               return k",
        ),
        "Key users"
    );
    assert_eq!(
        ty_of(
            &[("n", "Text")],
            "transact return key (insert users { name: n })"
        ),
        "Key users"
    );
}

#[test]
fn a_nested_insert_is_evaluated_once_per_statement() {
    let once = "evaluated once per statement";
    no(
        &[],
        "insert identities.Google from (
           from x in imports.Oauth select { user: key (insert users { name: x.sub }), gid: x.sub })",
        once,
    );
    no(
        &[],
        "update g in identities.Google set Google { g with user: key (insert users { name: \"a\" }) }",
        once,
    );
    no(
        &[],
        "delete g in identities.Google where g.user == key (insert users { name: \"a\" })",
        once,
    );
    no(
        NG,
        "insert identities.Google { user: u, gid: g }
         unless conflict on (gid) in Google
         else update old set Google { old with user: key (insert users { name: n }) }",
        once,
    );
    no(
        &[("s", "Status"), ("n", "Text"), ("g", "Text")],
        "match s
           | Draft  -> skip
           | Sent x -> insert identities.Google { user: key (insert users { name: n }), gid: g }
           | Paid p -> skip",
        once,
    );
    no(
        &[
            ("c", "Bool"),
            ("n", "Text"),
            ("g", "Text"),
            ("u", "Key users"),
        ],
        "insert identities.Google { user: if c then key (insert users { name: n }) else u, gid: g }",
        once,
    );
}

#[test]
fn a_nested_insert_under_a_write_branch_is_refused() {
    let once = "evaluated once per statement";
    no(
        &[("s", "Status"), ("n", "Text"), ("g", "Text")],
        "match s
           | Draft  -> insert identities.Google { user: key (insert users { name: n }), gid: g }
           | Sent x -> skip
           | Paid p -> skip",
        once,
    );
    no(
        &[("c", "Bool"), ("n", "Text"), ("g", "Text")],
        "if c then insert identities.Google { user: key (insert users { name: n }), gid: g } else skip",
        once,
    );
    no(
        &[("c", "Bool"), ("n", "Text"), ("g", "Text")],
        "if c then skip else insert identities.Google { user: key (insert users { name: n }), gid: g }",
        once,
    );
}

#[test]
fn key_of_a_write_takes_a_single_row_insert() {
    no(
        &[("g", "Text")],
        "insert identities.Google { user: key (insert users from (from x in users select x)), gid: g }",
        "takes a single-row insert",
    );
    refuses(
        &format!("{WRITES}\nfn f (n: Text) -> Key users = key (insert users {{ name: n }})"),
        "a write appears only where a change set is expected",
    );
}

// Rejects

#[test]
fn rejects_go_into_a_rejected_row_relation() {
    let (s, a) = ok(
        &[],
        "insert identities.Google from (
           from r in imports.Oauth select Google { user: r.user, gid: r.sub })
         rejects into google_rejects",
    );
    let Kind::Rejects { into, insert } = a.terms.kind(a.items[0]) else {
        panic!("rejects, found {:?}", a.terms.kind(a.items[0]));
    };
    assert_eq!(*into, rel_id(&s, "google_rejects"));
    let Kind::Insert(_, rows) = a.terms.kind(*insert) else {
        panic!("an insert");
    };
    assert!(matches!(a.terms.kind(*rows), Kind::For(..)));
    no(
        &[],
        "insert identities.Google from (
           from r in imports.Oauth select Google { user: r.user, gid: r.sub })
         rejects into users",
        "`rejects into` names a relation of `RejectedRow`",
    );
}

#[test]
fn rejects_need_a_single_source_generator() {
    let one = "a single generator over a source relation";
    no(
        &[],
        "insert identities.Google from (
           from r in imports.Oauth, u in users where r.user == key u
           select Google { user: r.user, gid: r.sub })
         rejects into google_rejects",
        one,
    );
    no(
        &[],
        "insert identities.Google from (from r in (from x in imports.Oauth select x)
                                       select Google { user: r.user, gid: r.sub })
         rejects into google_rejects",
        one,
    );
    no(
        NG,
        "insert identities.Google { user: u, gid: g } rejects into google_rejects",
        one,
    );
}

#[test]
fn reject_appears_only_in_an_insert_source() {
    let (_, a) = ok(
        &[],
        "insert identities from (
           from r in imports
           select match r
             | Email e -> EmailPass { user: e.user, email: e.addr, hash: e.h }
             | Oauth o -> reject \"no oauth\")
         rejects into google_rejects",
    );
    assert!(matches!(a.terms.kind(a.items[0]), Kind::Rejects { .. }));
    no(
        &[],
        "from r in imports select match r | Email e -> e.addr | Oauth o -> reject \"no\"",
        "allowed only in the source query of an insert",
    );
    no(
        &[("u", "Key users")],
        "insert identities.Google { user: u, gid: reject \"no\" }",
        "allowed only in the source query of an insert",
    );
}

// Write forms

#[test]
fn compound_writes() {
    let (_, a) = ok(&[], "skip");
    assert_eq!(a.terms.kind(a.items[0]), &Kind::Skip);
    let (_, a) = ok(&[], "fail \"no\"");
    assert!(matches!(a.terms.kind(a.items[0]), Kind::Fail(_)));
    no(&[], "fail 1", "expected Text");
    let (_, a) = ok(NG, "(insert users { name: n }, delete s in logins.Revoked)");
    assert!(matches!(a.terms.kind(a.items[0]), Kind::Par(ws) if ws.len() == 2));
    let (_, a) = ok(
        &[("c", "Bool"), ("n", "Text")],
        "if c then insert users { name: n } else skip",
    );
    assert!(matches!(a.terms.kind(a.items[0]), Kind::If(..)));
    let (_, a) = ok(
        &[("s", "Status"), ("n", "Text")],
        "match s | Draft -> skip | Sent x -> insert users { name: n } | Paid p -> fail \"paid\"",
    );
    assert!(matches!(a.terms.kind(a.items[0]), Kind::Let(..)));
}

#[test]
fn a_write_or_pattern_binds_each_name_once() {
    let (_, a) = ok(
        &[("s", "Status"), ("t", "Instant"), ("n", "Text")],
        "match s
           | Sent x | Paid x if x.at > t -> insert users { name: n }
           | Sent _ | Paid _             -> skip
           | Draft                       -> skip",
    );
    assert_one_binding_site("checked", &a.terms, &adhoc_roots(&a.items, &a.commands));
}

#[test]
fn a_write_match_is_exhaustive() {
    txn_no(
        "txn t (s: Status) -> () =
           transact
             match s | Draft -> skip | Sent x -> skip",
        "does not cover",
    );
}

#[test]
fn let_binds_four_forms() {
    let binds = "`let` binds the rows of";
    for w in [
        "skip",
        "fail \"no\"",
        "(insert users { name: n }, insert users { name: n })",
        "if true then insert users { name: n } else skip",
        "match 1 | 1 -> skip | _ -> skip",
    ] {
        no(
            &[("n", "Text")],
            &format!("transact\n  let x = {w}\n  return 1"),
            binds,
        );
    }
}

#[test]
fn let_types_of_the_four_forms() {
    assert_eq!(
        ty_of(
            &[("n", "Text"), ("g", "Text")],
            "transact
               let u = insert users { name: n }
               let i = insert identities.Google { user: key u, gid: g }
               return key i",
        ),
        "Key identities.Google"
    );
    assert_eq!(
        ty_of(
            &[],
            "transact
               let r = insert identities.Google from (
                         from x in imports.Oauth select { user: x.user, gid: x.sub })
               return r",
        ),
        "Set Identity.Google"
    );
    assert_eq!(
        ty_of(
            &[("u", "Key users"), ("g2", "Text")],
            "transact
               let s = update g in identities.Google where g.user == u
                       set Google { g with gid: g2 }
               return s",
        ),
        "Set Identity.Google"
    );
    assert_eq!(
        ty_of(
            &[("u", "Key users")],
            "transact
               let d = delete s in logins.Revoked where s.user == u
               return d",
        ),
        "Set Login.Revoked"
    );
    assert_eq!(
        ty_of(
            &[("n", "Text")],
            "transact\n  let u = (insert users { name: n })\n  return u"
        ),
        "User"
    );
}

#[test]
fn a_let_bound_write_is_a_let_then_its_change() {
    let (s, a) = ok(
        &[("n", "Text")],
        "transact
           let u = insert users { name: n }
           return key u",
    );
    let Kind::Transact(body, _) = a.terms.kind(a.items[0]) else {
        panic!("a transact, found {:?}", a.terms.kind(a.items[0]));
    };
    let Kind::Let(v, _, blk) = a.terms.kind(*body) else {
        panic!("a let");
    };
    let Kind::Block(ins, ret) = a.terms.kind(*blk) else {
        panic!("a block");
    };
    let Kind::Insert(rel, rows) = a.terms.kind(*ins) else {
        panic!("an insert");
    };
    assert_eq!(*rel, rel_id(&s, "users"));
    assert!(
        matches!(a.terms.kind(*rows), Kind::Singleton(x) if a.terms.kind(*x) == &Kind::Var(*v))
    );
    assert!(matches!(a.terms.kind(*ret), Kind::Key(_)));
    let (_, a) = ok(
        &[("u", "Key users"), ("g2", "Text")],
        "transact
           let s = update g in identities.Google where g.user == u set Google { g with gid: g2 }
           return s",
    );
    let Kind::Transact(body, _) = a.terms.kind(a.items[0]) else {
        panic!("a transact");
    };
    assert!(matches!(a.terms.kind(*body), Kind::LetBang(..)));
}

#[test]
fn a_block_sees_its_lets_and_returns_unit_without_return() {
    assert_eq!(
        ty_of(
            &[("n", "Text")],
            "transact
               let c = count(u for u in users)
               insert users { name: n }",
        ),
        "()"
    );
}

// Transact options

#[test]
fn transact_options_are_carried() {
    let (_, a) = ok(
        &[],
        "transact read_only deferrable
           return count(i for i in identities)",
    );
    let Kind::Transact(_, o) = a.terms.kind(a.items[0]) else {
        panic!("a transact");
    };
    assert!(o.read_only && o.deferrable && o.retry.is_none());
    let (_, a) = ok(
        &[("u", "Key users"), ("g", "Text")],
        "transact retry 5 durability quorum
           let i = insert identities.Google { user: u, gid: g }
           return key i",
    );
    let Kind::Transact(_, o) = a.terms.kind(a.items[0]) else {
        panic!("a transact");
    };
    assert_eq!(o.retry, Some(5));
    assert_eq!(o.durability, Some(Durability::Quorum));
    assert!(!o.read_only);
}

#[test]
fn a_read_only_block_never_writes() {
    no(
        &[("n", "Text")],
        "transact read_only
           insert users { name: n }",
        "a `read_only` block may not write",
    );
    no(
        &[("n", "Text")],
        "transact read_only
           let u = insert users { name: n }
           return key u",
        "a `read_only` block may not write",
    );
    no(
        &[("n", "Text")],
        "transact read_only
           let k = key (insert users { name: n })
           return k",
        "a `read_only` block may not write",
    );
    no(
        &[],
        "transact read_only durability local
           return count(i for i in identities)",
        "takes no `durability`",
    );
    no(
        &[],
        "transact read_only deferrable durability quorum
           return count(i for i in identities)",
        "takes no `durability`",
    );
}

#[test]
fn a_transact_option_is_given_once() {
    no(
        &[],
        "transact retry 1 retry 2
           return 1",
        "given twice",
    );
}

// Registered transactions

#[test]
fn registered_transactions() {
    txn_ok(
        "txn sign_up (n: Text, g: Text) -> Key identities =
           transact
             let u = insert users { name: n }
             let i = insert identities.Google { user: key u, gid: g }
             return key i",
    );
    txn_ok(
        "txn note (b: Text) -> () =
           transact
             insert notes { author: None, body: b }",
    );
    txn_no(
        "txn sign_up (n: Text) -> Text =
           transact
             let u = insert users { name: n }
             return key u",
        "expected Text",
    );
    txn_no(
        "txn sign_up (n: Text) -> Key users =
           transact
             insert users { name: n }",
        "returns nothing",
    );
}

// Delete actions

#[test]
fn a_retag_action_is_elaborated_per_relation() {
    let s = wschema();
    let p = check_schema(&s).expect("checks");
    let logins = rel_id(&s, "logins");
    let tm = p.retags[&logins];
    let Kind::Lam(x, body) = p.terms.kind(tm) else {
        panic!("a lambda");
    };
    assert!(matches!(p.terms.kind(*body), Kind::Ctor { ctor: 1, .. }));
    assert_eq!(
        p.terms.kind(field(&p.terms, *body, "at")),
        &Kind::Fresh(Generator::Clock, logins)
    );
    let Kind::Field(base, _) = p.terms.kind(field(&p.terms, *body, "user")) else {
        panic!("the header is copied");
    };
    assert_eq!(p.terms.kind(*base), &Kind::Var(*x));
}

#[test]
fn delete_actions_are_checked_with_the_schema() {
    schema_no(
        "type H { id: Key hs, u: Key users on delete retag B {} } = A { a: I64 } | B { b: I64 }
         relation hs : H key id generate sequence",
        "not allowed on header field",
    );
    schema_no(
        "type H { id: Key hs } = A { u: Key users on delete retag A {} } | B { b: I64 }
         relation hs : H key id generate sequence",
        "needs a constructor without the field `u`",
    );
    schema_no(
        "type H { id: Key hs } = A { u: Key users on delete retag B { b: 1 },
                                      v: Key users on delete retag B { b: 2 } }
                               | B { b: I64 }
         relation hs : H key id generate sequence",
        "differ",
    );
    schema_no(
        "type H { id: Key hs } = A { u: Key users on delete retag B { b: 1 + 1 } } | B { b: I64 }
         relation hs : H key id generate sequence",
        "a record of literals",
    );
    schema_no(
        "type H { id: Key hs } = A { u: Key users on delete retag B {} } | B { b: I64 }
         relation hs : H key id generate sequence",
        "field `b` of `B` is missing",
    );
    accepts(&format!(
        "{WRITES}
         type H {{ id: Key hs }} = A {{ u: Key users on delete retag B {{ s: Draft, o: None }} }}
                                 | B {{ s: Status, o: Option I64 }}
         relation hs : H key id generate sequence"
    ));
    schema_no(
        "type Q = X | Y
         type H { id: Key hs } = A { u: Key users on delete retag B { q: X },
                                      v: Key users on delete retag B { q: Y } }
                               | B { q: Q }
         relation hs : H key id generate sequence",
        "differ",
    );
    accepts(&format!(
        "{WRITES}
         type H {{ id: Key hs }} = A {{ u: Key users on delete retag B {{ b: 1 }},
                                        v: Key users on delete retag B {{ b: 1 }} }}
                                 | B {{ b: I64 }}
         relation hs : H key id generate sequence"
    ));
}

// Chunk 1 still holds

#[test]
fn a_write_inside_a_query_is_refused() {
    refuses(
        &format!(
            "{WRITES}\nfn f (n: Text) -> Set User = from u in users where (insert users {{ name: n }}) == u select u"
        ),
        "a write appears only where a change set is expected",
    );
}

#[test]
fn update_match_arms_join_declared_row_types() {
    let (s, a) = ok(
        &[("k", "Key identities")],
        "transact
           let r = update i in identities where i.id == k
                   set match i
                     | EmailPass e -> Google { gid: e.email }
                     | Google g    -> Apple { sub: g.gid }
                     | Apple a     -> a
           return r",
    );
    assert_eq!(
        "Set Identity.(Google | Apple)",
        s.show_ty(a.terms.ty(a.items[0]))
    );
    assert!(anon_ctors(&a.terms, a.items[0]).is_empty());
    no(
        &[("k", "Key identities")],
        "update i in identities where i.id == k
         set match i
           | EmailPass e -> Zed { n: 1 }
           | Google g    -> Alpha
           | Apple a     -> a",
        "Zed",
    );
}

#[test]
fn upsert_rows_join_declared_row_types() {
    let (s, a) = ok(
        &[("u", "Key users"), ("g", "Text"), ("k", "Key identities")],
        "transact
           let r = insert identities.Google { id: k, user: u, gid: g }
                   unless conflict on key
                   else match old
                     | Google o    -> update o set o
                     | EmailPass e -> update e set Apple { sub: g }
                     | Apple a     -> skip
           return r",
    );
    assert_eq!("Identity", s.show_ty(a.terms.ty(a.items[0])));
    assert!(anon_ctors(&a.terms, a.items[0]).is_empty());
    no(
        &[("u", "Key users"), ("g", "Text")],
        "insert identities.Google { user: u, gid: g }
         unless conflict on (gid) in Google
         else update old set Zed { n: 1 }",
        "Zed",
    );
}

const VOLATILE_WRITE: &str = "a write must not depend on a volatile relation";

#[test]
fn a_volatile_read_never_reaches_a_write() {
    no(
        &[],
        "insert users from (from s in sys::sessions select { name: s.principal })",
        VOLATILE_WRITE,
    );
    no(
        &[],
        "delete u in users where exists s in sys::sessions where s.principal == u.name",
        VOLATILE_WRITE,
    );
    no(
        &[],
        "transact
           let n = count(s for s in sys::sessions)
           insert users { name: text(n) }",
        VOLATILE_WRITE,
    );
    no(
        &[],
        "transact
           let busy = exists s in sys::sessions where s.principal == \"x\"
           if busy then insert users { name: \"x\" } else skip",
        VOLATILE_WRITE,
    );
}

#[test]
fn a_volatile_read_may_be_returned() {
    ok(&[], "from s in sys::sessions select s.principal");
    ok(
        &[],
        "transact
           let open = from s in sys::sessions select s.principal
           insert users { name: \"x\" }
           return open",
    );
}

#[test]
fn a_registered_transaction_keeps_volatile_reads_out_of_its_writes() {
    txn_no(
        "txn t () -> () =
           transact
             let n = count(s for s in sys::sessions)
             insert users { name: text(n) }",
        VOLATILE_WRITE,
    );
}

fn feed(text: &str) -> Result<(RelId, CtorSet), String> {
    let s = wschema();
    let items = parse_example(text, Options::default())
        .ok()
        .expect("parses");
    let Some(ExampleItem::BundleItem(BundleItem::Feed(f))) = items.into_iter().next() else {
        panic!("{text}\nis not a feed");
    };
    let module = s.module_named("auth").expect("module auth");
    check_feed(&s, module, &f).map_err(|d| d.message)
}

#[test]
fn a_feed_resolves_over_the_modules_its_bundle_sees() {
    let auth = parse_schema(&source(""), Options::default())
        .ok()
        .expect("module auth parses");
    let billing = parse_schema(
        "module billing\ntype Payout = { id: Key payouts, cents: I64 }\nrelation payouts : Payout key id generate sequence\n",
        Options::default(),
    )
    .ok()
    .expect("module billing parses");
    let s = build(&[(FileId(0), auth), (FileId(1), billing)])
        .unwrap_or_else(|ds| panic!("the schema builds: {ds:?}"));
    let items = parse_example("feed payout_changes of payouts", Options::default())
        .ok()
        .expect("parses");
    let Some(ExampleItem::BundleItem(BundleItem::Feed(f))) = items.into_iter().next() else {
        panic!("not a feed");
    };
    let module = |n: &str| s.module_named(n).expect("a module");
    let vis = Visible {
        modules: vec![module("auth"), module("billing")],
    };
    let (rel, _) = check_feed_in(&s, vis, &f).unwrap_or_else(|d| panic!("refused: {d:?}"));
    assert_eq!("payouts", s.relation(rel).name);
    assert!(check_feed(&s, module("auth"), &f).is_err());
}

#[test]
fn a_feed_names_no_volatile_relation() {
    assert_eq!(
        Err("a feed must not name a volatile relation".to_string()),
        feed("feed sessions_changes of sys::sessions")
    );
    let (_, ctors) = feed("feed identity_changes of identities.(Google | Apple)")
        .unwrap_or_else(|e| panic!("refused: {e}"));
    assert_eq!(2, ctors.len());
    assert!(feed("feed nothing of nosuch").is_err_and(|e| e.contains("nosuch")));
}

#[test]
fn schema_objects_never_read_a_volatile_relation() {
    schema_no(
        "constraint quiet = from s in sys::sessions where s.principal == \"\" select s",
        "a constraint must not read a volatile relation",
    );
    schema_no(
        "fn busy () -> Set Text = from s in sys::sessions select s.principal\nstore busy",
        "a stored view must not read a volatile relation",
    );
    schema_no(
        "type Tick = { id: Key ticks, n: I64 }\nrelation ticks : Tick key id generate sequence default n = count(s for s in sys::sessions)",
        "a default must not read a volatile relation",
    );
    schema_no(
        "type Tick = { id: Key ticks, n: I64 }\nrelation ticks : Tick key id generate sequence computed busy: I64 = count(s for s in sys::sessions)",
        "a computed field must not read a volatile relation",
    );
    schema_no(
        "fn sessions_now () -> I64 = count(s for s in sys::sessions)\nconstraint none = from u in users where sessions_now() > 100 select u",
        "a constraint must not read a volatile relation",
    );
}
