use super::*;
use crate::ast::{BundleItem, ExampleItem};
use crate::kernel::Prim;
use crate::parse::{Options, parse_adhoc, parse_example, parse_schema, parse_type};
use crate::schema::build;

const RUNNING: &str = r#"
module auth
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
set OAuth of identities = Google | Apple
fn display_name (i: Identity) -> Text = match i
  | EmailPass e -> e.email
  | Google g    -> g.gid
  | Apple a     -> a.sub

type Post = { id: Key posts, author: Key users, created_at: Instant, title: Text }
type Phone = { id: Key phones, owner: Key users on delete cascade, seen: Instant }
type Follow = { src: Key users, dst: Key users }
type Employee = { id: Key employees, name: Text, manager: Option (Key employees) }
type Order = { id: Key orders, user: Key users, total: I64, qty: I64 }
type Reset = { id: Key resets, identity: Key identities.EmailPass on delete cascade }
type Raw = { id: Key raws, n: I64 }
relation posts : Post key id generate sequence
relation phones : Phone key id generate sequence
relation follows : Follow key (src, dst)
relation employees : Employee key id generate sequence
relation orders : Order key id generate sequence
relation resets : Reset key id generate sequence
relation raws : Raw key id generate sequence

type Status = Draft | Sent { at: Instant } | Paid { at: Instant }
type Invoice = { id: Key invoices, status: Status, issued: Instant }
relation invoices : Invoice key id generate sequence
type Via = Courier { name: Text } | Mail { tracking: Text }
type Shipment = Pending | Shipped { via: Via }
type Session = { id: Key sessions, owner: Option (Key users) }
relation sessions : Session key id generate sequence
type Sale = { id: Key sales, qty: I32, unit_cents: I64 }
relation sales : Sale key id generate sequence
"#;

fn source(extra: &str) -> String {
    format!("{RUNNING}\n{extra}")
}

fn schema(extra: &str) -> Result<Schema, Vec<String>> {
    let src = source(extra);
    let parsed = parse_schema(&src, Options::default());
    if !parsed.diags.is_empty() {
        panic!(
            "{extra}\ndoes not parse: {:?}",
            parsed
                .diags
                .iter()
                .map(|d| d.render(&src))
                .collect::<Vec<_>>()
        );
    }
    let file = parsed.ok().expect("parsed");
    build(&[(FileId(0), file)]).map_err(|ds| ds.into_iter().map(|d| d.diag.message).collect())
}

fn program(extra: &str) -> Result<(Schema, Program), Vec<String>> {
    let s = schema(extra)?;
    match check_schema(&s) {
        Ok(p) => Ok((s, p)),
        Err(ds) => Err(ds.into_iter().map(|d| d.diag.message).collect()),
    }
}

fn accepts(extra: &str) {
    if let Err(errs) = program(extra) {
        panic!("{extra}\nrefused: {errs:?}");
    }
}

fn refuses(extra: &str, needle: &str) {
    match program(extra) {
        Ok(_) => panic!("{extra}\naccepted; expected an error containing {needle:?}"),
        Err(errs) => assert!(
            errs.iter().any(|e| e.contains(needle)),
            "{extra}\nno error containing {needle:?} in {errs:?}"
        ),
    }
}

/// Checks ad hoc `text` against the running schema, its parameters given as `name: Type`.
fn adhoc(params: &[(&str, &str)], text: &str) -> Result<(Schema, Adhoc), Vec<String>> {
    let s = schema("").expect("the running schema builds");
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
    match check_adhoc(&s, &items, &tys) {
        Ok(a) => Ok((s, a)),
        Err(ds) => Err(ds.into_iter().map(|d| d.message).collect()),
    }
}

/// The type of the single item of ad hoc text.
fn adhoc_ty(params: &[(&str, &str)], text: &str) -> String {
    match adhoc(params, text) {
        Ok((s, a)) => s.show_ty(a.terms.ty(a.items[0])),
        Err(errs) => panic!("{text}\nrefused: {errs:?}"),
    }
}

fn adhoc_accepts(params: &[(&str, &str)], text: &str) {
    if let Err(errs) = adhoc(params, text) {
        panic!("{text}\nrefused: {errs:?}");
    }
}

fn adhoc_refuses(params: &[(&str, &str)], text: &str, needle: &str) {
    match adhoc(params, text) {
        Ok(_) => panic!("{text}\naccepted; expected an error containing {needle:?}"),
        Err(errs) => assert!(
            errs.iter().any(|e| e.contains(needle)),
            "{text}\nno error containing {needle:?} in {errs:?}"
        ),
    }
}

/// Checks a registered query `text` against the running schema extended by `extra`.
fn query(extra: &str, text: &str) -> Result<(), String> {
    let s = schema(extra).expect("the schema builds");
    let items = parse_example(text, Options::default())
        .ok()
        .expect("parses");
    let Some(ExampleItem::BundleItem(BundleItem::Query(q))) = items.into_iter().next() else {
        panic!("{text}\nis not a query");
    };
    let module = s.module_named("auth").expect("module auth");
    check_query(&s, module, &q)
        .map(|_| ())
        .map_err(|d| d.message)
}

fn query_accepts(text: &str) {
    if let Err(e) = query("", text) {
        panic!("{text}\nrefused: {e}");
    }
}

fn query_refuses(text: &str, needle: &str) {
    match query("", text) {
        Ok(()) => panic!("{text}\naccepted; expected an error containing {needle:?}"),
        Err(e) => assert!(e.contains(needle), "{text}\nerror {e:?} lacks {needle:?}"),
    }
}

/// The body of function `name`, inside its `Lam`.
fn body_of(s: &Schema, p: &Program, name: &str) -> Tm {
    let i = s
        .functions
        .iter()
        .position(|f| f.name == name)
        .expect("a function of that name");
    let mut t = p.functions[i].expect("checked");
    for _ in 0..s.functions[i].params.len().max(1) {
        t = match p.terms.kind(t) {
            Kind::Lam(_, body) => *body,
            k => panic!("a function is one Lam per parameter, found {k:?}"),
        };
    }
    t
}

// Signatures and inference

#[test]
fn signatures_are_written_and_bodies_inferred() {
    query_accepts(
        "query by_status (s: Status, since: Instant) -> Set ({ invoice: Key invoices }) =
           from i in invoices where i.status == s and i.issued >= since
           select { invoice: key i }",
    );
    query_accepts("query n () -> I64 = count(u for u in users)");
    accepts("fn n_users () -> I64 = count(u for u in users)");
}

#[test]
fn match_arms_join_to_the_signature() {
    accepts(
        "fn oauth_owner (u: Key users) -> Set (Key identities) =
           from i in identities where i.user == u
           select match i | Google g -> key g | Apple a -> key a | EmailPass e -> key e",
    );
}

#[test]
fn adhoc_free_names_are_typed_by_their_descriptors() {
    adhoc_accepts(
        &[("u", "Key users")],
        "from i in identities where i.user == u select i",
    );
    adhoc_refuses(
        &[],
        "from i in identities where i.user == u select i",
        "the call supplies no value for it",
    );
}

#[test]
fn a_body_is_checked_against_its_result() {
    refuses(
        "fn f () -> Text = count(u for u in users)",
        "expected Text, found I64",
    );
    refuses(
        "fn f () -> Set Text = from u in users select u",
        "expected Text, found User",
    );
}

// Subtyping

#[test]
fn constructor_rows_are_subtypes_of_their_sum() {
    accepts("fn f (g: Identity.Google) -> Identity = g");
    accepts("fn f (g: Identity.Google) -> Identity.OAuth = g");
    accepts("fn f (g: Identity.(Google | Apple)) -> Identity.OAuth = g");
    refuses(
        "fn f (g: Identity) -> Identity.Google = g",
        "expected Identity.Google, found Identity",
    );
    refuses(
        "fn f (g: Identity.OAuth) -> Identity.Google = g",
        "expected Identity.Google",
    );
}

#[test]
fn listing_every_constructor_is_the_sum() {
    accepts("fn f (g: Identity) -> Identity.(EmailPass | Google | Apple) = g");
}

#[test]
fn narrower_keys_widen() {
    accepts("fn f (k: Key identities.Google) -> Key identities = k");
    accepts("fn f (k: Key identities.Google) -> Key identities.OAuth = k");
    refuses(
        "fn f (k: Key identities) -> Key identities.Google = k",
        "expected Key identities.Google",
    );
}

#[test]
fn sets_and_functions_are_variant() {
    accepts("fn f (s: Set (Identity.Google)) -> Set Identity = s");
    accepts("fn f (p: Identity -> Bool) -> Identity.Google -> Bool = p");
    refuses(
        "fn f (p: Identity.Google -> Bool) -> Identity -> Bool = p",
        "expected",
    );
}

#[test]
fn declared_type_parameters_are_covariant() {
    accepts(
        "type Fallback = { id: Key fallbacks, backup: Option (Key identities.Google) }
         relation fallbacks : Fallback key id generate sequence
         fn backups () -> Set (Option (Key identities)) =
           from f in fallbacks select f.backup",
    );
}

#[test]
fn a_parameter_in_no_field_relates_any_instantiations() {
    accepts(
        "type Phantom a = { id: Key phantoms }
         relation phantoms : Phantom I64 key id generate sequence
         fn f (x: Phantom I64) -> Phantom Text = x",
    );
}

#[test]
fn distinct_types_are_unrelated() {
    refuses(
        "type Email = new Text
         fn f (e: Email) -> Text = e",
        "expected Text, found Email",
    );
    refuses("fn f (x: I32) -> I64 = x", "expected I64, found I32");
    refuses(
        "fn f (x: { a: I64, b: I64 }) -> { a: I64 } = x",
        "expected { a: I64 }",
    );
    refuses(
        "fn f (x: { k: Key identities.Google }) -> { k: Key identities } = x",
        "expected { k: Key identities }",
    );
    refuses("fn f (x: Key users) -> Key posts = x", "expected Key posts");
    refuses(
        "fn f (x: User) -> { id: Key users, name: Text } = x",
        "expected { id: Key users, name: Text }, found User",
    );
}

#[test]
fn anonymous_sums_are_equal_up_to_order() {
    accepts("fn f (x: A { a: I64 } | B { b: Text }) -> B { b: Text } | A { a: I64 } = x");
    refuses(
        "fn f (x: A { a: I64 } | B { b: Text }) -> A { a: I64 } | C { b: Text } = x",
        "expected",
    );
}

// Joins

#[test]
fn match_arms_join_by_constructor_set() {
    assert_eq!(
        adhoc_ty(
            &[],
            "from i in identities select match i
               | Google g    -> g
               | Apple a     -> a
               | EmailPass e -> e"
        ),
        "Set Identity"
    );
    assert_eq!(
        adhoc_ty(
            &[("i", "Identity")],
            "match i | Google g -> key g | Apple a -> key a | EmailPass e -> key e"
        ),
        "Key identities"
    );
}

#[test]
fn partial_constructor_sets_join_to_their_union() {
    assert_eq!(
        adhoc_ty(
            &[("i", "Identity")],
            "from x in identities select match x | Google g -> key g | Apple a -> key a | EmailPass e -> key e"
        ),
        "Set (Key identities)"
    );
    assert_eq!(
        adhoc_ty(
            &[],
            "(from g in identities.Google select g) union (from a in identities.Apple select a)"
        ),
        "Set Identity.(Google | Apple)"
    );
}

#[test]
fn record_arms_join_only_when_equal() {
    adhoc_refuses(
        &[("i", "Identity")],
        "match i
           | Google g    -> { sub: g.gid, team: g.gid }
           | Apple a     -> { sub: a.sub }
           | EmailPass e -> { sub: e.email }",
        "field team missing",
    );
    adhoc_refuses(
        &[("i", "Identity")],
        "match i | Google g -> { n: 1 } | Apple a -> { n: a.sub } | EmailPass e -> { n: 2 }",
        "no common type",
    );
}

#[test]
fn a_written_type_meets_arms_at_a_wider_field() {
    accepts(
        "fn f (i: Identity) -> { k: Key identities } = match i
           | Google g -> { k: key g } | Apple a -> { k: key a } | EmailPass e -> { k: key e }",
    );
}

#[test]
fn different_sums_have_no_join() {
    adhoc_refuses(
        &[("i", "Identity"), ("u", "User")],
        "match i | Google g -> g | Apple a -> u | EmailPass e -> e",
        "no common type",
    );
}

#[test]
fn union_operands_join() {
    accepts(
        "fn f () -> Set Identity = (from g in identities.Google select g) union (from e in identities.EmailPass select e)",
    );
    adhoc_refuses(
        &[],
        "(from u in users select u.name) union (from u in users select key u)",
        "no common type",
    );
}

#[test]
fn comparisons_meet_at_the_join() {
    accepts(
        "type Visit = { id: Key visits, identity: Key identities }
         relation visits : Visit key id generate sequence
         fn f () -> Set Visit =
           from v in visits, g in identities.Google
           where v.identity == key g
           select v",
    );
    refuses(
        "fn f () -> Set User = from u in users, p in posts where key u == key p select u",
        "cannot be compared",
    );
    refuses("fn f (s: Status) -> Bool = s < s", "has no order");
    accepts("fn f (a: Decimal 2, b: Decimal 3) -> Bool = a < b");
}

#[test]
fn a_constructor_operand_takes_the_other_operand_type() {
    accepts("fn f (p: Option I64) -> Bool = p == None");
    accepts("fn f (p: Option I64) -> Bool = Some { value: 1 } == p");
    accepts("fn f (r: { a: I64 }) -> Bool = r == { a: 1 }");
}

#[test]
fn generic_constructor_arguments_come_from_the_expected_type_or_fields() {
    accepts("fn f () -> Set (Option I64) = [Some { value: 1 }]");
    adhoc_accepts(&[], "Some { value: 1 }");
    adhoc_refuses(
        &[],
        "None",
        "the type argument `a` of `Option` is not known here",
    );
    adhoc_refuses(
        &[],
        "Err { error: 1 }",
        "the type argument `a` of `Result` is not known here",
    );
}

#[test]
fn lambdas_are_curried() {
    accepts(
        "fn ap (g: I64 -> I64 -> I64) -> I64 = g(1, 2)
         fn h () -> I64 = ap(fn (a: I64, b: I64) -> a + b)",
    );
    accepts(
        "fn add (a: I64, b: I64) -> I64 = a + b
         fn ap (g: I64 -> I64 -> I64) -> I64 = g(1, 2)
         fn h () -> I64 = ap(add)",
    );
    accepts(
        "fn v () -> I64 = 1
fn h () -> () -> I64 = v",
    );
}

#[test]
fn no_literal_checks_against_a_key() {
    refuses(
        "fn f () -> Set Identity = from i in identities where i.user == 3 select i",
        "Key users",
    );
    refuses("fn f () -> Key users = 3", "Key users");
    accepts("fn f () -> Set Identity = from i in identities where i.user == users.key(3) select i");
}

// Records, constructors and fields

#[test]
fn functions_take_nominal_types() {
    accepts("fn email_of (e: Identity.EmailPass) -> Text = e.email");
    refuses(
        "fn f (e: Identity) -> Text = e.email",
        "has no field `email`",
    );
}

#[test]
fn a_constructor_application_gives_every_field() {
    accepts(
        "fn f (u: Key users, k: Key identities) -> Identity.Google = Google { id: k, user: u, gid: \"g\" }",
    );
    refuses(
        "fn f (u: Key users, k: Key identities) -> Identity.Google = Google { id: k, user: u }",
        "field `gid` of `Google` is missing",
    );
    refuses(
        "fn f (u: Key users, k: Key identities) -> Identity.Google = Google { id: k, user: u, gid: \"g\", sub: \"s\" }",
        "has no field `sub`",
    );
}

#[test]
fn a_constructor_application_synthesizes_its_row_type() {
    assert_eq!(
        adhoc_ty(
            &[("u", "Key users"), ("k", "Key identities")],
            "Google { id: k, user: u, gid: \"g\" }"
        ),
        "Identity.Google"
    );
    assert_eq!(adhoc_ty(&[], "Draft"), "Status.Draft");
}

#[test]
fn a_record_literal_against_a_row_type_constructs_it() {
    accepts(
        "fn f (x: Identity.Google) -> Identity.Google = { id: x.id, user: x.user, gid: \"a\" }",
    );
    accepts("fn f (u: Key users) -> Set Follow = [{ src: u, dst: u }]");
    assert_eq!(adhoc_ty(&[], "{ b: 1, a: \"x\" }"), "{ a: Text, b: I64 }");
}

#[test]
fn constructor_names_resolve_against_the_expected_type() {
    accepts("fn f () -> Option I64 = Some { value: 1 }");
    accepts("fn f () -> Option I64 = None");
    accepts("fn f () -> Status = Draft");
    refuses(
        "fn f () -> Status = Bogus",
        "`Bogus` is not a constructor of Status",
    );
    refuses(
        "fn f () -> Status = Pending",
        "expected Status, found Shipment.Pending",
    );
}

#[test]
fn an_anonymous_constructor_is_only_checked() {
    accepts(
        "fn f () -> Set (Local { n: I64 } | Remote { p: Text }) = [Local { n: 1 }, Remote { p: \"x\" }]",
    );
    refuses(
        "fn f () -> I64 = count(x for x in [Local { n: 1 }])",
        "no constructor `Local` in scope",
    );
}

#[test]
fn ambiguous_constructors_are_qualified() {
    accepts(
        "type Other = Google { x: I64 } | Nope
         fn f () -> Other.Google = Other.Google { x: 1 }",
    );
    refuses(
        "type Other = Google { x: I64 } | Nope
         fn f (u: Key users, k: Key identities) -> I64 = count(x for x in [Google { id: k, user: u, gid: \"g\" }])",
        "qualify it",
    );
}

#[test]
fn a_newtype_is_built_by_a_call() {
    accepts("type Email = new Text\nfn f () -> Email = Email(\"a@b\")");
    refuses(
        "type Email = new Text\nfn f () -> Email = Email(3)",
        "expected Text",
    );
    accepts("type Email = new Text\nfn f (e: Email) -> Text = unwrap(e)");
}

#[test]
fn field_access_follows_the_type() {
    accepts("fn f (i: Identity) -> Key users = i.user");
    accepts("fn f (g: Identity.Google) -> Text = g.gid");
    accepts("fn f (o: Identity.OAuth) -> Key users = o.user");
    refuses(
        "fn f (o: Identity.OAuth) -> Text = o.gid",
        "has no field `gid`",
    );
    accepts("fn f (r: { a: I64, b: Text }) -> Text = r.b");
    refuses("fn f (r: { a: I64 }) -> Text = r.b", "has no field `b`");
    refuses(
        "fn f (i: Identity) -> Text = i.gid",
        "only header fields are reached on the whole sum",
    );
}

#[test]
fn shared_fields_of_a_constructor_set_are_reached() {
    accepts(
        "type Shape = Circle { r: I64, name: Text } | Square { s: I64, name: Text } | Dot
         fn f (x: Shape.(Circle | Square)) -> Text = x.name",
    );
    refuses(
        "type Shape = Circle { r: I64, name: Text } | Square { s: I64, name: Text } | Dot
         fn f (x: Shape.(Circle | Square)) -> I64 = x.r",
        "has no field `r`",
    );
    refuses(
        "type Shape = Circle { v: I64 } | Square { v: Text }
         fn f (x: Shape) -> I64 = match x | Circle c -> 1 | Square s -> 2
         fn g (x: Shape.(Circle | Square)) -> I64 = x.v",
        "has no field `v`",
    );
}

#[test]
fn navigation_follows_a_reference() {
    accepts("fn f () -> Set Text = from i in identities select i.user.name");
    accepts("fn f () -> Text = users.key(3).name");
    refuses(
        "fn f () -> Set Text = from e in employees select e.manager.name",
        "a path stops at an `Option`",
    );
}

#[test]
fn navigation_desugars_to_deref() {
    let (s, p) = program("fn f (i: Identity) -> Text = i.user.name").unwrap();
    let body = body_of(&s, &p, "f");
    let Kind::Field(inner, name) = p.terms.kind(body) else {
        panic!("a field, found {:?}", p.terms.kind(body));
    };
    assert_eq!(name, "name");
    assert!(matches!(
        p.terms.kind(*inner),
        Kind::Deref {
            reference: true,
            ..
        }
    ));
    let (s, p) = program("fn f (k: Key users) -> Text = k.name").unwrap();
    let body = body_of(&s, &p, "f");
    let Kind::Field(inner, _) = p.terms.kind(body) else {
        panic!("a field");
    };
    assert!(matches!(
        p.terms.kind(*inner),
        Kind::Deref {
            reference: false,
            ..
        }
    ));
}

// key

#[test]
fn key_of_a_row_whose_relation_is_known() {
    accepts("fn f () -> Set (Key identities.Google) = from g in identities.Google select key g");
    accepts("fn f () -> Set (Key identities.OAuth) = from g in identities.OAuth select key g");
    accepts("fn f () -> Set (Key follows) = from x in (from f in follows select f) select key x");
    accepts(
        "fn f (u: Key users) -> Set (Key identities) = from i in u.<identities.user select key i",
    );
    accepts(
        "fn fs () -> Set Follow = from f in follows select f
         fn g () -> Set (Key follows) = from x in fs() select key x",
    );
    accepts(
        "fn f () -> Set (Key identities) = from i in identities select match i
           | Google g -> key g | Apple a -> key a | EmailPass e -> key e",
    );
}

#[test]
fn key_of_a_value_with_a_self_key_field() {
    accepts("fn f (u: User) -> Key users = key u");
}

#[test]
fn key_is_refused_elsewhere() {
    refuses(
        "fn f (x: Follow) -> Key follows = key x",
        "`key` needs a row whose relation is known here",
    );
    refuses(
        "fn f () -> Set (Key follows) = from x in (from f in follows select { src: f.src, dst: f.dst }) select key x",
        "`key` needs a row",
    );
}

#[test]
fn key_values_are_built_by_r_key() {
    accepts("fn f () -> Key users = users.key(3)");
    accepts("fn f (a: Key users, b: Key users) -> Key follows = follows.key(src: a, dst: b)");
    refuses(
        "fn f (a: Key users) -> Key follows = follows.key(a)",
        "composite key",
    );
    refuses("fn f () -> Key users = users.key(\"x\")", "expected I64");
    accepts("fn f (k: Key users) -> I64 = value(k)");
    accepts("fn f (k: Key follows) -> { dst: Key users, src: Key users } = value(k)");
}

// Navigation and backlinks

#[test]
fn backlinks_are_sets() {
    accepts(
        "fn f () -> Set ({ name: Text, n: I64 }) = from u in users
           select { name: u.name, n: count(i for i in u.<identities.user) }",
    );
    accepts("fn f (u: User) -> Set Identity = u.<identities.user");
    accepts("fn f (u: Key users) -> Set Identity = u.<identities.user");
    refuses(
        "fn f (u: User) -> Identity = u.<identities.user",
        "expected Identity",
    );
    accepts("fn f (u: User) -> Set Session = u.<sessions.owner");
    accepts("fn f (u: User) -> Set (Identity.Google) = u.<identities.Google.user");
    refuses(
        "fn f (p: Post) -> Set Identity = p.<identities.user",
        "references rows of `users`",
    );
}

#[test]
fn a_backlink_desugars_to_a_filtered_comprehension() {
    let (s, p) = program("fn f (u: User) -> Set Session = u.<sessions.owner").unwrap();
    let body = body_of(&s, &p, "f");
    let Kind::For(_, src, inner) = p.terms.kind(body) else {
        panic!("a for, found {:?}", p.terms.kind(body));
    };
    assert!(matches!(p.terms.kind(*src), Kind::Rel(..)));
    let Kind::If(test, _, _) = p.terms.kind(*inner) else {
        panic!("an if");
    };
    let Kind::Prim(Prim::Eq, args) = p.terms.kind(*test) else {
        panic!("an equality");
    };
    assert!(matches!(p.terms.kind(args[1]), Kind::Ctor { ctor: 1, .. }));
}

#[test]
fn joins_compare_a_reference_with_a_key() {
    accepts(
        "fn f () -> Set Text = from i in identities, u in users where i.user == key u select u.name",
    );
    accepts(
        "fn f () -> Set Text = from i in identities, u in users where key u == i.user select u.name",
    );
}

#[test]
fn a_path_to_a_sum_is_matched() {
    accepts(
        "fn f () -> Set Text = from r in resets select match r.identity | EmailPass e -> e.email",
    );
    accepts(
        "fn f () -> Set Text = from e in employees
           select match e.manager
             | None              -> \"none\"
             | Some { value: m } -> m.name",
    );
}

// Comprehensions and nested results

#[test]
fn generators_see_earlier_binders() {
    accepts(
        "fn f () -> Set ({ name: Text, id: Key identities }) = from u in users, i in u.<identities.user
           select { name: u.name, id: key i }",
    );
    accepts("fn f () -> Set (Key users) = from p in posts select p.author");
    refuses(
        "fn f () -> Set Text = from u in users select u",
        "expected Text",
    );
    refuses(
        "fn f () -> Set Text = from u in 3 select u",
        "a generator ranges over a set",
    );
}

#[test]
fn where_takes_a_bool() {
    refuses(
        "fn f () -> Set User = from u in users where u.name select u",
        "expected Bool",
    );
}

#[test]
fn a_comprehension_is_nested_for_if_singleton() {
    let (s, p) = program(
        "fn f () -> Set Text = from u in users, i in identities where i.user == key u select u.name",
    )
    .unwrap();
    let body = body_of(&s, &p, "f");
    let Kind::For(_, src, inner) = p.terms.kind(body) else {
        panic!("a for, found {:?}", p.terms.kind(body));
    };
    assert!(matches!(p.terms.kind(*src), Kind::Rel(..)));
    let Kind::For(_, _, inner) = p.terms.kind(*inner) else {
        panic!("a second for");
    };
    let Kind::If(_, yes, no) = p.terms.kind(*inner) else {
        panic!("an if");
    };
    assert!(matches!(p.terms.kind(*yes), Kind::Singleton(_)));
    assert!(matches!(p.terms.kind(*no), Kind::EmptySet));
}

#[test]
fn nested_results_hold_sets() {
    accepts(
        "fn f () -> Set ({ name: Text, phones: Set Phone }) = from u in users
           select { name: u.name,
                    phones: from d in phones where d.owner == key u select d }",
    );
    accepts(
        "fn f () -> Set ({ name: Text, seen: Instant }) =
           from r in (from u in users select { name: u.name, ds: u.<phones.owner }),
                d in r.ds
           select { name: r.name, seen: d.seen }",
    );
}

#[test]
fn exists_tests_emptiness() {
    accepts(
        "fn active () -> Set User = from u in users where exists i in u.<identities.user select u",
    );
    accepts(
        "fn f () -> Set User = from u in users where not exists i in u.<identities.user where i.user == key u select u",
    );
    let (s, p) = program("fn f (u: User) -> Bool = exists i in u.<identities.user").unwrap();
    assert!(matches!(
        p.terms.kind(body_of(&s, &p, "f")),
        Kind::Exists(_)
    ));
}

#[test]
fn and_or_not_are_ifs() {
    let (s, p) = program("fn f (a: Bool, b: Bool) -> Bool = a and not b").unwrap();
    let Kind::If(_, rhs, f) = p.terms.kind(body_of(&s, &p, "f")) else {
        panic!("an if");
    };
    assert!(matches!(p.terms.kind(*f), Kind::Ctor { ctor: 0, .. }));
    assert!(matches!(p.terms.kind(*rhs), Kind::If(..)));
}

#[test]
fn set_constructor_selection() {
    accepts(
        "fn sign_ins () -> Set (Local { id: Key identities, email: Text }
                              | Remote { id: Key identities, provider: Text }) =
           from i in identities select match i
             | EmailPass e -> Local  { id: key e, email: e.email }
             | Google g    -> Remote { id: key g, provider: \"google\" }
             | Apple a     -> Remote { id: key a, provider: \"apple\" }
         fn remote () -> Set Text = from l in sign_ins().Remote select l.provider",
    );
    accepts("fn f () -> Set (Identity.OAuth) = (from i in identities select i).OAuth");
    refuses(
        "fn f () -> Set Identity = (from i in identities.Google select i).Apple",
        "is not among the constructors of Identity.Google",
    );
    refuses(
        "fn f (i: Identity) -> Identity = i.Google",
        "`.C` selects constructors of a set",
    );
}

#[test]
fn a_result_sum_selection_has_a_row_type() {
    accepts(
        "fn rs () -> Set (A { n: I64 } | B { m: Text }) = [A { n: 1 }, B { m: \"x\" }]
         fn f () -> Set (A { n: I64 } | B { m: Text }) = rs().A
         fn g () -> Set I64 = from a in rs().A select a.n",
    );
    refuses(
        "fn rs () -> Set (A { n: I64 } | B { m: Text }) = [A { n: 1 }, B { m: \"x\" }]
         fn g () -> Set I64 = from a in rs().(A | B) select a.n",
        "has no field `n`",
    );
}

#[test]
fn constructor_relations_are_relations() {
    let (s, p) = program("fn f () -> Set (Identity.Google) = identities.Google").unwrap();
    assert!(matches!(
        p.terms.kind(body_of(&s, &p, "f")),
        Kind::Rel(_, Some(_))
    ));
    let (s, p) = program("fn rs () -> Set Identity = from i in identities select i\nfn f () -> Set (Identity.Google) = rs().Google").unwrap();
    assert!(matches!(
        p.terms.kind(body_of(&s, &p, "f")),
        Kind::Restrict(..)
    ));
}

// Result sums

#[test]
fn result_sums_are_checked_against_the_signature() {
    query_accepts(
        "query login_key (u: Key users) -> Set (ByEmail { email: Text } | ByGoogle { gid: Text }) =
           from i in identities.(EmailPass | Google) where i.user == u
           select match i
             | EmailPass e -> ByEmail { email: e.email }
             | Google g    -> ByGoogle { gid: g.gid }",
    );
    query_refuses(
        "query login_key (u: Key users) -> Set (ByEmail { email: Text } | ByGoogle { gid: Text }) =
           from i in identities.(EmailPass | Google) where i.user == u
           select match i
             | EmailPass e -> ByEmail { email: e.email }
             | Google g    -> ByApple { gid: g.gid }",
        "ByApple",
    );
}

#[test]
fn adhoc_result_sums_are_inferred() {
    assert_eq!(
        adhoc_ty(
            &[],
            "from i in identities select match i
               | EmailPass e -> ByEmail { email: e.email }
               | Google g    -> ByGoogle { gid: g.gid }
               | Apple a     -> ByGoogle { gid: a.sub }"
        ),
        "Set (ByEmail { email: Text } | ByGoogle { gid: Text })"
    );
    adhoc_refuses(
        &[],
        "from i in identities select match i
           | EmailPass e -> A { x: e.email }
           | Google g    -> A { x: 1 }
           | Apple a     -> B",
        "no common type",
    );
}

#[test]
fn a_result_sum_is_matched_exhaustively() {
    accepts(
        "fn rs () -> Set (A { n: I64 } | B { m: Text }) = [A { n: 1 }]
         fn f () -> Set I64 = from r in rs() select match r | A a -> a.n | B b -> 0",
    );
    refuses(
        "fn rs () -> Set (A { n: I64 } | B { m: Text }) = [A { n: 1 }]
         fn f () -> Set I64 = from r in rs() select match r | A a -> a.n",
        "match does not cover `B _`",
    );
    accepts(
        "fn rs () -> Set (A { n: I64 } | B { m: Text } | C) = [C]
         fn f () -> Set Text = from r in rs().(B | C) select match r | B b -> b.m | C -> \"c\"",
    );
    refuses(
        "fn rs () -> Set (A { n: I64 } | B { m: Text } | C) = [C]
         fn f () -> Set Text = from r in rs().(B | C) select match r | B b -> b.m | C -> \"c\" | A _ -> \"a\"",
        "is not among the constructors",
    );
}

#[test]
fn or_binders_over_result_sums_get_the_row_type() {
    accepts(
        "fn rs () -> Set (A { n: I64 } | B { n: I64 } | C) = [C]
         fn f () -> Set I64 = from r in rs() select match r | A x | B x -> x.n | C -> 0",
    );
}

// Match

#[test]
fn a_match_on_a_relation_row_is_a_case_per_constructor() {
    let (s, p) = program("fn f (u: Key users) -> Set Text = from i in identities where i.user == u select display_name(i)").unwrap();
    let _ = body_of(&s, &p, "f");
    let i = s
        .functions
        .iter()
        .position(|f| f.name == "display_name")
        .unwrap();
    let Kind::Lam(_, body) = p.terms.kind(p.functions[i].unwrap()) else {
        panic!("a lam");
    };
    let Kind::Let(_, _, case) = p.terms.kind(*body) else {
        panic!("a let, found {:?}", p.terms.kind(*body));
    };
    let Kind::Case { arms, .. } = p.terms.kind(*case) else {
        panic!("a case, found {:?}", p.terms.kind(*case));
    };
    assert_eq!(
        arms.iter().map(|(c, _, _)| *c).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    for (_, x, arm) in arms {
        let Kind::Let(z, y, _) = p.terms.kind(*arm) else {
            panic!("an arm binds its binder, found {:?}", p.terms.kind(*arm));
        };
        assert_ne!(z, x);
        assert_eq!(p.terms.kind(*y), &Kind::Var(*x));
    }
}

#[test]
fn a_match_on_a_constructor_set_cases_over_the_set() {
    let (s, p) = program(
        "fn f (o: Identity.OAuth) -> Text = match o | Google g -> g.gid | Apple a -> a.sub",
    )
    .unwrap();
    let Kind::Let(_, _, case) = p.terms.kind(body_of(&s, &p, "f")) else {
        panic!("a let");
    };
    let Kind::Case { arms, .. } = p.terms.kind(*case) else {
        panic!("a case");
    };
    assert_eq!(
        arms.iter().map(|(c, _, _)| *c).collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn a_guarded_chain_ends_unreachable() {
    let (s, p) = program(
        "fn f (i: Identity) -> Text = match i
           | Google g if g.gid == \"corp\" -> \"staff\"
           | Google _                    -> \"guest\"
           | EmailPass _                 -> \"guest\"
           | Apple _                     -> \"guest\"",
    )
    .unwrap();
    let Kind::Let(_, _, case) = p.terms.kind(body_of(&s, &p, "f")) else {
        panic!("a let");
    };
    let Kind::Case { arms, .. } = p.terms.kind(*case) else {
        panic!("a case");
    };
    let google = arms.iter().find(|(c, _, _)| *c == 1).unwrap().2;
    let Kind::Let(_, _, guarded) = p.terms.kind(google) else {
        panic!("the guarded arm binds g, found {:?}", p.terms.kind(google));
    };
    let Kind::If(_, _, rest) = p.terms.kind(*guarded) else {
        panic!("a guard is an if");
    };
    assert!(matches!(p.terms.kind(*rest), Kind::Const(_)));
}

#[test]
fn a_nested_pattern_cases_on_the_field() {
    let (s, p) = program(
        "fn f (s: Shipment) -> Text = match s
           | Shipped { via: Courier c } -> c.name
           | Shipped { via: Mail m }    -> m.tracking
           | Pending                    -> \"pending\"",
    )
    .unwrap();
    let Kind::Let(_, _, case) = p.terms.kind(body_of(&s, &p, "f")) else {
        panic!("a let");
    };
    let Kind::Case { arms, .. } = p.terms.kind(*case) else {
        panic!("a case");
    };
    let shipped = arms.iter().find(|(c, _, _)| *c == 1).unwrap().2;
    let Kind::Case {
        scrutinee,
        arms: via,
    } = p.terms.kind(shipped)
    else {
        panic!("a case on the field, found {:?}", p.terms.kind(shipped));
    };
    assert!(matches!(p.terms.kind(*scrutinee), Kind::Field(_, f) if f == "via"));
    assert_eq!(via.len(), 2);
    let Kind::Case { arms: rest, .. } = p.terms.kind(via[1].2) else {
        panic!("the mail arm continues with the next arm's case");
    };
    assert!(matches!(p.terms.kind(rest[0].2), Kind::Unreachable));
}

#[test]
fn an_or_pattern_shares_its_body() {
    let (s, p) = program(
        "fn f (i: Identity) -> Key users = match i | Google g | Apple g -> g.user | EmailPass e -> e.user",
    )
    .unwrap();
    let Kind::Let(_, _, case) = p.terms.kind(body_of(&s, &p, "f")) else {
        panic!("a let");
    };
    let Kind::Case { arms, .. } = p.terms.kind(*case) else {
        panic!("a case");
    };
    let bodies: Vec<(Var, Tm)> = arms
        .iter()
        .filter(|(c, _, _)| *c != 0)
        .map(|(_, _, t)| match p.terms.kind(*t) {
            Kind::Let(v, _, b) => (*v, *b),
            k => panic!("a let, found {k:?}"),
        })
        .collect();
    assert_eq!(bodies[0], bodies[1]);
}

#[test]
fn a_scalar_match_is_a_chain_without_case() {
    let (s, p) =
        program("fn f (c: Text) -> I64 = match c | \"US\" -> 0 | \"CA\" -> 1 | _ -> 2").unwrap();
    let Kind::Let(_, _, chain) = p.terms.kind(body_of(&s, &p, "f")) else {
        panic!("a let");
    };
    let Kind::If(test, _, rest) = p.terms.kind(*chain) else {
        panic!("an if, found {:?}", p.terms.kind(*chain));
    };
    assert!(matches!(p.terms.kind(*test), Kind::Prim(Prim::Eq, _)));
    assert!(matches!(p.terms.kind(*rest), Kind::If(..)));
}

#[test]
fn a_match_on_a_key_derefs_it() {
    let (s, p) =
        program("fn f (r: Reset) -> Text = match r.identity | EmailPass e -> e.email").unwrap();
    let Kind::Let(_, scrutinee, _) = p.terms.kind(body_of(&s, &p, "f")) else {
        panic!("a let");
    };
    assert!(matches!(
        p.terms.kind(*scrutinee),
        Kind::Deref {
            reference: true,
            ..
        }
    ));
    let (s, p) =
        program("fn f (k: Key identities.EmailPass) -> Text = match k | EmailPass e -> e.email")
            .unwrap();
    let Kind::Let(_, scrutinee, _) = p.terms.kind(body_of(&s, &p, "f")) else {
        panic!("a let");
    };
    assert!(matches!(
        p.terms.kind(*scrutinee),
        Kind::Deref {
            reference: false,
            ..
        }
    ));
}

#[test]
fn a_match_over_a_relation_is_a_union_of_arm_queries() {
    accepts(
        "fn f (u: Key users) -> Set Text = from i in identities where i.user == u
           select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub
         fn g (u: Key users) -> Set Text =
           (from e in identities.EmailPass where e.user == u select e.email)
           union (from g in identities.Google where g.user == u select g.gid)
           union (from a in identities.Apple where a.user == u select a.sub)",
    );
}

#[test]
fn match_is_an_expression_anywhere() {
    accepts(
        "fn f () -> Set Text = from i in identities
           where match i | Google g -> true | Apple a -> true | EmailPass e -> false
           select display_name(i)",
    );
    accepts(
        "fn f () -> I64 = count(x for x in (from i in identities
           select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub))",
    );
    accepts(
        "fn f () -> Set ({ owner: Key users, login: Text }) = from i in identities select match i
           | EmailPass e -> { owner: e.user, login: e.email }
           | Google g    -> { owner: g.user, login: g.gid }
           | Apple a     -> { owner: a.user, login: a.sub }",
    );
}

#[test]
fn if_is_a_match_on_bool() {
    accepts("fn f (b: Bool) -> I64 = if b then 1 else 0");
    accepts("fn f (b: Bool) -> I64 = match b | true -> 1 | false -> 0");
    accepts("fn f (b: Bool) -> I64 = match b | True -> 1 | False -> 0");
    refuses(
        "fn f (b: Bool) -> I64 = match b | true -> 1",
        "match does not cover `False`",
    );
    refuses("fn f (b: I64) -> I64 = if b then 1 else 0", "expected Bool");
}

#[test]
fn a_match_on_a_tag() {
    accepts("fn f (i: Identity) -> I64 = match tag(i) | EmailPass -> 1 | Google -> 2 | Apple -> 3");
    refuses(
        "fn f (i: Identity) -> I64 = match tag(i) | EmailPass -> 1 | Google -> 2",
        "match does not cover `Apple`",
    );
}

#[test]
fn match_scrutinees_are_values() {
    refuses(
        "fn f () -> I64 = match users | User u -> 1",
        "a set is ranged over",
    );
}

// Patterns and binder types

#[test]
fn a_constructor_binder_has_the_row_type() {
    accepts(
        "fn f (i: Identity) -> Identity = match i | Google g -> g | EmailPass e -> e | Apple a -> a",
    );
    accepts(
        "fn f (i: Identity) -> Set (Identity.Google) = match i | Google g -> [g] | EmailPass _ -> [] | Apple _ -> []",
    );
    accepts(
        "fn f (i: Identity) -> Set (Key identities.Google) = match i
           | Google g -> [key g] | EmailPass _ -> [] | Apple _ -> []",
    );
    refuses(
        "fn f (o: Option I64) -> I64 = match o | Some s -> key s | None -> 0",
        "`key` needs a row",
    );
}

#[test]
fn or_patterns_bind_the_join_of_their_alternatives() {
    accepts(
        "fn f (i: Identity) -> Key users = match i | Google g | Apple g -> g.user | EmailPass e -> e.user",
    );
    accepts(
        "fn f (i: Identity) -> Identity = match i | Google g | Apple g -> g | EmailPass e -> e",
    );
    refuses(
        "fn f (i: Identity) -> Identity.Google = match i | Google g | Apple g -> g | EmailPass e -> e",
        "expected Identity.Google, found Identity.(Google | Apple)",
    );
    accepts(
        "fn f (s: Status, t: Instant) -> Instant = match s | Sent { at } | Paid { at } -> at | Draft -> t",
    );
}

#[test]
fn or_patterns_whose_binders_have_no_join_are_refused() {
    refuses(
        "type Mixed = A { v: I64 } | B { v: Text }
         fn f (m: Mixed) -> I64 = match m | A { v } | B { v } -> 1",
        "no common type across the alternatives",
    );
    refuses(
        "fn f (i: Identity) -> Key users = match i | EmailPass e | Google g -> e.user | Apple a -> a.user",
        "binds the same names",
    );
}

#[test]
fn named_set_patterns() {
    accepts(
        "fn f (i: Identity) -> Text = match i
           | EmailPass e -> e.email
           | OAuth o     -> display_name(o)",
    );
    accepts(
        "fn f (i: Identity) -> Key users = match i | OAuth o -> o.user | EmailPass e -> e.user",
    );
    refuses(
        "fn f (i: Identity) -> Text = match i | EmailPass e -> e.email | OAuth o -> o.gid",
        "has no field `gid`",
    );
    refuses(
        "fn f (i: Identity) -> Text = match i | OAuth o -> display_name(o) | Google g -> g.gid | EmailPass e -> e.email",
        "unreachable",
    );
}

#[test]
fn record_patterns() {
    accepts(
        "fn f (i: Identity) -> Text = match i
           | EmailPass { email, hash: _ } -> email
           | Google { gid }               -> gid
           | Apple { sub }                -> sub",
    );
    accepts("fn f (u: User) -> Text = match u | User { name } -> name");
    accepts(
        "fn f (i: Identity) -> Key users = match i | EmailPass { user } -> user | Google { user } -> user | Apple { user } -> user",
    );
    refuses(
        "fn f (i: Identity) -> Text = match i | EmailPass { nope } -> nope | Google g -> g.gid | Apple a -> a.sub",
        "has no field `nope`",
    );
}

#[test]
fn nested_patterns() {
    accepts(
        "fn f (s: Shipment) -> Text = match s
           | Shipped { via: Courier c } -> c.name
           | Shipped { via: Mail m }    -> m.tracking
           | Pending                    -> \"pending\"",
    );
    refuses(
        "fn f (s: Shipment) -> Text = match s
           | Shipped { via: Courier c } -> c.name
           | Pending                    -> \"pending\"",
        "match does not cover `Shipped { via: Mail _ }`",
    );
    accepts(
        "fn f (s: Shipment) -> Text = match s
           | Shipped { via: Courier { name: \"x\" } } -> \"x\"
           | Shipped { via: Courier c } -> c.name
           | Shipped { via: Mail m }    -> m.tracking
           | Pending                    -> \"pending\"",
    );
}

#[test]
fn literal_patterns_take_the_scrutinee_type() {
    accepts("fn f (c: Text) -> I64 = match c | \"US\" -> 0 | \"CA\" -> 1 | _ -> 2");
    accepts("fn f (n: I32) -> I64 = match n | 1 -> 0 | -1 -> 1 | x -> 2");
    refuses(
        "fn f (n: I8) -> I64 = match n | 300 -> 0 | _ -> 1",
        "out of range",
    );
    refuses(
        "fn f (c: Text) -> I64 = match c | 1 -> 0 | _ -> 1",
        "expected Text",
    );
    refuses(
        "fn f (s: Status) -> I64 = match s | 1 -> 0 | _ -> 1",
        "a literal pattern matches a scalar",
    );
}

#[test]
fn a_scalar_match_ends_with_a_wildcard_or_binder() {
    refuses(
        "fn f (c: Text) -> I64 = match c | \"US\" -> 0 | \"CA\" -> 1",
        "end it with `_` or a binder",
    );
    refuses(
        "fn f (b: Bool, n: I64) -> I64 = match n | 0 -> 0",
        "end it with `_` or a binder",
    );
    accepts("type Email = new Text\nfn f (e: Email) -> I64 = match e | _ -> 0");
}

#[test]
fn a_constructor_pattern_resolves_against_the_scrutinee() {
    refuses(
        "fn f (i: Identity) -> I64 = match i | Pending -> 1",
        "`Pending` is not a constructor of Identity",
    );
    refuses(
        "fn f (o: Identity.OAuth) -> I64 = match o | Google g -> 1 | Apple a -> 2 | EmailPass e -> 3",
        "`EmailPass` is not among the constructors of Identity.(Google | Apple)",
    );
    accepts("fn f (o: Identity.(Google | Apple)) -> I64 = match o | Google g -> 1 | Apple a -> 2");
    refuses(
        "fn f (c: Text) -> I64 = match c | Google g -> 1 | _ -> 2",
        "has no constructors",
    );
}

#[test]
fn a_bare_constructor_is_nullary() {
    refuses(
        "fn f (i: Identity) -> I64 = match i | Google -> 1 | EmailPass _ -> 2 | Apple _ -> 3",
        "`Google _`",
    );
    accepts("fn f (s: Status) -> I64 = match s | Draft -> 1 | Sent _ -> 2 | Paid p -> 3");
}

// Exhaustiveness

#[test]
fn a_match_names_every_constructor() {
    refuses(
        "fn f (i: Identity) -> Text = match i | EmailPass e -> e.email | Google g -> g.gid",
        "match does not cover `Apple _`",
    );
    accepts(
        "fn f (i: Identity) -> Text = match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub",
    );
}

#[test]
fn a_match_over_a_constructor_set_covers_the_set() {
    accepts(
        "fn f () -> Set Text = from i in identities.(Google | Apple) select match i | Google g -> g.gid | Apple a -> a.sub",
    );
    refuses(
        "fn f () -> Set Text = from i in identities.(Google | Apple) select match i | Google g -> g.gid",
        "match does not cover `Apple _`",
    );
}

#[test]
fn wildcards_do_not_match_a_sum() {
    refuses(
        "fn f (i: Identity) -> Text = match i | Google g -> g.gid | _ -> \"other\"",
        "do not match a value of a sum",
    );
    refuses(
        "fn f (i: Identity) -> Text = match i | x -> display_name(x)",
        "do not match a value of a sum",
    );
    refuses(
        "fn f (u: User) -> Text = match u | x -> x.name",
        "do not match a value of a sum",
    );
}

#[test]
fn adhoc_wildcards_cover_the_remaining_constructors() {
    adhoc_accepts(
        &[("i", "Identity")],
        "match i | Google g -> g.gid | _ -> \"other\"",
    );
    adhoc_accepts(
        &[],
        "from i in identities select match i | Google g -> g.gid | _ -> \"other\"",
    );
    adhoc_accepts(
        &[("i", "Identity")],
        "match i | Google g -> g.gid | x -> display_name(x)",
    );
    adhoc_refuses(
        &[("i", "Identity")],
        "match i | Google g -> g.gid | _ -> \"other\" | Apple a -> a.sub",
        "unreachable",
    );
    adhoc_refuses(
        &[("i", "Identity")],
        "match i | Google g -> g.gid",
        "match does not cover",
    );
}

#[test]
fn nested_wildcards_at_a_discriminated_position() {
    refuses(
        "fn f (s: Shipment) -> Text = match s
           | Shipped { via: Courier c } -> c.name
           | Shipped _                  -> \"other\"
           | Pending                    -> \"pending\"",
        "another arm names a constructor here",
    );
    refuses(
        "fn f (s: Shipment) -> Text = match s
           | Shipped { via: Courier c } -> c.name
           | Shipped { via: v }         -> \"other\"
           | Pending                    -> \"pending\"",
        "another arm names a constructor here",
    );
    accepts(
        "fn f (s: Shipment) -> Text = match s
           | Shipped { via: v } -> \"shipped\"
           | Pending            -> \"pending\"",
    );
    accepts(
        "fn f (s: Shipment) -> Text = match s
           | Shipped _ -> \"shipped\"
           | Pending   -> \"pending\"",
    );
    adhoc_accepts(
        &[("s", "Shipment")],
        "match s | Shipped { via: Courier c } -> c.name | Shipped _ -> \"other\" | Pending -> \"pending\"",
    );
}

#[test]
fn guarded_arms_count_for_nothing() {
    accepts(
        "fn f (i: Identity) -> Text = match i
           | Google g if g.gid == \"corp\" -> \"staff\"
           | Google _                    -> \"guest\"
           | EmailPass _                 -> \"guest\"
           | Apple _                     -> \"guest\"",
    );
    refuses(
        "fn f (i: Identity) -> Text = match i
           | Google g if g.gid == \"root\" -> \"staff\"
           | EmailPass _                 -> \"guest\"
           | Apple _                     -> \"guest\"",
        "match does not cover `Google _`",
    );
    refuses(
        "fn f (c: Text) -> I64 = match c | x if x == \"a\" -> 1",
        "end it with `_` or a binder",
    );
}

#[test]
fn unreachable_arms_are_refused() {
    refuses(
        "fn f (i: Identity) -> I64 = match i | Google _ -> 1 | Google g -> 2 | EmailPass _ -> 3 | Apple _ -> 4",
        "unreachable",
    );
    refuses(
        "fn f (i: Identity) -> I64 = match i | Google _ -> 1 | Google g if g.gid == \"x\" -> 2 | EmailPass _ -> 3 | Apple _ -> 4",
        "unreachable",
    );
    refuses(
        "fn f (c: Text) -> I64 = match c | _ -> 1 | \"US\" -> 2",
        "unreachable",
    );
    refuses(
        "fn f (c: Text) -> I64 = match c | \"US\" -> 1 | \"US\" -> 2 | _ -> 3",
        "unreachable",
    );
    accepts("fn f (c: Text) -> I64 = match c | \"US\" if true -> 1 | \"US\" -> 2 | _ -> 3");
}

#[test]
fn a_key_scrutinee_matches_the_referenced_row() {
    accepts(
        "fn f () -> Set Text = from r in resets select match r.identity | EmailPass e -> e.email",
    );
    accepts(
        "fn f (k: Key identities) -> Key identities = match k | Google g -> key g | EmailPass e -> key e | Apple a -> key a",
    );
    refuses(
        "fn f (k: Key identities) -> Text = match k | Google g -> g.gid | EmailPass e -> e.email",
        "match does not cover `Apple _`",
    );
    refuses(
        "fn f () -> Set Text = from r in resets select match r.identity | e -> e.email",
        "not patterns for a key",
    );
    adhoc_refuses(
        &[],
        "from r in resets select match r.identity | _ -> \"x\"",
        "not patterns for a key",
    );
    accepts(
        "fn f (r: Reset) -> Key identities.EmailPass = match r.identity | EmailPass e -> key e",
    );
}

#[test]
fn nested_patterns_do_not_follow_references() {
    refuses(
        "fn f (r: Reset) -> Text = match r | Reset { identity: EmailPass e } -> e.email",
        "does not follow a reference",
    );
    accepts("fn f (r: Reset) -> Key identities.EmailPass = match r | Reset { identity: k } -> k");
}

#[test]
fn option_matches() {
    accepts("fn or_else [a] (p: Option a, d: a) -> a = match p | None -> d | Some s -> s.value");
    refuses(
        "fn f (p: Option I64) -> I64 = match p | Some s -> s.value",
        "match does not cover `None`",
    );
    accepts(
        "fn f (p: Option (Option I64)) -> I64 = match p | None -> 0 | Some { value: None } -> 1 | Some { value: Some { value: n } } -> n",
    );
    refuses(
        "fn f (p: Option (Option I64)) -> I64 = match p | None -> 0 | Some { value: Some { value: n } } -> n",
        "match does not cover `Some { value: None }`",
    );
}

// Grouping and aggregates

#[test]
fn group_into_binds_key_and_set() {
    accepts(
        "fn f () -> Set ({ user: Key users, n: I64 }) =
           from i in identities group i by i.user into g
           select { user: g.key, n: count(x for x in g) }",
    );
    accepts(
        "fn f () -> Set ({ kind: Tag Identity, n: I64 }) =
           from i in identities group i by tag(i) into g
           select { kind: g.key, n: count(x for x in g) }",
    );
    accepts(
        "fn f () -> Set ({ user: Key users, bulk: Bool }) =
           from o in orders group o by { user: o.user, bulk: o.qty > 10 } into g select g.key",
    );
    accepts("fn f () -> Set (Set Order) = from o in orders group o by o.user into g select g");
    accepts(
        "fn f () -> Set (Key users) = from o in orders group o by o.user into g where count(x for x in g) > 1 select g.key",
    );
    refuses(
        "fn f () -> Set Text = from o in orders group o by o.user into g select g.name",
        "a group binder has only `key`",
    );
}

#[test]
fn a_group_desugars_to_group_over_binder_records() {
    let (s, p) = program(
        "fn f () -> Set (Key users) = from o in orders group o by o.user into g select g.key",
    )
    .unwrap();
    let Kind::For(_, src, _) = p.terms.kind(body_of(&s, &p, "f")) else {
        panic!("a for");
    };
    let Kind::Group { source, .. } = p.terms.kind(*src) else {
        panic!("a group, found {:?}", p.terms.kind(*src));
    };
    assert!(matches!(p.terms.kind(*source), Kind::For(..)));
}

#[test]
fn aggregates_over_a_group_are_nonempty() {
    accepts(
        "fn f () -> Set ({ user: Key users, top: I64 }) =
           from o in orders group o by o.user into g
           select { user: g.key, top: max(x.total for x in g) }",
    );
    accepts(
        "fn f () -> Set ({ name: Text, top: Option I64 }) = from u in users
           select { name: u.name,
                    top: max(o.total for o in orders where o.user == key u) }",
    );
    refuses(
        "fn f () -> Set ({ user: Key users, top: I64 }) =
           from o in orders group o by o.user into g
           select { user: g.key, top: max(x.total for x in g where x.qty > 1) }",
        "expected",
    );
    accepts(
        "fn f () -> Set (Option F64) = from o in orders group o by o.user into g select avg(f64(x.total) for x in g where x.qty > 1)",
    );
    accepts(
        "fn f () -> Set F64 = from o in orders group o by o.user into g select avg(f64(x.total) for x in g)",
    );
}

#[test]
fn aggregate_types() {
    accepts("fn f (u: User) -> I64 = sum(o.total for o in orders where o.user == key u)");
    accepts("fn f () -> I64 = count(i for i in identities)");
    accepts("fn f () -> Option Instant = min(p.created_at for p in posts)");
    refuses(
        "fn f () -> Text = sum(u.name for u in users)",
        "`sum` adds numbers",
    );
    refuses(
        "fn f () -> Option I64 = avg(o.total for o in orders)",
        "`avg` averages `F32` or `F64`",
    );
    refuses(
        "fn f () -> Option Status = max(i.status for i in invoices)",
        "needs an ordered type",
    );
    accepts(
        "fn f () -> I64 = count(x for x in (from i in identities
           select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub))",
    );
    accepts("fn f () -> I64 = count(a for a in (from p in posts select p.author))");
    accepts("fn f () -> I64 = count(1 for u in users, p in posts where p.author == key u)");
}

#[test]
fn sum_refuses_a_projection() {
    accepts("fn f () -> I64 = sum(o.total for o in orders)");
    accepts("fn f () -> I64 = sum(x.total for x in (from o in orders where o.qty > 1 select o))");
    refuses(
        "fn f () -> I64 = sum(t for t in (from o in orders select o.total))",
        "`sum` ranges over rows",
    );
    accepts("fn f (xs: Set I64) -> I64 = sum(x for x in xs)");
    accepts("fn f (u: User) -> I64 = sum(o.total for o in u.<orders.user)");
}

#[test]
fn an_aggregate_is_an_aggregate_node() {
    let (s, p) = program("fn f () -> I64 = sum(o.total for o in orders where o.qty > 1)").unwrap();
    let Kind::Agg { source, .. } = p.terms.kind(body_of(&s, &p, "f")) else {
        panic!("an agg");
    };
    assert!(matches!(p.terms.kind(*source), Kind::For(..)));
}

// Ordering and limits

#[test]
fn order_by_on_the_outermost_query() {
    adhoc_accepts(&[], "from u in users select u order by u.name");
    query_accepts("query q () -> Set User = from u in users select u order by u.name");
    query_accepts("query q () -> Set Text = from u in users select u.name order by u.name desc");
    refuses(
        "fn f () -> Set User = from u in users select u order by u.name",
        "`order by` is allowed only on the outermost comprehension",
    );
    query_refuses(
        "query q () -> Set (Set User) = from x in users select (from u in users select u order by u.name)",
        "`order by` is allowed only on the outermost comprehension",
    );
}

#[test]
fn an_ordering_key_is_a_function_of_the_selected_value() {
    adhoc_refuses(
        &[],
        "from u in users, p in posts select p order by u.name",
        "an ordering key reads only the selected value",
    );
    adhoc_accepts(&[], "from u in users select { n: u.name } order by u.name");
    adhoc_refuses(
        &[],
        "from i in invoices select i order by i.status",
        "an ordering key has an ordered type",
    );
    adhoc_refuses(
        &[],
        "from u in users select { n: u.name } order by { n: u.name }",
        "an ordering key has an ordered type",
    );
}

#[test]
fn take_and_after_need_a_determinate_order() {
    query_accepts(
        "query page (cursor: Option (Key users)) -> Set User =
           from u in users select u order by u.id take 50 after cursor",
    );
    query_accepts(
        "query feed_page (cursor: Option ({ at: Instant, id: Key posts })) -> Set Post =
           from p in posts select p order by (p.created_at desc, key p) take 20 after cursor",
    );
    query_refuses(
        "query q () -> Set User = from u in users select u order by u.name take 5",
        "need a determinate order",
    );
    adhoc_accepts(
        &[],
        "from i in identities.EmailPass select i order by i.email take 5",
    );
    adhoc_refuses(
        &[],
        "from i in identities select i order by i.user take 5",
        "need a determinate order",
    );
    adhoc_accepts(
        &[],
        "from p in posts select p.author order by p.author take 5",
    );
    query_refuses(
        "query feed_page (cursor: Option ({ at: Instant, id: Key users })) -> Set Post =
           from p in posts select p order by (p.created_at desc, key p) take 20 after cursor",
        "field `id` of the `after` record",
    );
    query_refuses(
        "query page (cursor: Option Text) -> Set User =
           from u in users select u order by u.id take 50 after cursor",
        "expected Option (Key users)",
    );
}

#[test]
fn top_is_a_set_at_any_depth() {
    accepts(
        "fn f () -> Set ({ user: User, latest: Set Post }) = from u in users
           select { user: u,
                    latest: top 3 by (p.created_at desc, key p)
                              (from p in posts where p.author == key u select p) }",
    );
    refuses(
        "fn f (u: User) -> Set Post = top 3 by p.created_at (from p in posts where p.author == key u select p)",
        "`top` needs a determinate order",
    );
    refuses(
        "fn f (xs: Set Post) -> Set Post = top 3 by 1 (xs)",
        "the operand of `top` is a comprehension",
    );
    let (s, p) =
        program("fn f () -> Set Post = top 3 by key p (from p in posts select p)").unwrap();
    assert!(matches!(
        p.terms.kind(body_of(&s, &p, "f")),
        Kind::Top { .. }
    ));
}

// Recursion

#[test]
fn fix_binder_types_come_from_base_operands() {
    accepts(
        "fn f (me: Key users) -> Set User =
           fix reach = (from f in follows where f.src == me select f.dst)
                     union (from r in reach, f in follows where f.src == r select f.dst)
           in from u in users where exists r in reach where r == key u select u",
    );
    accepts(
        "fn f (me: Key users) -> Set User =
           fix even = (from u in users where key u == me select key u)
                      union (from o in odd, f in follows where f.src == o select f.dst),
               odd  = from v in even, f in follows where f.src == v select f.dst
           in from u in users where exists o in odd where o == key u select u",
    );
    accepts(
        "fn f (me: Key users) -> Set User =
           fix reach = (from f in follows where f.src == me select f.dst)
                     union (from r in reach, f in follows where f.src == r select f.dst)
           in from u in users where not exists r in reach where r == key u select u",
    );
    refuses(
        "fn f () -> Set (Key users) = fix a = from x in a select x in a",
        "the type of `a` is not known",
    );
    refuses(
        "fn f (me: Key users) -> Set (Key users) =
           fix reach = (from f in follows where f.src == me select f.dst)
                     union (from r in reach select users.key(1).name)
           in reach",
        "the operands of `union` have no common type: Key users and Text",
    );
}

#[test]
fn fix_binder_types_widen_across_iterations() {
    accepts(
        "fn f () -> Set (Key identities.(Google | Apple)) =
           fix r = (from g in identities.Google select key g)
                 union (from x in r, a in identities.Apple select key a)
           in r",
    );
    refuses(
        "fn f () -> Set (Key identities.Google) =
           fix r = (from g in identities.Google select key g)
                 union (from x in r, a in identities.Apple select key a)
           in r",
        "expected Set (Key identities.Google)",
    );
    accepts(
        "fn f () -> Set (Key identities.OAuth) =
           fix a = (from g in identities.Google select key g) union b,
               b = (from x in a select x) union (from p in identities.Apple select key p)
           in a",
    );
    refuses(
        "fn f () -> Set (Key identities.Google) =
           fix a = (from g in identities.Google select key g) union b,
               b = (from x in a select x) union (from p in identities.Apple select key p)
           in a",
        "expected Set (Key identities.Google)",
    );
    refuses(
        "fn f () -> Set I64 =
           fix a = [1] union (from x in a select { v: x })
           in a",
        "no common type",
    );
}

#[test]
fn fix_is_a_kernel_fix() {
    let (s, p) = program(
        "fn f (me: Key users) -> Set (Key users) =
           fix reach = (from f in follows where f.src == me select f.dst)
                     union (from r in reach, f in follows where f.src == r select f.dst)
           in reach",
    )
    .unwrap();
    let Kind::Fix(binds, _) = p.terms.kind(body_of(&s, &p, "f")) else {
        panic!("a fix");
    };
    assert_eq!(binds.len(), 1);
}

#[test]
fn functions_do_not_recurse() {
    refuses("fn f () -> I64 = g()\nfn g () -> I64 = f()", "calls itself");
    refuses("fn f () -> I64 = f()", "calls itself");
}

// Functions, generics and lambdas

#[test]
fn functions_and_views() {
    accepts(
        "fn logins_of (u: Key users) -> Set Text =
           from i in identities where i.user == u select display_name(i)
         fn active () -> Set User = from u in users where exists i in u.<identities.user select u
         fn q () -> Set ({ name: Text, login: Text }) =
           from u in active(), l in logins_of(key u) select { name: u.name, login: l }",
    );
    refuses("fn f () -> I64 = display_name()", "takes 1 arguments");
}

#[test]
fn higher_order_functions_take_lambdas() {
    accepts(
        "fn having (p: Identity -> Bool) -> Set (Key users) =
           from i in identities where p(i) select i.user
         fn g () -> Set (Key users) =
           having(fn i -> match i | Google _ -> true | EmailPass _ -> false | Apple _ -> false)",
    );
    refuses(
        "fn f () -> I64 = count(x for x in [fn x -> x])",
        "is not known here",
    );
    accepts("fn f () -> Identity -> Text = display_name");
}

#[test]
fn generics_solve_from_arguments() {
    accepts(
        "fn or_else [a] (p: Option a, d: a) -> a = match p | None -> d | Some s -> s.value
         fn latest [a: Ord] (xs: Set a) -> Option a = max(x for x in xs)
         fn f (p: Option I32) -> I32 = or_else(p, 0)
         fn g () -> Option Instant = latest(from p in posts select p.created_at)",
    );
    refuses(
        "fn latest [a: Ord] (xs: Set a) -> Option a = max(x for x in xs)
         fn g (xs: Set ZonedDateTime) -> Option ZonedDateTime = latest(xs)",
        "does not satisfy the bound",
    );
    refuses(
        "fn or_else [a] (p: Option a, d: a) -> a = match p | None -> d | Some s -> s.value
         fn f (p: Option I32) -> I32 = or_else(p, \"x\")",
        "expected I32",
    );
}

#[test]
fn generics_solve_from_the_expected_result() {
    accepts(
        "fn none [a] () -> Option a = None
         fn h () -> Option I64 = none()",
    );
    refuses(
        "fn none [a] () -> Option a = None
         fn h () -> I64 = count(x for x in [none()])",
        "the type argument `a` of `none` is not known here",
    );
}

#[test]
fn generic_joins_of_lower_bounds() {
    accepts(
        "fn pick [a] (x: a, y: a) -> a = x
         fn f (g: Identity.Google, e: Identity.EmailPass) -> Identity = pick(g, e)",
    );
}

#[test]
fn num_bounds() {
    accepts("fn twice [a: Num] (x: a) -> a = x + x");
    accepts("fn neg [a: Num] (x: a) -> a = -x");
    accepts("fn total [a: Num] (xs: Set a) -> a = sum(x for x in xs)");
    accepts("fn inc [a: Num] (x: a) -> a = x + 1");
    refuses(
        "fn sq [a: Num] (x: a) -> a = x * x",
        "`*` and division are defined per concrete type",
    );
    refuses("fn twice [a] (x: a) -> a = x + x", "does not apply");
    refuses(
        "fn twice [a: Num] (x: a) -> a = x + x
         fn f () -> Text = twice(\"x\")",
        "does not satisfy the bound",
    );
}

#[test]
fn ord_bounds() {
    accepts("fn lt [a: Ord] (x: a, y: a) -> Bool = x < y");
    refuses("fn lt [a] (x: a, y: a) -> Bool = x < y", "has no order");
    accepts("fn eq [a] (x: a, y: a) -> Bool = x == y");
}

#[test]
fn spans_need_an_ordered_argument() {
    accepts("fn f (a: Instant, b: Instant) -> Span Instant = span(a, b)");
    refuses(
        "fn f (a: ZonedDateTime, b: ZonedDateTime) -> Bool = start(span(a, b)) == a",
        "needs an ordered",
    );
    refuses(
        "fn f (s: Span ZonedDateTime) -> Bool = true",
        "needs an ordered",
    );
}

// Literals and numeric types

#[test]
fn integer_literals_take_the_expected_type() {
    accepts("fn f () -> I8 = 127");
    refuses("fn f () -> I8 = 128", "literal out of range of I8");
    accepts("fn f () -> I8 = -128");
    accepts("fn f () -> Decimal 2 = 3");
    accepts("fn f () -> F32 = 3");
    accepts("fn f () -> F64 = 3");
    assert_eq!(adhoc_ty(&[], "1"), "I64");
}

#[test]
fn fractional_literals() {
    accepts("fn f () -> Decimal 2 = 1.25");
    accepts("fn f () -> Decimal 3 = 1.25");
    refuses(
        "fn f () -> Decimal 1 = 1.25",
        "does not check against Decimal 1",
    );
    refuses(
        "fn f () -> I64 = 1.5",
        "a fractional literal does not check against I64",
    );
    accepts("fn f () -> F64 = 1.5");
    assert_eq!(adhoc_ty(&[], "1.25"), "Decimal 2");
}

#[test]
fn literal_operands_take_the_other_operand_type() {
    accepts("fn f () -> Set I32 = from s in sales select s.qty + 1");
    accepts("fn f () -> Set Bool = from s in sales select s.qty > 1");
    accepts("fn f (d: Decimal 2) -> Decimal 2 = d + 1.5");
    accepts("fn f (d: Decimal 2) -> Decimal 3 = d + 1.125");
    refuses(
        "fn f () -> Set I32 = from s in sales select s.qty + 1.5",
        "fractional",
    );
}

#[test]
fn arithmetic_has_one_numeric_type() {
    accepts("fn f () -> Set I64 = from s in sales select i64(s.qty) * s.unit_cents");
    refuses(
        "fn f () -> Set I64 = from s in sales select s.qty * s.unit_cents",
        "`*` does not apply to I32 and I64",
    );
    refuses(
        "fn f (a: Decimal 2, b: Decimal 2) -> Decimal 2 = a / b",
        "use `div`",
    );
    accepts("fn f (a: Decimal 2, b: Decimal 3) -> Decimal 5 = a * b");
    accepts("fn f (a: Decimal 2, b: Decimal 3) -> Decimal 3 = a + b");
}

#[test]
fn text_and_booleans() {
    assert_eq!(adhoc_ty(&[], "\"x\""), "Text");
    assert_eq!(adhoc_ty(&[], "true"), "Bool");
    accepts("fn f (a: Text, b: Text) -> Text = a ++ b");
    refuses(
        "fn f (a: Text, b: I64) -> Text = a ++ b",
        "`++` concatenates",
    );
}

// Prelude overloads

#[test]
fn prelude_overloads_resolve_by_argument_types() {
    accepts("fn f (x: I32) -> I32 = abs(x)");
    accepts("fn f (x: Decimal 2) -> Decimal 2 = abs(x)");
    refuses(
        "fn f (x: Text) -> Text = abs(x)",
        "`abs` does not take Text",
    );
    accepts("fn f (x: Decimal 2) -> I64 = i64(x, HalfEven)");
    refuses("fn f (x: Decimal 2) -> I64 = i64(x)", "takes a rounding");
    accepts("fn f (x: I32) -> I64 = i64(x)");
    accepts("fn f (x: I64) -> Option I32 = checked_i32(x)");
    accepts("fn f (x: I64) -> Text = text(x)");
    accepts("fn f (x: Instant) -> Text = text(x)");
    accepts("fn f (a: I64, b: I64) -> Option I64 = checked_div(a, b)");
    accepts("fn f (a: I64) -> Option I64 = checked_add(a, 1)");
    accepts("fn f (a: Decimal 2, b: Decimal 2) -> Decimal 4 = div(a, b, 4, HalfEven)");
    accepts("fn f (s: Span Instant, t: Instant) -> Bool = covers(s, t)");
    accepts("fn f (s: Span Instant, t: Span Instant) -> Bool = covers(s, t)");
    accepts("fn f (s: Span Instant, t: Span Instant) -> Bool = overlaps(s, t)");
    accepts("fn f (d: Date) -> Option Date = add_period(d, months(1), Reject)");
    accepts("fn f (d: Date) -> Date = add_period(d, months(1), Clamp)");
    accepts("fn f (t: Text) -> Option I64 = I64.parse(t)");
}

#[test]
fn checked_operators_return_option() {
    accepts(
        "fn f () -> Set I64 = from o in orders
           select match checked_div(o.total, o.qty) | None -> 0 | Some { value: q } -> q",
    );
}

#[test]
fn try_returns_a_result() {
    accepts("fn f () -> Set (Result I32 Fault) = from r in raws select try i32(r.n)");
    accepts(
        "fn f () -> Set ({ line: Key raws, n: Result I32 Fault }) = from r in raws select { line: key r, n: try i32(r.n) }",
    );
    accepts("fn f () -> Set Raw = from r in raws where r.n != 0 and 1000 / r.n > 10 select r");
}

#[test]
fn optional_filters_match_option_parameters() {
    query_accepts(
        "query search (name: Option Text, after: Option Instant) -> Set Post =
           from p in posts
           where match name  | None -> true | Some { value: n } -> p.title == n
           where match after | None -> true | Some { value: t } -> p.created_at > t
           select p",
    );
    query_accepts(
        "query by_status (s: Status, since: Instant) -> Set Invoice =
           from i in invoices where i.status == s and i.issued >= since select i",
    );
    query_refuses(
        "query q (p: I64 -> I64) -> I64 = p(1)",
        "never has a function type",
    );
}

// Constraints and other bodies

#[test]
fn constraints_are_set_queries() {
    accepts("constraint no_empty = from u in users where u.name == \"\" select u");
    refuses(
        "constraint bad = count(u for u in users)",
        "a constraint is a query returning a set",
    );
}

#[test]
fn writes_are_not_checked_yet() {
    adhoc_refuses(
        &[("u", "Key users"), ("g", "Text")],
        "insert identities.Google { user: u, gid: g }",
        "writes are not checked yet",
    );
}

// Least types of arms

#[test]
fn constructor_arms_meet_at_their_selection() {
    assert_eq!(
        adhoc_ty(
            &[("b", "Bool"), ("t", "Instant")],
            "if b then Draft else Sent { at: t }"
        ),
        "Status.(Draft | Sent)"
    );
    assert_eq!(
        adhoc_ty(
            &[("b", "Bool"), ("t", "Instant")],
            "match b | true -> Draft | false -> Sent { at: t }"
        ),
        "Status.(Draft | Sent)"
    );
    assert_eq!(
        adhoc_ty(
            &[("s", "Status")],
            "match s | Draft -> Draft | Sent x -> Draft | Paid p -> Draft"
        ),
        "Status.Draft"
    );
}

#[test]
fn a_generic_constructor_arm_takes_its_arguments_from_the_other_arms() {
    assert_eq!(
        adhoc_ty(&[("b", "Bool")], "if b then None else Some { value: 1 }"),
        "Option I64"
    );
    assert_eq!(
        adhoc_ty(&[("b", "Bool")], "if b then Some { value: 1 } else None"),
        "Option I64"
    );
    assert_eq!(
        adhoc_ty(
            &[("b", "Bool")],
            "match b | true -> None | false -> Some { value: 1 }"
        ),
        "Option I64"
    );
}

#[test]
fn arms_with_no_least_type_are_refused() {
    adhoc_refuses(
        &[("b", "Bool")],
        "match b | true -> None | false -> None",
        "the type argument `a` of `Option` is not known here",
    );
    adhoc_refuses(&[("b", "Bool")], "if b then None else 1", "expected");
    adhoc_refuses(
        &[("b", "Bool"), ("t", "Instant")],
        "if b then Draft else Pending",
        "no common type",
    );
}

// Compound cursors

#[test]
fn a_cursor_record_matches_the_keys_by_field_name_order() {
    query_accepts(
        "query feed_page (cursor: Option ({ zz: Key posts, aa: Instant })) -> Set Post =
           from p in posts select p order by (p.created_at desc, key p) take 20 after cursor",
    );
    query_refuses(
        "query feed_page (cursor: Option ({ aa: Key posts, zz: Instant })) -> Set Post =
           from p in posts select p order by (p.created_at desc, key p) take 20 after cursor",
        "field `aa` of the `after` record",
    );
}

// Redundant alternatives

#[test]
fn a_repeated_alternative_is_unreachable() {
    refuses(
        "fn f (i: Identity) -> I64 = match i | Google _ | Google _ -> 1 | EmailPass _ -> 2 | Apple _ -> 3",
        "unreachable",
    );
    refuses(
        "fn f (i: Identity) -> Key users = match i | OAuth o | Google o -> o.user | EmailPass e -> e.user",
        "unreachable",
    );
}

#[test]
fn an_alternative_covered_by_an_earlier_arm_is_unreachable() {
    refuses(
        "fn f (i: Identity) -> I64 = match i | Google _ -> 1 | Google _ | Apple _ -> 2 | EmailPass _ -> 3",
        "unreachable",
    );
    accepts(
        "fn f (i: Identity) -> I64 = match i | Google g if g.gid == \"x\" -> 1 | Google _ | Apple _ -> 2 | EmailPass _ -> 3",
    );
    refuses(
        "fn f (c: Text) -> I64 = match c | \"a\" -> 1 | \"b\" | \"a\" -> 2 | _ -> 3",
        "unreachable",
    );
}

// Unary functions in the kernel

#[test]
fn a_function_is_one_lam_per_parameter() {
    let (s, p) = program("fn add (a: I64, b: I64) -> I64 = a + b").unwrap();
    let i = s.functions.iter().position(|f| f.name == "add").unwrap();
    let Kind::Lam(_, inner) = p.terms.kind(p.functions[i].unwrap()) else {
        panic!("a lam");
    };
    let Kind::Lam(_, body) = p.terms.kind(*inner) else {
        panic!("a second lam, found {:?}", p.terms.kind(*inner));
    };
    assert!(matches!(p.terms.kind(*body), Kind::Prim(Prim::Add, _)));
}

#[test]
fn a_view_is_a_lam_over_unit() {
    let (s, p) = program("fn v () -> I64 = 1").unwrap();
    let i = s.functions.iter().position(|f| f.name == "v").unwrap();
    let Kind::Lam(x, _) = p.terms.kind(p.functions[i].unwrap()) else {
        panic!("a lam");
    };
    assert_eq!(p.terms.var_info(*x).ty, Ty::Unit);
}

#[test]
fn applying_a_function_value_nests_apps() {
    let (s, p) = program("fn ap (g: I64 -> I64 -> I64) -> I64 = g(1, 2)").unwrap();
    let Kind::App(inner, two) = p.terms.kind(body_of(&s, &p, "ap")) else {
        panic!("an app");
    };
    assert!(matches!(p.terms.kind(*two), Kind::Const(_)));
    let Kind::App(g, _) = p.terms.kind(*inner) else {
        panic!("a nested app, found {:?}", p.terms.kind(*inner));
    };
    assert!(matches!(p.terms.kind(*g), Kind::Var(_)));
    let (s, p) = program("fn ap (g: () -> I64) -> I64 = g()").unwrap();
    let Kind::App(_, unit) = p.terms.kind(body_of(&s, &p, "ap")) else {
        panic!("an app");
    };
    assert_eq!(p.terms.ty(*unit), &Ty::Unit);
}

#[test]
fn a_declared_call_keeps_its_arguments() {
    let (s, p) = program(
        "fn add (a: I64, b: I64) -> I64 = a + b
         fn h () -> I64 = add(1, 2)",
    )
    .unwrap();
    let Kind::Call { args, .. } = p.terms.kind(body_of(&s, &p, "h")) else {
        panic!("a call");
    };
    assert_eq!(args.len(), 2);
}

#[test]
fn a_function_named_without_a_call_is_nested_lams_over_its_call() {
    let (s, p) = program(
        "fn add (a: I64, b: I64) -> I64 = a + b
         fn h () -> I64 -> I64 -> I64 = add",
    )
    .unwrap();
    let Kind::Lam(_, inner) = p.terms.kind(body_of(&s, &p, "h")) else {
        panic!("a lam");
    };
    let Kind::Lam(_, call) = p.terms.kind(*inner) else {
        panic!("a second lam");
    };
    assert!(matches!(p.terms.kind(*call), Kind::Call { args, .. } if args.len() == 2));
}

#[test]
fn a_lambda_of_several_parameters_is_nested_lams() {
    let (s, p) = program("fn h () -> I64 -> I64 -> I64 = fn (a: I64, b: I64) -> a + b").unwrap();
    let Kind::Lam(_, inner) = p.terms.kind(body_of(&s, &p, "h")) else {
        panic!("a lam");
    };
    assert!(matches!(p.terms.kind(*inner), Kind::Lam(..)));
}

// Group is never written

#[test]
fn group_is_never_a_written_type() {
    refuses(
        "fn f (g: Group (Key users) Order) -> I64 = 1",
        "`Group K T` is the type of a group binder",
    );
    refuses(
        "fn f () -> Set (Group (Key users) Order) = from o in orders group o by o.user into g select g",
        "`Group K T` is the type of a group binder",
    );
    refuses(
        "fn f () -> I64 = count(x for x in [fn (g: Group I64 I64) -> 1])",
        "`Group K T` is the type of a group binder",
    );
    let err = query("", "query q (g: Group I64 I64) -> I64 = 1").unwrap_err();
    assert!(
        err.contains("`Group K T` is the type of a group binder"),
        "{err}"
    );
}

#[test]
fn a_group_is_passed_as_a_set() {
    accepts(
        "fn n (xs: Set Order) -> I64 = count(x for x in xs)
         fn f () -> Set I64 = from o in orders group o by o.user into g select n(g)",
    );
}
