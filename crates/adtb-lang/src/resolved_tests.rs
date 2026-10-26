use super::*;
use crate::parse::parse_migration;

const FIRST: &str = r#"module auth
add module auth
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
add type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text, hash: Bytes }
  | Google    { gid: Text }
  | Apple     { sub: Text }
add relation identities : Identity
  key id generate uuidv7
  unique (email) in EmailPass
add set OAuth of identities = Google | Apple
add unique (gid) in identities.Google
add index (user) in identities
add interval index (name) in users
define fn display_name (i: Identity) -> Text = match i
  | EmailPass e -> e.email
  | Google g    -> g.gid
  | Apple a     -> a.sub
define fn stamp (t: Text) -> Text = host "stamp" version "1.2"
define aggregate longest (x: Text) -> Text =
  state { best: Text }
  init { best: "" }
  step (s, x) -> if scalar_len(x) > scalar_len(s.best) then { best: x } else s
  merge (a, b) -> if scalar_len(b.best) > scalar_len(a.best) then b else a
  finish s -> s.best
add constraint nameless = from u in users where u.name == "" select u
add check short_gid on identities.Google (scalar_len(gid) <= 64)
"#;

fn migrated(from: &Source, text: &str) -> Source {
    let file = parse_migration(text, Options::default())
        .ok()
        .expect("the migration parses");
    from.migrate(&file, text)
        .unwrap_or_else(|r| panic!("the migration applies: {r:?}"))
}

fn first() -> Source {
    migrated(&Source::default(), FIRST)
}

fn ops(src: &Source, number: u64) -> Vec<ResolvedOp> {
    src.resolved(number)
        .unwrap_or_else(|ds| panic!("migration {number} resolves: {ds:?}"))
}

fn id(src: &Source, path: &str) -> u64 {
    *src.ids
        .get(path)
        .unwrap_or_else(|| panic!("an id for `{path}` among {:?}", src.ids.keys()))
}

#[test]
fn every_operator_resolves_in_ordinal_order() {
    let src = first();
    let kinds: Vec<String> = ops(&src, 1)
        .iter()
        .map(|o| {
            format!("{:?}", o.op)
                .split(' ')
                .next()
                .unwrap_or("")
                .to_string()
        })
        .collect();
    assert_eq!(
        vec![
            "AddModule",
            "AddType",
            "AddRelation",
            "AddType",
            "AddRelation",
            "AddSet",
            "AddUnique",
            "AddIndex",
            "AddIndex",
            "DefineFn",
            "DefineFn",
            "DefineAggregate",
            "AddConstraint",
            "AddCheck",
        ],
        kinds
    );
}

#[test]
fn created_objects_are_named_by_their_ids() {
    let src = first();
    let ops = ops(&src, 1);
    assert_eq!(
        Resolved::AddModule {
            module: "auth".into()
        },
        ops[0].op
    );
    let Resolved::AddType { sum, .. } = &ops[1].op else {
        panic!("{:?}", ops[1].op)
    };
    assert_eq!(id(&src, "type auth::User"), *sum);
    let Resolved::AddRelation { relation, .. } = &ops[4].op else {
        panic!("{:?}", ops[4].op)
    };
    assert_eq!(id(&src, "relation auth::identities"), *relation);
    let Resolved::AddSet {
        set,
        relation: of,
        name,
    } = &ops[5].op
    else {
        panic!("{:?}", ops[5].op)
    };
    assert_eq!(
        (id(&src, "set auth::identities.OAuth"), *relation, "OAuth"),
        (*set, *of, name.as_str())
    );
    let Resolved::DefineFn { function, .. } = &ops[9].op else {
        panic!("{:?}", ops[9].op)
    };
    assert_eq!(id(&src, "fn auth::display_name"), *function);
    let Resolved::DefineAggregate { function, .. } = &ops[11].op else {
        panic!("{:?}", ops[11].op)
    };
    assert_eq!(id(&src, "fn auth::longest"), *function);
    let Resolved::AddConstraint { constraint, .. } = &ops[12].op else {
        panic!("{:?}", ops[12].op)
    };
    assert_eq!(id(&src, "constraint auth::nameless"), *constraint);
    let Resolved::AddCheck { constraint, .. } = &ops[13].op else {
        panic!("{:?}", ops[13].op)
    };
    assert_eq!(id(&src, "constraint auth::short_gid"), *constraint);
}

#[test]
fn an_added_unique_or_index_takes_the_id_of_the_clause_it_appends() {
    let src = first();
    let schema = src.schema().expect("it builds");
    let m = schema.module_named("auth").expect("auth");
    let rel = |n: &str| match schema.module(m).values[n] {
        ValueDef::Relation(r) => r,
        _ => panic!("`{n}` is a relation"),
    };
    let (identities, users) = (rel("identities"), rel("users"));
    let ops = ops(&src, 1);
    assert_eq!(
        Resolved::AddUnique {
            index: schema.ids.indexes[&(identities, 1)],
            relation: schema.ids.relations[&identities],
            fields: "gid".into(),
        },
        ops[6].op
    );
    assert_eq!(
        Resolved::AddIndex {
            index: schema.ids.indexes[&(identities, 2)],
            relation: schema.ids.relations[&identities],
            interval: false,
            fields: "user".into(),
        },
        ops[7].op
    );
    assert_eq!(
        Resolved::AddIndex {
            index: schema.ids.indexes[&(users, 0)],
            relation: schema.ids.relations[&users],
            interval: true,
            fields: "name".into(),
        },
        ops[8].op
    );
}

#[test]
fn a_named_set_and_a_scoped_unique_list_their_constructors_as_member_clauses() {
    let src = first();
    let ops = ops(&src, 1);
    let member = |c: &str| Clause::Member {
        ctor: id(&src, &format!("ctor auth::Identity.{c}")),
    };
    assert_eq!(vec![member("Google"), member("Apple")], ops[5].clauses);
    assert_eq!(vec![member("Google")], ops[6].clauses);
    assert!(ops[7].clauses.is_empty(), "{:?}", ops[7].clauses);
    let others: Vec<&ResolvedOp> = ops
        .iter()
        .enumerate()
        .filter(|(i, _)| ![5, 6].contains(i))
        .map(|(_, o)| o)
        .collect();
    assert!(others.iter().all(|o| o.clauses.is_empty()), "{others:?}");
}

#[test]
fn operands_are_in_canonical_token_form_with_names_qualified() {
    let src = first();
    let ops = ops(&src, 1);
    assert_eq!(
        Resolved::AddType {
            sum: id(&src, "type auth::User"),
            decl: "type auth::User = { id : Key auth::users , name : Text }".into(),
        },
        ops[1].op
    );
    assert_eq!(
        Resolved::AddRelation {
            relation: id(&src, "relation auth::users"),
            decl: "relation auth::users : auth::User key id generate sequence".into(),
        },
        ops[2].op
    );
    assert_eq!(
        Resolved::AddConstraint {
            constraint: id(&src, "constraint auth::nameless"),
            body:
                "constraint auth::nameless = from u in auth::users where u . name == \"\" select u"
                    .into(),
        },
        ops[12].op
    );
}

#[test]
fn the_printed_text_of_a_migration_does_not_change_its_resolved_form() {
    let spaced = FIRST
        .replace(
            "add type User = { id: Key users, name: Text }",
            "add type User = {\n  id:   Key users,  -- the key\n  name: Text\n}",
        )
        .replace(
            "(gid) in identities.Google",
            "( gid )  in  identities.Google",
        );
    assert_ne!(FIRST, spaced);
    assert_eq!(
        ops(&first(), 1),
        ops(&migrated(&Source::default(), &spaced), 1)
    );
}

#[test]
fn a_later_migration_resolves_against_the_source_after_it() {
    let src = migrated(
        &first(),
        "module auth\nadd type Device = { id: Key devices, owner: Key users }\nadd relation devices : Device key id\n",
    );
    let later = ops(&src, 2);
    assert_eq!(
        Resolved::AddType {
            sum: id(&src, "type auth::Device"),
            decl: "type auth::Device = { id : Key auth::devices , owner : Key auth::users }".into(),
        },
        later[0].op
    );
    assert_eq!(ops(&first(), 1), ops(&src, 1));
}

#[test]
fn an_operator_with_no_resolved_form_is_refused_at_its_span() {
    let text = "module auth\nadd default users.name = \"anon\"\n";
    let src = migrated(&first(), text);
    let ds = src
        .resolved(2)
        .expect_err("a default has no resolved form yet");
    assert_eq!(1, ds.len(), "{ds:?}");
    assert_eq!(
        "add default users.name = \"anon\"",
        &text[ds[0].span.range()]
    );
}

#[test]
#[should_panic(expected = "the log holds migration 3")]
fn resolving_a_migration_the_log_lacks_panics() {
    let _unresolved = first().resolved(3);
}

#[test]
fn functions_carry_their_signature_and_code() {
    let src = first();
    let fs = src.functions().expect("the functions resolve");
    let by_name = |n: &str| {
        fs.iter()
            .find(|f| f.name == n)
            .unwrap_or_else(|| panic!("a function `{n}` among {fs:?}"))
    };
    let display = by_name("display_name");
    assert_eq!(
        (
            id(&src, "fn auth::display_name"),
            "auth",
            "(i: Identity) -> Text"
        ),
        (
            display.id,
            display.module.as_str(),
            display.signature.as_str()
        )
    );
    assert_eq!(
        Code::Language {
            body: "match i\n  | EmailPass e -> e.email\n  | Google g    -> g.gid\n  | Apple a     -> a.sub".into(),
            canonical: "match i | auth::EmailPass e -> e . email | auth::Google g -> g . gid | auth::Apple a -> a . sub".into(),
        },
        display.code
    );
    assert_eq!(
        Code::Host {
            symbol: "stamp".into(),
            version: "1.2".into(),
        },
        by_name("stamp").code
    );
    assert!(!by_name("stamp").deterministic);
    assert!(display.deterministic);
    assert!(by_name("longest").deterministic);
    let longest = by_name("longest");
    assert_eq!("(x: Text) -> Text", longest.signature);
    let Code::Aggregate { body, canonical } = &longest.code else {
        panic!("{:?}", longest.code)
    };
    assert!(body.starts_with("state { best: Text }"), "{body}");
    assert!(body.ends_with("finish s -> s.best"), "{body}");
    assert!(
        canonical.starts_with("state { best : Text } init { best : \"\" }"),
        "{canonical}"
    );
    assert_eq!(3, fs.len(), "{fs:?}");
}

#[test]
fn a_function_body_canonical_form_ignores_whitespace_and_comments() {
    let respaced = FIRST.replace(
        "define fn display_name (i: Identity) -> Text = match i\n  | EmailPass e -> e.email",
        "define fn display_name (i: Identity) -> Text =\n  -- by kind\n  match i | EmailPass   e ->  e.email",
    );
    assert_ne!(FIRST, respaced);
    let canonical = |src: &Source| {
        src.functions()
            .expect("the functions resolve")
            .into_iter()
            .find(|f| f.name == "display_name")
            .map(|f| match f.code {
                Code::Language { canonical, .. } => canonical,
                other => panic!("{other:?}"),
            })
    };
    assert_eq!(
        canonical(&first()),
        canonical(&migrated(&Source::default(), &respaced))
    );
}

fn determinism(decls: &[&str]) -> BTreeMap<String, bool> {
    let src = Source::new(
        decls
            .iter()
            .map(|text| SourceDecl {
                module: "app".into(),
                text: (*text).to_string(),
            })
            .collect(),
    )
    .unwrap_or_else(|e| panic!("the schema builds: {e:?}"));
    src.functions()
        .unwrap_or_else(|e| panic!("the functions resolve: {e:?}"))
        .into_iter()
        .map(|f| (f.name, f.deterministic))
        .collect()
}

#[test]
fn a_function_calling_a_nondeterministic_host_function_is_nondeterministic_transitively() {
    let det = determinism(&[
        r#"fn clock (x: I64) -> I64 = host "clock" version "1""#,
        r#"fn pure (x: I64) -> I64 = host "pure" version "1" deterministic"#,
        "fn direct (x: I64) -> I64 = clock(x) + 1",
        "fn indirect (x: I64) -> I64 = direct(x) * 2",
        "fn clean (x: I64) -> I64 = pure(x) + abs(x)",
        "fn passed (x: I64) -> I64 = apply(clock, x)",
        "fn apply (f: I64 -> I64, x: I64) -> I64 = f(x)",
    ]);
    assert_eq!(
        BTreeMap::from([
            ("apply".to_string(), true),
            ("clean".to_string(), true),
            ("clock".to_string(), false),
            ("direct".to_string(), false),
            ("indirect".to_string(), false),
            ("passed".to_string(), false),
            ("pure".to_string(), true),
        ]),
        det
    );
}

#[test]
fn a_cycle_of_functions_is_deterministic_unless_it_reaches_a_nondeterministic_one() {
    let det = determinism(&[
        "fn even (x: I64) -> Bool = if x == 0 then True else odd(x - 1)",
        "fn odd (x: I64) -> Bool = if x == 0 then False else even(x - 1)",
        r#"fn clock (x: I64) -> I64 = host "clock" version "1""#,
        "fn ping (x: I64) -> I64 = if x == 0 then clock(x) else pong(x - 1)",
        "fn pong (x: I64) -> I64 = ping(x)",
    ]);
    assert_eq!(
        BTreeMap::from([
            ("clock".to_string(), false),
            ("even".to_string(), true),
            ("odd".to_string(), true),
            ("ping".to_string(), false),
            ("pong".to_string(), false),
        ]),
        det
    );
}

#[test]
fn an_aggregate_calling_a_nondeterministic_function_is_nondeterministic() {
    let det = determinism(&[
        r#"fn clock (x: I64) -> I64 = host "clock" version "1""#,
        "fn later (x: I64) -> I64 = clock(x)",
        "aggregate stamped (x: I64) -> I64 =
  state { n: I64 }
  init { n: 0 }
  step (s, x) -> { n: s.n + later(x) }
  merge (a, b) -> { n: a.n + b.n }
  finish s -> s.n",
        "aggregate total (x: I64) -> I64 =
  state { n: I64 }
  init { n: 0 }
  step (s, x) -> { n: s.n + x }
  merge (a, b) -> { n: a.n + b.n }
  finish s -> s.n",
    ]);
    assert_eq!(Some(&false), det.get("stamped"), "{det:?}");
    assert_eq!(Some(&true), det.get("total"), "{det:?}");
}

#[test]
fn a_check_body_keeps_its_on_target() {
    let src = first();
    assert_eq!(
        Resolved::AddCheck {
            constraint: id(&src, "constraint auth::short_gid"),
            body: "check auth::short_gid on auth::identities . auth::Google ( scalar_len ( gid ) <= 64 )"
                .into(),
        },
        ops(&src, 1)[13].op
    );
}

#[test]
fn a_fields_term_is_the_field_list_without_its_parentheses() {
    let src = migrated(
        &first(),
        "module auth\nadd unique (user, email) in identities.EmailPass\n",
    );
    let Resolved::AddUnique { fields, .. } = &ops(&src, 2)[0].op else {
        panic!("{:?}", ops(&src, 2))
    };
    assert_eq!("user , email", fields);
}

#[test]
fn an_operand_whose_spelling_recurs_resolves_after_a_forward_reference() {
    let text = "module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id
add type Device = { id: Key devices, name: Text }
add unique (name) in users
add relation devices : Device key id
";
    let src = migrated(&Source::default(), text);
    let ops = ops(&src, 1);
    let Resolved::AddUnique { fields, .. } = &ops[4].op else {
        panic!("{ops:?}")
    };
    assert_eq!("name", fields);
    assert_eq!(
        Resolved::AddType {
            sum: id(&src, "type app::Device"),
            decl: "type app::Device = { id : Key app::devices , name : Text }".into(),
        },
        ops[3].op
    );
}

#[test]
fn an_operand_a_later_operator_edits_around_still_resolves() {
    let text = "module app
add module app
add type Owner = { id: Key owners, name: Text }
add relation owners : Owner key id
add type Device = { id: Key devices, owner: Key owners }
add relation devices : Device key id
add field Device.label: Text fill \"\"
";
    let src = migrated(&Source::default(), text);
    let names = src.log[0]
        .names()
        .unwrap_or_else(|ds| panic!("the names resolve: {ds:?}"));
    let decl = "type Device = { id: Key devices, owner: Key owners }";
    let at = text.find(decl).expect("the declaration");
    assert_eq!(
        "type app::Device = { id : Key app::devices , owner : Key app::owners }",
        canonical_term(text, Span::new(at, at + decl.len()), &names)
    );
}

#[test]
fn renames_and_drops_resolve_to_the_ids_of_what_they_name() {
    let text = "module auth\nrename type User to Person\ndrop index (user) in identities\ndrop fn display_name\n";
    let src = migrated(&first(), text);
    let got: Vec<Resolved> = ops(&src, 2).into_iter().map(|o| o.op).collect();
    assert_eq!(3, got.len(), "{got:?}");
    assert_eq!(
        Resolved::RenameType {
            sum: id(&src, "type auth::Person"),
            name: "Person".into()
        },
        got[0]
    );
    assert!(matches!(got[1], Resolved::DropIndex { .. }), "{got:?}");
    assert!(matches!(got[2], Resolved::DropFn { .. }), "{got:?}");
}
