use super::*;
use crate::ids::{CatConstraintId, CatFunctionId, CatIndexId, CatRelId, CatSetId, CatTypeId};
use crate::parse::parse_schema;

const RUNNING: &str = r"
module auth
type User = { id: Key users, name: Text }
type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text, hash: Bytes }
  | Google    { gid: Text }
relation users : User key id generate sequence
relation identities : Identity
  key id generate uuidv7
  unique (email) in EmailPass
  unique (gid) in Google
set OAuth of identities = Google
fn display_name (i: Identity) -> Text = match i
  | EmailPass e -> e.email
  | Google g    -> g.gid
";

fn build_src(srcs: &[&str]) -> Result<Schema, Vec<String>> {
    let files: Vec<(FileId, ast::SchemaFile)> = srcs
        .iter()
        .enumerate()
        .map(|(i, s)| {
            (
                FileId(i as u32),
                parse_schema(s, Options::default()).ok().unwrap(),
            )
        })
        .collect();
    build(&files).map_err(|ds| ds.into_iter().map(|d| d.diag.message).collect())
}

fn errors(srcs: &[&str]) -> Vec<String> {
    match build_src(srcs) {
        Ok(_) => panic!("built without errors"),
        Err(e) => e,
    }
}

fn assert_error(srcs: &[&str], needle: &str) {
    let errs = errors(srcs);
    assert!(
        errs.iter().any(|e| e.contains(needle)),
        "no error containing {needle:?} in {errs:?}"
    );
}

fn module(body: &str) -> String {
    format!("module m\n{body}")
}

#[test]
fn prelude_builds_alone() {
    let s = build_src(&[]).unwrap();
    let bool_ = s.ty(s.prelude_type("Bool"));
    assert!(bool_.ordered);
    assert_eq!(bool_.ctors().len(), 2);
}

#[test]
fn running_schema_builds() {
    let s = build_src(&[RUNNING]).unwrap();
    let auth = s.module_named("auth").unwrap();
    let ValueDef::Relation(ids) = s.module(auth).values["identities"] else {
        panic!()
    };
    let r = s.relation(ids);
    assert_eq!(r.key, vec!["id"]);
    assert_eq!(r.generate, Some(Generator::Uuidv7));
    assert_eq!(r.sets[0].ctors, vec![1]);
    let def = s.ty(r.elem_def);
    assert!(matches!(
        def.field_named(0, "user").unwrap().ty,
        Ty::Key { .. }
    ));
    assert_eq!(s.show_ty(&r.elem), "Identity");
}

#[test]
fn modules_qualify_names() {
    let billing = "module billing\ntype Account { id: Key accounts } = Owned { who: Key auth::identities } | Shared { team: Text }\nrelation accounts : Account key id generate sequence";
    build_src(&[RUNNING, billing]).unwrap();
    let unqualified = "module billing\ntype Account = { id: Key accounts, who: Key identities }\nrelation accounts : Account key id";
    assert_error(&[RUNNING, unqualified], "no `identities` in scope");
}

#[test]
fn reserved_and_prelude_names_are_refused() {
    assert_error(&[&module("type Set = A | B")], "reserved type name");
    assert_error(&[&module("type Option = A | B")], "prelude type");
    assert_error(&[&module("type Text = A")], "prelude type");
    assert_error(&[&module("fn count () -> I64 = 1")], "prelude function");
}

#[test]
fn a_constructor_or_field_may_take_a_reserved_name() {
    build_src(&[&module("type T = Set { key: I64 } | Num")]).unwrap();
}

#[test]
fn duplicate_names_are_refused() {
    assert_error(&[&module("type A = X\ntype A = Y")], "declared twice");
    assert_error(&[&module("type A = X | X")], "listed twice");
    assert_error(
        &[&module("type A = X { f: I64, f: I64 }")],
        "declared twice",
    );
    assert_error(
        &[&module("type A { f: I64 } = X { f: I64 }")],
        "also a header field",
    );
    assert_error(
        &[&module(
            "type A = { k: I64 }\nrelation a : A key k\nfn a () -> I64 = 1",
        )],
        "declared twice",
    );
}

#[test]
fn another_languages_type_name_suggests_the_prelude_type() {
    assert_error(
        &[&module("type A = { f: Int }")],
        "no type `Int`; did you mean `I64`?",
    );
    assert_error(
        &[&module("type A = { f: String }")],
        "no type `String`; did you mean `Text`?",
    );
}

#[test]
fn fields_have_named_types() {
    assert_error(
        &[&module("type A = { f: { g: I64 } }")],
        "declare a record type",
    );
    assert_error(
        &[&module("type A = { f: X { g: I64 } | Y }")],
        "declare a sum type",
    );
    assert_error(&[&module("type A = { f: Set I64 }")], "never holds a set");
    assert_error(&[&module("type A = { f: I64 -> I64 }")], "function type");
    assert_error(&[&module("type A = { f: Nope }")], "no type `Nope`");
    assert_error(
        &[&module("type A = { f: Option }")],
        "takes 1 type argument",
    );
    assert_error(&[&module("type A = { f: Decimal 39 }")], "at most 38");
    assert_error(&[&module("type A = { f: Option 3 }")], "scale of `Decimal`");
}

#[test]
fn newtypes_are_over_base_types() {
    build_src(&[&module("type Email = new Text where matches(self, \"@\")")]).unwrap();
    assert_error(&[&module("type A = X\ntype B = new A")], "over a base type");
}

#[test]
fn value_types_are_not_recursive() {
    assert_error(&[&module("type A = X { a: A } | Y")], "reaches itself");
    assert_error(
        &[&module("type A = X { b: B } | Y\ntype B = { a: Option A }")],
        "reaches itself",
    );
    build_src(&[&module(
        "type N { id: Key nodes } = Lit { n: I64 } | Add { l: Key nodes, r: Key nodes }\nrelation nodes : N key id generate sequence",
    )])
    .unwrap();
}

#[test]
fn references_are_direct_fields_of_element_types() {
    let base = "type U = { id: Key us }\nrelation us : U key id generate sequence\n";
    build_src(&[&module(&format!(
        "{base}type V = {{ id: Key vs, u: Option (Key us) }}\nrelation vs : V key id"
    ))])
    .unwrap();
    assert_error(
        &[&module(&format!(
            "{base}type V = {{ u: Result (Key us) Text }}"
        ))],
        "direct field",
    );
    assert_error(
        &[&module(&format!(
            "{base}type Ref = {{ u: Key us }}\ntype V = {{ id: Key vs, r: Ref }}\nrelation vs : V key id"
        ))],
        "holds references",
    );
}

#[test]
fn reference_selections_name_constructors_or_sets() {
    let ok = format!(
        "{RUNNING}\ntype R = {{ id: Key rs, g: Key identities.OAuth, e: Key identities.(EmailPass | Google) }}\nrelation rs : R key id generate sequence"
    );
    let s = build_src(&[&ok]).unwrap();
    let auth = s.module_named("auth").unwrap();
    let t = s.ty(s.module(auth).types["R"]);
    assert!(
        matches!(&t.field_named(0, "g").unwrap().ty, Ty::Key { ctors: Some(c), .. } if *c == vec![1])
    );
    assert!(matches!(
        &t.field_named(0, "e").unwrap().ty,
        Ty::Key { ctors: None, .. }
    ));
    let bad = format!(
        "{RUNNING}\ntype R = {{ id: Key rs, g: Key identities.Apple }}\nrelation rs : R key id"
    );
    assert_error(&[&bad], "`Apple` is neither a constructor");
}

#[test]
fn relations_need_one_valid_key() {
    assert_error(
        &[&module("type A = { k: I64 }\nrelation a : A")],
        "exactly one key",
    );
    assert_error(
        &[&module("type A = { k: I64 }\nrelation a : A key j")],
        "key field `j`",
    );
    assert_error(
        &[&module("type A = { k: I64 }\nrelation a : A key k key k")],
        "one key",
    );
    assert_error(
        &[&module("type A = X { k: I64 } | Y\nrelation a : A key k")],
        "only constructor",
    );
    assert_error(
        &[&module(
            "type A = { k: I64 }\nrelation a : A key k generate sequence",
        )],
        "generated key",
    );
    assert_error(
        &[&module(
            "type A = { k: Key a }\nrelation a : A key k generate serial",
        )],
        "not a key generator",
    );
}

#[test]
fn a_self_keyed_type_belongs_to_one_relation() {
    assert_error(
        &[&module(
            "type A = { k: Key a }\nrelation a : A key k generate sequence\nrelation b : A key k",
        )],
        "element type of `a` only",
    );
    build_src(&[&module(
        "type C = { code: Text, name: Text }\nrelation countries : C key code\nrelation retired : C key code",
    )])
    .unwrap();
}

#[test]
fn relation_element_types() {
    assert_error(
        &[&module("relation a : I64 key k")],
        "declared sum or record",
    );
    assert_error(
        &[&module("type B = new I64\nrelation a : B key k")],
        "declared sum or record",
    );
    build_src(&[&module(
        "type P a = { k: Text, v: a }\nrelation ps : P I64 key k",
    )])
    .unwrap();
}

#[test]
fn defaults_name_fields_and_generators() {
    let t = "type L { id: Key ls } = Active | Revoked { at: Instant }\n";
    build_src(&[&module(&format!(
        "{t}relation ls : L key id generate sequence default Revoked.at = generate clock"
    ))])
    .unwrap();
    assert_error(
        &[&module(&format!(
            "{t}relation ls : L key id default Revoked.nope = 1"
        ))],
        "no field `nope`",
    );
    assert_error(
        &[&module(&format!(
            "{t}relation ls : L key id default id = generate clock"
        ))],
        "fills an `Instant` field",
    );
    assert_error(
        &[&module(&format!(
            "{t}relation ls : L key id default Revoked.at = generate uuidv7"
        ))],
        "generates only `clock`",
    );
}

#[test]
fn named_sets_list_constructors_consistently() {
    assert_error(
        &[&format!(
            "{RUNNING}\nset Bad of identities = Google | Apple"
        )],
        "`Apple` is not a constructor",
    );
    assert_error(
        &[&format!(
            "{RUNNING}\nrelation old_ids : Identity key id\nset OAuth of old_ids = EmailPass"
        )],
        "other constructors",
    );
}

#[test]
fn ordered_types_have_ordered_fields() {
    build_src(&[&module("type S ordered = Draft | Sent { at: Instant }")]).unwrap();
    assert_error(
        &[&module("type S ordered = Sent { p: Period }")],
        "unordered type",
    );
    assert_error(
        &[&module("type U = A | B\ntype S ordered = X { u: U }")],
        "unordered type",
    );
    assert_error(&[&module("type B = { s: Span Period }")], "ordered `a`");
    build_src(&[&module("type B = { s: Span Instant }")]).unwrap();
}

#[test]
fn ordered_nested_instantiations_are_finite() {
    let schema = build_src(&[&module(
        "type Holder ordered = { value: Option (Option I8) }",
    )])
    .expect("nested finite ordered instantiations build");
    let m = schema.module_named("m").unwrap();
    assert!(schema.is_ordered(&Ty::Data {
        def: schema.module(m).types["Holder"],
        args: Vec::new(),
        ctors: None,
    }));
}

#[test]
fn ordered_shared_graph_does_not_expand_every_path() {
    let mut source = "module m\ntype T0 ordered = { leaf: I8 }\n".to_owned();
    for level in 1..=30 {
        let previous = level - 1;
        source.push_str(&format!(
            "type T{level} ordered = {{ left: T{previous}, right: T{previous} }}\n"
        ));
    }
    let parsed = parse_schema(&source, Options::default()).ok().unwrap();
    let schema = build(&[(FileId(0), parsed)]).expect("the ordered type graph builds");
    crate::check::check_schema(&schema).expect("the ordered graph typechecks");
    let m = schema.module_named("m").unwrap();
    assert!(schema.is_ordered(&Ty::Data {
        def: schema.module(m).types["T30"],
        args: Vec::new(),
        ctors: None,
    }));
}

#[test]
fn ordered_requirements_use_the_instantiation_and_current_parameter_context() {
    let schema = build_src(&[&module(
        "type Last a b ordered = { value: b }\ntype Phantom a = new I64",
    )])
    .unwrap();
    let m = schema.module_named("m").unwrap();
    let last = schema.module(m).types["Last"];
    let at_zero = Ty::Data {
        def: last,
        args: vec![Ty::Base(Base::Period), Ty::Param(0)],
        ctors: None,
    };
    assert!(schema.is_ordered_with(&at_zero, &|index| index == 0));
    assert!(!schema.is_ordered_with(&at_zero, &|_| false));
    assert!(schema.is_ordered_with(&at_zero, &|_| true));
    let at_one = Ty::Data {
        def: last,
        args: vec![Ty::Param(0), Ty::Param(1)],
        ctors: None,
    };
    assert!(schema.is_ordered_with(&at_one, &|index| index == 1));
    assert!(!schema.is_ordered_with(&at_one, &|index| index == 0));
    assert!(!schema.is_ordered(&Ty::Data {
        def: last,
        args: vec![Ty::Base(Base::I8), Ty::Base(Base::Period)],
        ctors: None,
    }));
    assert!(schema.is_ordered(&Ty::Data {
        def: schema.module(m).types["Phantom"],
        args: vec![Ty::Base(Base::Period)],
        ctors: None,
    }));
    assert_error(&[&module("type Invalid a = new a")], "over a base type");
}

#[test]
fn ordered_recursive_declarations_are_refused_without_unfolding_arguments() {
    for source in [
        "type Loop ordered = { next: Loop }",
        "type Loop a ordered = { next: Loop (Option a) }",
        "type A a ordered = { next: B (Option a) }\ntype B b ordered = { next: A b }",
    ] {
        assert_error(&[&module(source)], "reaches itself");
    }
}

#[test]
fn ordered_fields_do_not_reuse_another_instantiation_or_schema_state() {
    assert_error(
        &[&module(
            "type Invalid ordered = { good: Option I8, bad: Option Period }",
        )],
        "unordered type",
    );
    let stage = |ordered: &str| {
        let schema = build_src(&[&module(&format!("type Stage{ordered} = A | B"))]).unwrap();
        let m = schema.module_named("m").unwrap();
        let ty = Ty::Data {
            def: schema.module(m).types["Stage"],
            args: Vec::new(),
            ctors: None,
        };
        let tag = Ty::Tag(Box::new(ty.clone()));
        (schema.is_ordered(&ty), schema.is_ordered(&tag))
    };
    assert_eq!((true, true), stage(" ordered"));
    assert_eq!((false, false), stage(""));
    let schema = build_src(&[&module("type Stage ordered = A | B")]).unwrap();
    assert!(!schema.is_ordered(&Ty::Record(vec![])));
    assert!(!schema.is_ordered(&Ty::Unit));
}

#[test]
fn ordered_composite_key_graphs_share_requirements() {
    let mut source = "module m\ntype K0 ordered = { id: I8 }\nrelation k0 : K0 key id\n".to_owned();
    for level in 1..=30 {
        let previous = level - 1;
        source.push_str(&format!(
            "type K{level} ordered = {{ left: Key k{previous}, right: Key k{previous} }}\nrelation k{level} : K{level} key (left, right)\n"
        ));
    }
    let schema = build_src(&[&source]).expect("the shared key graph builds");
    crate::check::check_schema(&schema).expect("the shared key graph typechecks");
    assert!(schema.is_ordered(&Ty::Key {
        rel: relation(&schema, "k30"),
        ctors: None,
    }));
}

#[test]
fn ordered_keys_require_a_finite_ordered_representation() {
    let schema = build_src(&[&module(
        "type Root = { id: Key roots }\nrelation roots : Root key id\ntype Calendar = { id: Period }\nrelation calendars : Calendar key id\ntype Alpha = { target: Key betas }\nrelation alphas : Alpha key target\ntype Beta = { target: Key alphas }\nrelation betas : Beta key target",
    )])
    .unwrap();
    for (name, ordered) in [("roots", true), ("calendars", false), ("alphas", false)] {
        assert_eq!(
            ordered,
            schema.is_ordered(&Ty::Key {
                rel: relation(&schema, name),
                ctors: None,
            }),
            "{name}"
        );
    }
}

#[test]
fn on_delete_actions() {
    let base = "type U = { id: Key us }\nrelation us : U key id generate sequence\n";
    build_src(&[&module(&format!(
        "{base}type L {{ id: Key ls, u: Key us on delete cascade }} = Active {{ o: Option (Key us) on delete clear }} | Gone\nrelation ls : L key id generate sequence"
    ))])
    .unwrap();
    assert_error(
        &[&module(&format!(
            "{base}type L = {{ id: Key ls, u: Key us on delete clear }}\nrelation ls : L key id"
        ))],
        "`Option (Key ...)` field",
    );
    assert_error(
        &[&module("type L = { n: I64 on delete cascade }")],
        "only on a `Key` field",
    );
}

#[test]
fn the_schema_may_declare_principal_once() {
    let s = build_src(&[&format!(
        "{RUNNING}\ntype Principal = EndUser {{ user: Text }} | Service {{ name: Text }} | Anonymous"
    )])
    .unwrap();
    assert_ne!(s.principal, s.prelude_type("Principal"));
    assert_error(
        &[
            &module("type Principal = EndUser { u: Text } | Service { name: Text } | Anonymous"),
            "module n\ntype Principal = EndUser { u: Text } | Service { name: Text } | Anonymous",
        ],
        "declares `Principal` once",
    );
}

#[test]
fn a_principal_without_the_required_constructors_is_refused() {
    let shapes = [
        "type Principal = Admin | Guest",
        "type Principal = EndUser { u: Text } | Service { name: I64 } | Anonymous",
        "type Principal = EndUser { u: Text } | Service { name: Text } | Anonymous { why: Text }",
        "type Principal = EndUser { u: Text } | Service { name: Text }",
        "type Principal = EndUser { u: Text } | Service { name: Text } | Anonymous | Robot",
        "type Principal { at: Text } = EndUser { u: Text } | Service { name: Text } | Anonymous",
        "type Principal = Service { name: Text, team: Text } | EndUser | Anonymous",
    ];
    for shape in shapes {
        assert_error(
            &[&module(shape)],
            "`Principal` has exactly the constructors",
        );
    }
}

#[test]
fn a_principal_in_any_constructor_order_checks() {
    build_src(&[&module(
        "type Principal = Anonymous | Service { name: Text } | EndUser { id: I64, org: Text }",
    )])
    .unwrap();
}

#[test]
fn engine_only_relation_forms_are_refused() {
    assert_error(
        &[&module(
            "type A = { k: I64 }\nrelation sys::a : A key k volatile",
        )],
        "engine's own declarations",
    );
}

#[test]
fn engine_only_relation_forms_name_both_refusals() {
    let errs = errors(&[&module(
        "type A = { k: I64 }\nrelation sys::a : A key k volatile",
    )]);
    assert!(
        errs.iter()
            .any(|e| e == "a relation name is qualified only in the engine's own declarations"),
        "{errs:?}"
    );
    assert!(
        errs.iter()
            .any(|e| e == "`volatile` appears only in the engine's own declarations"),
        "{errs:?}"
    );
}

#[test]
fn the_engine_declares_the_catalog_and_sys_modules() {
    let s = build_src(&[]).unwrap();
    for (module, relation, volatile) in [
        ("catalog", "types", false),
        ("catalog", "constructors", false),
        ("catalog", "ctor_states", false),
        ("catalog", "migration_ops", false),
        ("catalog", "bundles", false),
        ("catalog", "plans", false),
        ("catalog", "settings", false),
        ("catalog", "audit", false),
        ("catalog", "grants", false),
        ("sys", "ctor_stats", true),
        ("sys", "commit_times", true),
        ("sys", "sessions", true),
        ("sys", "calls", true),
        ("sys", "pending_drops", true),
        ("sys", "verify_findings", true),
        ("sys", "format", true),
    ] {
        let m = s.module_named(module).expect("an engine module");
        assert!(s.module(m).engine);
        let Some(&ValueDef::Relation(r)) = s.module(m).values.get(relation) else {
            panic!("no relation `{module}::{relation}`");
        };
        assert_eq!(volatile, s.relation(r).volatile, "{module}::{relation}");
        assert_eq!(m, s.relation(r).module, "{module}::{relation}");
    }
}

#[test]
fn every_sys_relation_is_volatile_and_no_catalog_relation_is() {
    let s = build_src(&[]).unwrap();
    for r in &s.relations {
        let module = &s.module(r.module).name;
        assert_eq!(module == "sys", r.volatile, "{module}::{}", r.name);
    }
}

#[test]
fn engine_modules_are_not_user_modules() {
    let s = build_src(&[RUNNING]).unwrap();
    let vis = crate::resolve::Visible::all(&s);
    let names: Vec<&str> = vis
        .modules
        .iter()
        .map(|&m| s.module(m).name.as_str())
        .collect();
    assert_eq!(vec!["auth"], names);
}

#[test]
fn no_user_module_takes_an_engine_module_name() {
    for name in ["prelude", "catalog", "sys"] {
        assert_error(
            &[&format!("module {name}\ntype A = {{ k: I64 }}")],
            &format!("no module may be named `{name}`"),
        );
    }
}

#[test]
fn catalog_relations_are_not_in_scope_unqualified() {
    assert_error(
        &[&module(
            "type Note = { id: Key notes, about: Key constructors }\nrelation notes : Note key id generate sequence",
        )],
        "no `constructors` in scope",
    );
}

#[test]
fn function_signatures() {
    let s = build_src(&[&module(
        "fn latest [a: Ord] (xs: Set a) -> Option a = max(x for x in xs)\nfn having (p: I64 -> Bool) -> Set I64 = []",
    )])
    .unwrap();
    let f = &s.functions[0];
    assert_eq!(f.type_params, vec![("a".to_string(), vec![Bound::Ord])]);
    assert_eq!(s.show_ty_with(&f.result, &["a".to_string()]), "Option a");
    assert_error(&[&module("fn f [a: Show] (x: a) -> a = x")], "not a bound");
    assert_error(
        &[&module("fn f (x: b) -> I64 = 1")],
        "no type parameter `b`",
    );
}

#[test]
fn stores_name_parameterless_functions() {
    build_src(&[&module("fn v () -> Set I64 = []\nstore v\n  key k")]).unwrap();
    assert_error(
        &[&module("fn v (x: I64) -> Set I64 = []\nstore v")],
        "no parameters",
    );
}

fn type_id(s: &Schema, name: &str) -> TypeId {
    TypeId(s.types.iter().position(|t| t.name == name).expect("a type") as u32)
}

fn rel_id(s: &Schema, module: &str, name: &str) -> RelId {
    let m = s.module_named(module).expect("a module");
    match s.module(m).values.get(name) {
        Some(&ValueDef::Relation(r)) => r,
        _ => panic!("no relation `{module}::{name}`"),
    }
}

#[test]
fn declarations_take_ids_in_declaration_order_from_1024() {
    let s = build_src(&[RUNNING]).unwrap();
    let user = type_id(&s, "User");
    let identity = type_id(&s, "Identity");
    let users = rel_id(&s, "auth", "users");
    let identities = rel_id(&s, "auth", "identities");
    assert_eq!(Some(1024), s.ids.type_id(user).map(u64::from));
    assert_eq!(
        Some(1025),
        s.ids.ctor_id(ctor_ref(&s, user, 0)).map(u64::from)
    );
    assert_eq!(Some(1026), s.ids.type_id(identity).map(u64::from));
    assert_eq!(
        Some(1027),
        s.ids.ctor_id(ctor_ref(&s, identity, 0)).map(u64::from)
    );
    assert_eq!(
        Some(1028),
        s.ids.ctor_id(ctor_ref(&s, identity, 1)).map(u64::from)
    );
    assert_eq!(Some(1029), s.ids.relation_id(users).map(u64::from));
    assert_eq!(Some(1030), s.ids.relation_id(identities).map(u64::from));
    let field = |t, c, f: &str| {
        field_ref(&s, t, c, f)
            .and_then(|f| s.ids.field_id(f))
            .map(u64::from)
    };
    assert_eq!(Some(1024), field(user, Some(0), "id"));
    assert_eq!(Some(1025), field(user, Some(0), "name"));
    assert_eq!(Some(1026), field(identity, None, "id"));
    assert_eq!(Some(1027), field(identity, None, "user"));
    assert_eq!(Some(1028), field(identity, Some(0), "email"));
    assert_eq!(Some(1030), field(identity, Some(1), "gid"));
    assert_eq!(Some(1024), s.ids.key_id(users).map(u64::from));
    assert_eq!(Some(1025), s.ids.key_id(identities).map(u64::from));
    assert_eq!(Some(1026), s.ids.index_id(identities, 0).map(u64::from));
    assert_eq!(Some(1027), s.ids.index_id(identities, 1).map(u64::from));
    assert_eq!(Some(1024), s.ids.generated_id(users).map(u64::from));
    assert_eq!(Some(1025), s.ids.generated_id(identities).map(u64::from));
    assert_eq!(Some(1024), s.ids.set_id(identities, "OAuth").map(u64::from));
    assert_eq!(Some(1024), s.ids.function_id(FnId(0)).map(u64::from));
}

#[test]
fn a_constructor_ref_is_minted_only_for_a_constructor_the_type_has() {
    let s = build_src(&[RUNNING, "module mail\ntype Email = new Text"]).unwrap();
    let identity = s.ty(type_id(&s, "Identity"));
    let google = identity
        .ctor_named("Google")
        .expect("Identity declares Google");
    assert_eq!((type_id(&s, "Identity"), 1), (google.def(), google.index()));
    assert_eq!(Some(google), identity.ctor_at(1));
    assert_eq!(None, identity.ctor_named("Missing"));
    assert_eq!(None, identity.ctor_at(2));
    assert_eq!(
        vec![identity.ctor_named("EmailPass"), Some(google)],
        identity.ctor_refs().map(Some).collect::<Vec<_>>()
    );
    let email = s.ty(type_id(&s, "Email"));
    assert_eq!(None, email.ctor_named("Email"));
    let only = email.ctor_at(0).expect("a newtype's one constructor");
    assert_eq!((type_id(&s, "Email"), 0), (only.def(), only.index()));
    assert_eq!(None, email.ctor_at(1));
    assert_eq!(vec![only], email.ctor_refs().collect::<Vec<_>>());
}

#[test]
fn a_field_ref_places_a_header_field_in_the_header_whichever_constructor_names_it() {
    let s = build_src(&[RUNNING]).unwrap();
    let identity = s.ty(type_id(&s, "Identity"));
    let (email_pass, google) = (
        identity.ctor_named("EmailPass"),
        identity.ctor_named("Google"),
    );
    let place = |c, name: &str| identity.field(c, name).map(|f| (f.part(), f.pos()));
    assert_eq!(Some((FieldPart::Header, 1)), place(None, "user"));
    assert_eq!(Some((FieldPart::Header, 1)), place(google, "user"));
    assert_eq!(Some((FieldPart::Ctor(0), 1)), place(email_pass, "hash"));
    assert_eq!(Some((FieldPart::Ctor(1), 0)), place(google, "gid"));
    assert_eq!(None, place(google, "hash"));
    assert_eq!(None, place(None, "hash"));
    assert_eq!(None, place(email_pass, "missing"));
    let hash = identity.field(email_pass, "hash").unwrap();
    assert_eq!("hash", identity.field_def(hash).name);
    let names: Vec<_> = identity
        .field_refs()
        .map(|(f, d)| (f.ctor(), d.name.as_str()))
        .collect();
    let expected = [
        (None, "id"),
        (None, "user"),
        (Some(0), "email"),
        (Some(0), "hash"),
        (Some(1), "gid"),
    ];
    assert_eq!(expected.to_vec(), names);
}

#[test]
fn the_engine_constructors_name_their_declarations_and_the_schemas_own_principal() {
    let named = |s: &Schema, c: CtorRef| {
        let t = s.ty(c.def());
        (
            s.module(t.module).name.clone(),
            t.name.clone(),
            t.ctor_def(c).name.clone(),
        )
    };
    let own = |m: &str, t: &str, c: &str| (m.to_string(), t.to_string(), c.to_string());
    let s = build_src(&[]).unwrap();
    let e = s.engine_ctors();
    assert_eq!(own("prelude", "Bool", "False"), named(&s, e.bool_false()));
    assert_eq!(own("prelude", "Bool", "True"), named(&s, e.bool_true()));
    assert_eq!(
        own("prelude", "Principal", "Service"),
        named(&s, e.service())
    );
    assert_eq!(own("catalog", "Role", "Role"), named(&s, e.role()));
    assert_eq!(
        own("catalog", "RoleGrant", "RoleGrant"),
        named(&s, e.role_grant())
    );
    assert_eq!(
        own("catalog", "GrantKind", "Deployment"),
        named(&s, e.deployment())
    );
    assert_eq!(own("sys", "Finding", "Violation"), named(&s, e.violation()));
    let s = build_src(&[&module(
        "type Principal = Service { name: Text } | EndUser { u: Text } | Anonymous",
    )])
    .unwrap();
    let service = s.engine_ctors().service();
    assert_eq!(own("m", "Principal", "Service"), named(&s, service));
    assert_eq!(0, service.index());
}

/// Assert that the reverse lookup `of` finds exactly its object for each pair of `pairs`.
///
/// Returns how many pairs it checked.
fn finds_each<K: PartialEq + std::fmt::Debug, I: Copy + std::fmt::Display>(
    kind: &str,
    pairs: impl Iterator<Item = (K, I)>,
    of: impl Fn(I) -> Option<K>,
) -> usize {
    let mut seen = 0;
    for (k, id) in pairs {
        assert_eq!(Some(k), of(id), "{kind} {id}");
        seen += 1;
    }
    seen
}

#[test]
fn every_id_of_a_built_schema_finds_its_object() {
    let src = format!(
        "{RUNNING}aggregate longest (x: Text) -> Text =
  state {{ best: Text }}
  init {{ best: \"\" }}
  step (s, x) -> if scalar_len(x) > scalar_len(s.best) then {{ best: x }} else s
  merge (a, b) -> if scalar_len(b.best) > scalar_len(a.best) then b else a
  finish s -> s.best
constraint nameless = from u in users where u.name == \"\" select u
"
    );
    let s = build_src(&[&src]).unwrap();
    let ids = &s.ids;
    let seen = finds_each("type", ids.type_ids(), |id| ids.type_of(id))
        + finds_each("constructor", ids.ctor_ids(), |id| ids.ctor_of(id))
        + finds_each("relation", ids.relation_ids(), |id| ids.relation_of(id))
        + finds_each("field", ids.field_ids(), |id| ids.field_of(id))
        + finds_each(
            "set",
            ids.set_ids().map(|(r, name, id)| ((r, name), id)),
            |id| ids.set_of(id),
        )
        + finds_each("index", ids.index_ids(), |id| ids.index_of(id))
        + finds_each("function", ids.function_ids(), |id| ids.function_of(id))
        + finds_each("aggregate", ids.aggregate_ids(), |id| ids.aggregate_of(id))
        + finds_each("constraint", ids.constraint_ids(), |id| {
            ids.constraint_of(id)
        });
    let user = type_id(&s, "User");
    let users = rel_id(&s, "auth", "users");
    let identities = rel_id(&s, "auth", "identities");
    assert_eq!(Some(user), ids.type_of(CatTypeId::from_raw(1024)));
    assert_eq!(Some(users), ids.relation_of(CatRelId::from_raw(1029)));
    let oauth = ids.set_of(CatSetId::from_raw(1024));
    assert_eq!(Some((identities, "OAuth")), oauth);
    let unique = ids.index_of(CatIndexId::from_raw(1026));
    assert_eq!(Some((identities, 0)), unique);
    assert_eq!(
        Some(FnId(0)),
        ids.function_of(CatFunctionId::from_raw(1024))
    );
    assert_eq!(Some(0), ids.aggregate_of(CatFunctionId::from_raw(1025)));
    assert_eq!(Some(0), ids.constraint_of(CatConstraintId::from_raw(1024)));
    assert!(seen > 1000, "{seen} ids");
}

#[test]
fn engine_types_constructors_and_relations_take_their_fixed_ids() {
    let s = build_src(&[]).unwrap();
    let bool_ = s.prelude_type("Bool");
    assert_eq!(Some(0), s.ids.type_id(bool_).map(u64::from));
    assert_eq!(
        Some(2),
        s.ids.ctor_id(ctor_ref(&s, bool_, 1)).map(u64::from)
    );
    let fault = s.prelude_type("RejectReason");
    let mig = s.ty(fault).ctor_index("MigrationInProgress").unwrap();
    assert_eq!(
        Some(372),
        s.ids.ctor_id(ctor_ref(&s, fault, mig)).map(u64::from)
    );
    assert_eq!(
        Some(219),
        s.ids
            .relation_id(rel_id(&s, "catalog", "types"))
            .map(u64::from)
    );
    assert_eq!(
        Some(340),
        s.ids
            .relation_id(rel_id(&s, "sys", "format"))
            .map(u64::from)
    );
    assert_eq!(
        Some(241),
        s.ids
            .relation_id(rel_id(&s, "catalog", "audit"))
            .map(u64::from)
    );
    for (i, t) in s.types.iter().enumerate() {
        if s.module(t.module).engine {
            assert!(
                s.ids.type_id(TypeId(i as u32)).is_some(),
                "no id for `{}`",
                t.name
            );
            for c in t.ctor_refs() {
                assert!(
                    s.ids.ctor_id(c).is_some(),
                    "no id for a constructor of `{}`",
                    t.name
                );
            }
        }
    }
    for (i, r) in s.relations.iter().enumerate() {
        assert!(
            s.ids.relation_id(RelId(i as u32)).is_some(),
            "no id for `{}`",
            r.name
        );
    }
}

#[test]
fn every_constructor_is_enabled_in_a_schema_built_from_declarations() {
    let s = build_src(&[RUNNING]).unwrap();
    let identities = rel_id(&s, "auth", "identities");
    assert_eq!(Stage::Enabled, s.stage(identities, 1));
    assert_eq!(Stage::Enabled, s.value_stage(type_id(&s, "Identity"), 1));
}

#[test]
fn row_too_large_takes_the_field_ids_after_internal() {
    let s = build_src(&[]).unwrap();
    let field = |t, c, f: &str| {
        field_ref(&s, t, c, f)
            .and_then(|f| s.ids.field_id(f))
            .map(u64::from)
    };
    let reject = s.prelude_type("RejectReason");
    let too_large = s.ty(reject).ctor_index("RowTooLarge").unwrap();
    assert_eq!(Some(70), field(reject, Some(too_large), "relation"));
    assert_eq!(Some(73), field(reject, Some(too_large), "limit"));
}

#[test]
fn key_too_large_appends_fields_without_renumbering_engine_fields() {
    let s = build_src(&[]).unwrap();
    let reject = s.prelude_type("RejectReason");
    let ctor = s.ty(reject).ctor_index("KeyTooLarge").unwrap();
    assert_eq!(
        Some(471),
        s.ids.ctor_id(ctor_ref(&s, reject, ctor)).map(u64::from)
    );
    for (offset, name) in ["relation", "ctor", "index", "size", "limit"]
        .iter()
        .enumerate()
    {
        assert_eq!(
            Some(564 + offset as u64),
            field_ref(&s, reject, Some(ctor), name)
                .and_then(|f| s.ids.field_id(f))
                .map(u64::from)
        );
    }
    let limit = s.prelude_type("Limit");
    let rounds = s.ty(limit).ctor_index("Rounds").unwrap();
    assert_eq!(
        Some(563),
        field_ref(&s, limit, Some(rounds), "rounds")
            .and_then(|f| s.ids.field_id(f))
            .map(u64::from)
    );
}

#[test]
fn engine_fields_and_keys_take_their_fixed_ids() {
    let s = build_src(&[]).unwrap();
    let field = |t, c, f: &str| {
        field_ref(&s, t, c, f)
            .and_then(|f| s.ids.field_id(f))
            .map(u64::from)
    };
    let option = s.prelude_type("Option");
    assert_eq!(Some(0), field(option, Some(1), "value"));
    let result = s.prelude_type("Result");
    assert_eq!(Some(1), field(result, Some(0), "value"));
    assert_eq!(Some(2), field(result, Some(1), "error"));
    assert_eq!(
        Some(0),
        s.ids.key_id(rel_id(&s, "catalog", "types")).map(u64::from)
    );
    assert_eq!(
        Some(1),
        s.ids
            .key_id(rel_id(&s, "catalog", "constructors"))
            .map(u64::from)
    );
    for (i, t) in s.types.iter().enumerate() {
        if !s.module(t.module).engine {
            continue;
        }
        let def = TypeId(i as u32);
        for f in t.header() {
            assert!(field(def, None, &f.name).is_some_and(|id| id < 1024));
        }
        for (k, c) in t.ctors().iter().enumerate() {
            for f in &c.fields {
                assert!(
                    field(def, Some(k as u32), &f.name).is_some_and(|id| id < 1024),
                    "no fixed id for `{}.{}.{}`",
                    t.name,
                    c.name,
                    f.name
                );
            }
        }
    }
    let credentials = rel_id(&s, "catalog", "credentials");
    let key = s.ids.key_id(credentials).map(u64::from).unwrap();
    assert_eq!(Some(key + 1), s.ids.index_id(credentials, 0).map(u64::from));
    assert_eq!(
        Some(key + 2),
        s.ids.key_id(rel_id(&s, "sys", "collations")).map(u64::from)
    );
    for (i, r) in s.relations.iter().enumerate() {
        let rel = RelId(i as u32);
        assert!(s.ids.key_id(rel).unwrap().raw() < 1024);
        for k in 0..r.indexes.len() {
            assert!(s.ids.index_id(rel, k).unwrap().raw() < 1024);
        }
    }
    let reject = s.prelude_type("RejectReason");
    let damaged = s.ty(reject).ctor_index("DumpDamaged").unwrap();
    assert_eq!(Some(68), field(reject, Some(damaged), "part"));
    let internal = s.ty(reject).ctor_index("Internal").unwrap();
    assert_eq!(Some(69), field(reject, Some(internal), "what"));
    let limit = s.prelude_type("Limit");
    let rounds = s.ty(limit).ctor_index("Rounds").unwrap();
    assert_eq!(Some(563), field(limit, Some(rounds), "rounds"));
    assert_eq!(Some(574), s.ids.field_ids().map(|(_, id)| id.raw()).max());
    assert_eq!(
        Some(60),
        s.ids
            .index_ids()
            .map(|(_, id)| id.raw())
            .chain(s.ids.key_ids().map(|(_, id)| id.raw()))
            .max()
    );
}

#[test]
fn the_engine_fields_of_catalog_and_sys_start_after_the_prelude() {
    let s = build_src(&[]).unwrap();
    let catalog_min = s
        .ids
        .field_ids()
        .filter(|(f, _)| s.module(s.ty(f.def()).module).name != "prelude")
        .map(|(_, id)| id.raw())
        .min();
    assert_eq!(Some(92), catalog_min);
}

fn relation(s: &Schema, name: &str) -> RelId {
    let m = s.module_named("m").unwrap();
    match s.module(m).values[name] {
        ValueDef::Relation(r) => r,
        other => panic!("`{name}` is a relation, found {other:?}"),
    }
}

#[test]
fn reference_fields_name_their_holders_targets_and_leading_indexes() {
    let s = build_src(&[&module(
        "type U = { id: Key us }
relation us : U key id generate sequence
type N { id: Key ns, owner: Key us }
  = A { parent: Option (Key ns), by: Key us }
  | B { tag: Text, other: Key us }
relation ns : N key id generate sequence
  index (by) in A
type M = { u: Key us, n: I64 }
relation ms : M key u",
    )])
    .unwrap();
    let (us, ns, ms) = (relation(&s, "us"), relation(&s, "ns"), relation(&s, "ms"));
    let got: Vec<_> = s
        .reference_fields(ns)
        .into_iter()
        .map(|f| {
            (
                f.ctor,
                f.field,
                f.at,
                f.ctors,
                f.target,
                f.target_ctors,
                f.optional,
                f.index,
            )
        })
        .collect();
    assert_eq!(
        vec![
            (
                None,
                "owner".to_string(),
                1,
                vec![0, 1],
                us,
                None,
                false,
                ReferenceIndex::Implied
            ),
            (
                Some(0),
                "parent".to_string(),
                2,
                vec![0],
                ns,
                None,
                true,
                ReferenceIndex::Implied
            ),
            (
                Some(0),
                "by".to_string(),
                3,
                vec![0],
                us,
                None,
                false,
                ReferenceIndex::Declared(0)
            ),
            (
                Some(1),
                "other".to_string(),
                3,
                vec![1],
                us,
                None,
                false,
                ReferenceIndex::Implied
            ),
        ],
        got
    );
    assert_eq!(
        vec!["owner", "parent", "other"],
        s.implied_indexes(ns)
            .iter()
            .map(|i| i.field.as_str())
            .collect::<Vec<_>>()
    );
    let keyed = s.reference_fields(ms);
    assert_eq!(1, keyed.len());
    assert_eq!(
        ("u", ReferenceIndex::Key),
        (keyed[0].field.as_str(), keyed[0].index)
    );
    assert!(s.reference_fields(us).is_empty());
}

#[test]
fn a_privilege_clause_takes_its_fixed_ids() {
    let s = build_src(&[]).unwrap();
    let catalog = s.module_named("catalog").unwrap();
    let clause = s.module(catalog).types["ClauseBody"];
    let privilege = s.ty(clause).ctor_index("Privilege").unwrap();
    assert_eq!(
        Some(472),
        s.ids
            .ctor_id(ctor_ref(&s, clause, privilege))
            .map(u64::from)
    );
    for (name, id) in [("privilege", 569), ("target", 570)] {
        assert_eq!(
            Some(id),
            field_ref(&s, clause, Some(privilege), name)
                .and_then(|f| s.ids.field_id(f))
                .map(u64::from)
        );
    }
}

#[test]
fn a_key_reaches_the_types_of_its_relations_key_fields_at_their_arguments() {
    let s = build_src(&[&module(
        "type Tier = Free | Paid\ntype Mark = Ink\ntype P a = { k: a, n: Text, m: Mark }\nrelation ps : P Tier key (k, n)",
    )])
    .unwrap();
    let key = Ty::Key {
        rel: rel_id(&s, "m", "ps"),
        ctors: None,
    };
    assert!(s.reaches(&key, type_id(&s, "Tier")));
    assert!(!s.reaches(&key, type_id(&s, "Mark")));
}

fn ctor_ref(s: &Schema, def: TypeId, c: u32) -> CtorRef {
    s.ty(def).ctor_at(c).expect("a constructor")
}

/// Returns field `name` of constructor `c` of `def` of `s`, of the header for `None`.
fn field_ref(s: &Schema, def: TypeId, c: Option<u32>, name: &str) -> Option<FieldRef> {
    let td = s.ty(def);
    td.field(c.and_then(|c| td.ctor_at(c)), name)
        .filter(|f| f.ctor() == c)
}

#[test]
fn the_field_ids_of_a_type_run_header_first_then_each_constructor_by_position() {
    let s = build_src(&[&module(
        "type T { z: I64 } = A { y: I64, b: I64 } | B { a: I64 }\nrelation ts : T key z",
    )])
    .unwrap();
    let def = type_id(&s, "T");
    let td = s.ty(def);
    let names: Vec<&str> = s
        .ids
        .type_field_ids(def)
        .map(|(f, _)| td.field_def(f).name.as_str())
        .collect();
    assert_eq!(vec!["z", "y", "b", "a"], names);
}
