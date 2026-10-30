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
fn an_added_default_names_its_relation_field_and_value() {
    let src = migrated(&first(), "module auth\nadd default users.name = \"anon\"\n");
    assert_eq!(
        Resolved::AddDefault {
            default: id(&src, "default auth::users User.name"),
            relation: id(&src, "relation auth::users"),
            field: id(&src, "field auth::User.User.name"),
            value: "\"anon\"".into(),
        },
        ops(&src, 2)[0].op
    );
}

#[test]
fn a_default_over_several_constructors_names_its_first_default() {
    let text = "module auth\nadd field Identity.Google.verified: Bool fill False\nadd field Identity.Apple.verified: Bool fill False\nadd default identities.verified = True\n";
    let src = migrated(&first(), text);
    assert_eq!(
        Resolved::AddDefault {
            default: id(&src, "default auth::identities Google.verified"),
            relation: id(&src, "relation auth::identities"),
            field: id(&src, "field auth::Identity.Google.verified"),
            value: "True".into(),
        },
        ops(&src, 2)[2].op
    );
    assert_eq!(
        vec![
            Clause::Member {
                ctor: id(&src, "ctor auth::Identity.Google"),
            },
            Clause::Member {
                ctor: id(&src, "ctor auth::Identity.Apple"),
            },
        ],
        ops(&src, 2)[2].clauses
    );
    let src = migrated(&src, "module auth\ndrop default identities.verified\n");
    assert_eq!(
        vec![
            Clause::Member {
                ctor: id(&src, "ctor auth::Identity.Google"),
            },
            Clause::Member {
                ctor: id(&src, "ctor auth::Identity.Apple"),
            },
        ],
        ops(&src, 3)[0].clauses
    );
}

#[test]
fn a_header_default_names_no_constructor() {
    let text = "module auth\nadd default identities.user = users.key(1)\n";
    assert_eq!(
        Vec::<Clause>::new(),
        ops(&migrated(&first(), text), 2)[0].clauses
    );
}

#[test]
fn a_generated_default_keeps_its_generate() {
    let text = "module auth\nadd type Stamp = { id: Key stamps, at: Instant }\nadd relation stamps : Stamp key id\nadd default stamps.at = generate clock\n";
    let src = migrated(&first(), text);
    let Resolved::AddDefault { value, .. } = &ops(&src, 2)[2].op else {
        panic!("an added default")
    };
    assert_eq!("generate clock", value);
}

#[test]
fn a_dropped_default_names_the_default_it_drops() {
    let src = migrated(&first(), "module auth\nadd default users.name = \"anon\"\n");
    let added = id(&src, "default auth::users User.name");
    let src = migrated(&src, "module auth\ndrop default users.name\n");
    assert_eq!(Resolved::DropDefault { default: added }, ops(&src, 3)[0].op);
}

#[test]
fn computed_fields_resolve_to_their_ids_and_declarations() {
    let text = "module auth\nadd computed users.shout: Text = self.name ++ \"!\"\n";
    let src = migrated(&first(), text);
    let computed = id(&src, "computed auth::users.shout");
    assert_eq!(
        Resolved::AddComputed {
            computed,
            relation: id(&src, "relation auth::users"),
            name: "shout".into(),
            decl: "shout : Text = self . name ++ \"!\"".into(),
        },
        ops(&src, 2)[0].op
    );
    let text = "module auth\nredefine computed users.shout: Text = self.name\n";
    let src = migrated(&src, text);
    assert_eq!(
        Resolved::RedefineComputed {
            computed,
            decl: "shout : Text = self . name".into(),
        },
        ops(&src, 3)[0].op
    );
    let src = migrated(&src, "module auth\ndrop computed users.shout\n");
    assert_eq!(Resolved::DropComputed { computed }, ops(&src, 4)[0].op);
}

#[test]
fn a_stored_view_resolves_to_its_relation_and_function() {
    let text = "module auth\ndefine fn named () -> Set { id: Key users, name: Text } = from u in users select { id: u.id, name: u.name }\nstore named key id\n";
    let src = migrated(&first(), text);
    let view = id(&src, "relation auth::named");
    let got = ops(&src, 2);
    assert_eq!(
        Resolved::Store {
            view,
            function: id(&src, "fn auth::named"),
            decl: "store auth::named key id".into(),
        },
        got[1].op
    );
    let src = migrated(&src, "module auth\nunstore named\n");
    assert_eq!(Resolved::Unstore { view }, ops(&src, 3)[0].op);
}

#[test]
fn grants_and_revokes_hold_their_privileges_in_canonical_form() {
    let text = "module auth\ngrant read identities.(EmailPass | Google) (user), call fn display_name\n  to support with grant option\nrevoke call fn display_name from support\n";
    let src = migrated(&first(), text);
    let got = ops(&src, 2);
    assert_eq!(
        Resolved::Grant {
            privilege: "read".into(),
            target: "auth::identities . ( auth::EmailPass | auth::Google ) ( user )".into(),
            grantee: "support".into(),
            grant_option: true,
        },
        got[0].op
    );
    assert_eq!(
        vec![Clause::Privilege {
            privilege: "call".into(),
            target: "auth::display_name".into(),
        }],
        got[0].clauses
    );
    assert_eq!(
        Resolved::Revoke {
            privilege: "call".into(),
            target: "auth::display_name".into(),
            grantee: "support".into(),
        },
        got[1].op
    );
}

#[test]
fn an_install_holds_its_package_and_an_uninstall_its_module() {
    let mut src = first();
    src.packages.insert(
        "geo-1.2.adtx".into(),
        "type Cell = { lat: F64, lon: F64 }".into(),
    );
    let src = migrated(
        &src,
        "module auth\ninstall geo \"1.2\" from \"geo-1.2.adtx\"\n",
    );
    assert_eq!(
        Resolved::Install {
            module: "geo".into(),
            version: "1.2".into(),
            package: "type Cell = { lat: F64, lon: F64 }".into(),
        },
        ops(&src, 2)[0].op
    );
    let src = migrated(&src, "module auth\nuninstall geo\n");
    assert_eq!(
        Resolved::Uninstall {
            module: "geo".into()
        },
        ops(&src, 3)[0].op
    );
}

#[test]
fn an_adopted_release_holds_its_data() {
    let mut src = first();
    src.packages.insert("tz-2026b.tzif".into(), "zones".into());
    let src = migrated(
        &src,
        "module auth\nadopt tzdb \"2026b\" from \"tz-2026b.tzif\"\n",
    );
    assert_eq!(
        Resolved::AdoptTzdb {
            release: "2026b".into(),
            data: "zones".into(),
        },
        ops(&src, 2)[0].op
    );
}

#[test]
fn an_adopted_release_whose_data_is_missing_is_refused_at_its_span() {
    let text = "module auth\nadopt tzdb \"2026b\" from \"tz-2026b.tzif\"\n";
    let src = migrated(&first(), text);
    let ds = src.resolved(2).expect_err("the release data is missing");
    assert_eq!(1, ds.len(), "{ds:?}");
    assert_eq!(
        "the release data `tz-2026b.tzif` is not available",
        ds[0].message
    );
    assert_eq!(
        "adopt tzdb \"2026b\" from \"tz-2026b.tzif\"",
        &text[ds[0].span.range()]
    );
}

#[test]
fn a_script_holds_its_write_in_canonical_form() {
    let text = "module auth\nscript\n  update u in users where u.name == \"\" set { u with name: \"anon\" }\n";
    let src = migrated(&first(), text);
    assert_eq!(
        Resolved::Script {
            write: "update u in auth::users where u . name == \"\" set { u with name : \"anon\" }"
                .into(),
        },
        ops(&src, 2)[0].op
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

#[test]
fn dropping_a_relation_then_its_type_resolves() {
    let text = "module auth\nadd type Label = { id: Key labels, name: Text }\nadd relation labels : Label key id\n";
    let src = migrated(&first(), text);
    let src = migrated(&src, "module auth\ndrop relation labels\ndrop type Label\n");
    let got: Vec<Resolved> = ops(&src, 3).into_iter().map(|o| o.op).collect();
    assert_eq!(2, got.len(), "{got:?}");
}

#[test]
fn an_arm_clause_resolves_to_its_target_and_canonical_arm() {
    let text = "module auth\nadd constructor Identity.Passkey { cred: Bytes }\n  arm display_name: Passkey p -> \"passkey\"\n";
    let src = migrated(&first(), text);
    let got = ops(&src, 2);
    assert_eq!(
        vec![Clause::Arm {
            target: ArmTarget::OfFn {
                function: id(&src, "fn auth::display_name")
            },
            arm: "auth::Passkey p -> \"passkey\"".into()
        }],
        got[0].clauses
    );
}

#[test]
fn a_merge_lists_the_constructors_it_replaces_and_its_arms() {
    let text = r"module auth
add type Provider = Google | Apple
merge constructor Identity.(Google | Apple) into Social { provider: Provider, sub: Text }
  by match
    | Google g -> Social { provider: Google, sub: g.gid }
    | Apple a  -> Social { provider: Apple, sub: a.sub }
  arm display_name: Social s -> s.sub
";
    let before = migrated(
        &first(),
        "module auth\ndrop unique (gid) in identities.Google\ndrop check short_gid\n",
    );
    let google = id(&before, "ctor auth::Identity.Google");
    let apple = id(&before, "ctor auth::Identity.Apple");
    let src = migrated(&before, text);
    let got = ops(&src, 3);
    assert_eq!(3, got[1].clauses.len(), "{:?}", got[1].clauses);
    assert_eq!(Clause::Member { ctor: google }, got[1].clauses[0]);
    assert_eq!(Clause::Member { ctor: apple }, got[1].clauses[1]);
    assert!(
        matches!(&got[1].clauses[2], Clause::Arm { arm, .. } if arm == "auth::Social s -> s . sub"),
        "{:?}",
        got[1].clauses
    );
}

#[test]
fn a_rekey_lists_its_carried_fields_and_a_revert_its_fills() {
    let src = migrated(
        &first(),
        "module auth\nadd field Identity.EmailPass.note: Text fill \"\"\n",
    );
    let note = id(&src, "field auth::Identity.EmailPass.note");
    let src = migrated(&src, "module auth\ndrop field Identity.EmailPass.note\n");
    let src = migrated(
        &src,
        "module auth\nrevert 0003\n  fill Identity.EmailPass.note = \"x\"\n",
    );
    let got = ops(&src, 4);
    let [Clause::Fill { field, fill }] = got[0].clauses.as_slice() else {
        panic!("one fill clause: {:?}", got[0].clauses);
    };
    assert_ne!(note, *field, "a restored field is a new object");
    assert_eq!("\"x\"", fill);
    let text =
        "module auth\nrekey relation users key name\n  carry Identity.user\ndrop field User.id\n";
    let user = id(&src, "field auth::Identity.user");
    let src = migrated(&src, text);
    assert_eq!(vec![Clause::Carry { field: user }], ops(&src, 5)[0].clauses);
}

#[test]
fn a_field_added_then_renamed_resolves_to_one_id() {
    let src = migrated(
        &first(),
        "module auth\nadd field User.nick: Text fill \"\"\nrename field User.nick to handle\n",
    );
    let got: Vec<Resolved> = ops(&src, 2).into_iter().map(|o| o.op).collect();
    let field = id(&src, "field auth::User.User.handle");
    match got.as_slice() {
        [
            Resolved::AddField { field: a, .. },
            Resolved::RenameField { field: b, name },
        ] => {
            assert_eq!(field, *a);
            assert_eq!(field, *b);
            assert_eq!("handle", name);
        }
        other => panic!("an add field and a rename: {other:?}"),
    }
}

#[test]
fn alter_field_resolves_to_the_field_and_its_action() {
    let src = migrated(
        &first(),
        "module auth\nalter field Identity.user on delete cascade\n",
    );
    let field = id(&src, "field auth::Identity.user");
    match ops(&src, 2)
        .into_iter()
        .map(|o| o.op)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [Resolved::AlterField { field: f, action }] => {
            assert_eq!(field, *f);
            assert_eq!("cascade", action);
        }
        other => panic!("an alter field: {other:?}"),
    }
}

#[test]
fn an_added_field_resolves_with_its_delete_action() {
    let src = migrated(
        &first(),
        "module auth\nadd field User.mentor: Option (Key users) on delete clear fill None\n",
    );
    match ops(&src, 2)
        .into_iter()
        .map(|o| o.op)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [Resolved::AddField { decl, .. }] => {
            assert_eq!("mentor : Option ( Key auth::users ) on delete clear", decl);
        }
        other => panic!("an add field: {other:?}"),
    }
}

#[test]
fn a_principal_declared_by_the_first_migration_resolves_to_its_type() {
    let src = migrated(
        &Source::default(),
        "module auth\nadd module auth\nadd type User = { id: Key users, name: Text }\n\
         add relation users : User key id generate sequence\n\
         add type Principal = EndUser { user: Key users } | Service { name: Text } | Anonymous\n",
    );
    let schema = src.schema().expect("the schema builds");
    let principal = schema.ids.types[&schema.principal];
    assert!(
        ops(&src, 1)
            .iter()
            .any(|op| matches!(op.op, Resolved::AddType { sum, .. } if sum == principal)),
        "the operator names the schema's principal"
    );
}
