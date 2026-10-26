use super::*;
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
    assert!(matches!(def.field(0, "user").unwrap().ty, Ty::Key { .. }));
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
        matches!(&t.field(0, "g").unwrap().ty, Ty::Key { ctors: Some(c), .. } if *c == vec![1])
    );
    assert!(matches!(
        &t.field(0, "e").unwrap().ty,
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
    assert_eq!(Some(&1024), s.ids.types.get(&user));
    assert_eq!(Some(&1025), s.ids.ctors.get(&(user, 0)));
    assert_eq!(Some(&1026), s.ids.types.get(&identity));
    assert_eq!(Some(&1027), s.ids.ctors.get(&(identity, 0)));
    assert_eq!(Some(&1028), s.ids.ctors.get(&(identity, 1)));
    assert_eq!(Some(&1029), s.ids.relations.get(&users));
    assert_eq!(Some(&1030), s.ids.relations.get(&identities));
    let field = |t, c, f: &str| s.ids.fields.get(&(t, c, f.to_string())).copied();
    assert_eq!(Some(1024), field(user, Some(0), "id"));
    assert_eq!(Some(1025), field(user, Some(0), "name"));
    assert_eq!(Some(1026), field(identity, None, "id"));
    assert_eq!(Some(1027), field(identity, None, "user"));
    assert_eq!(Some(1028), field(identity, Some(0), "email"));
    assert_eq!(Some(1030), field(identity, Some(1), "gid"));
    assert_eq!(Some(&1024), s.ids.keys.get(&users));
    assert_eq!(Some(&1025), s.ids.keys.get(&identities));
    assert_eq!(Some(&1026), s.ids.indexes.get(&(identities, 0)));
    assert_eq!(Some(&1027), s.ids.indexes.get(&(identities, 1)));
    assert_eq!(Some(&1024), s.ids.generated.get(&users));
    assert_eq!(Some(&1025), s.ids.generated.get(&identities));
    assert_eq!(
        Some(&1024),
        s.ids.sets.get(&(identities, "OAuth".to_string()))
    );
    assert_eq!(Some(&1024), s.ids.functions.get(&FnId(0)));
}

#[test]
fn engine_types_constructors_and_relations_take_their_fixed_ids() {
    let s = build_src(&[]).unwrap();
    let bool_ = s.prelude_type("Bool");
    assert_eq!(Some(&0), s.ids.types.get(&bool_));
    assert_eq!(Some(&2), s.ids.ctors.get(&(bool_, 1)));
    let fault = s.prelude_type("RejectReason");
    let mig = s.ty(fault).ctor_index("MigrationInProgress").unwrap();
    assert_eq!(Some(&372), s.ids.ctors.get(&(fault, mig)));
    assert_eq!(
        Some(&219),
        s.ids.relations.get(&rel_id(&s, "catalog", "types"))
    );
    assert_eq!(
        Some(&340),
        s.ids.relations.get(&rel_id(&s, "sys", "format"))
    );
    assert_eq!(
        Some(&241),
        s.ids.relations.get(&rel_id(&s, "catalog", "audit"))
    );
    for (i, t) in s.types.iter().enumerate() {
        if s.module(t.module).engine {
            assert!(
                s.ids.types.contains_key(&TypeId(i as u32)),
                "no id for `{}`",
                t.name
            );
            for c in 0..t.ctors().len() as u32 {
                assert!(
                    s.ids.ctors.contains_key(&(TypeId(i as u32), c)),
                    "no id for a constructor of `{}`",
                    t.name
                );
            }
        }
    }
    for (i, r) in s.relations.iter().enumerate() {
        assert!(
            s.ids.relations.contains_key(&RelId(i as u32)),
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
fn engine_fields_and_keys_take_their_fixed_ids() {
    let s = build_src(&[]).unwrap();
    let field = |t, c, f: &str| s.ids.fields.get(&(t, c, f.to_string())).copied();
    let option = s.prelude_type("Option");
    assert_eq!(Some(0), field(option, Some(1), "value"));
    let result = s.prelude_type("Result");
    assert_eq!(Some(1), field(result, Some(0), "value"));
    assert_eq!(Some(2), field(result, Some(1), "error"));
    assert_eq!(Some(&0), s.ids.keys.get(&rel_id(&s, "catalog", "types")));
    assert_eq!(
        Some(&1),
        s.ids.keys.get(&rel_id(&s, "catalog", "constructors"))
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
    let key = s.ids.keys[&credentials];
    assert_eq!(Some(&(key + 1)), s.ids.indexes.get(&(credentials, 0)));
    assert_eq!(
        Some(&(key + 2)),
        s.ids.keys.get(&rel_id(&s, "sys", "collations"))
    );
    for (i, r) in s.relations.iter().enumerate() {
        let rel = RelId(i as u32);
        assert!(s.ids.keys[&rel] < 1024);
        for k in 0..r.indexes.len() {
            assert!(s.ids.indexes[&(rel, k)] < 1024);
        }
    }
    let reject = s.prelude_type("RejectReason");
    let damaged = s.ty(reject).ctor_index("DumpDamaged").unwrap();
    assert_eq!(Some(68), field(reject, Some(damaged), "offset"));
    let internal = s.ty(reject).ctor_index("Internal").unwrap();
    assert_eq!(Some(69), field(reject, Some(internal), "what"));
    let limit = s.prelude_type("Limit");
    let rounds = s.ty(limit).ctor_index("Rounds").unwrap();
    assert_eq!(Some(556), field(limit, Some(rounds), "rounds"));
    assert_eq!(Some(556), s.ids.fields.values().max().copied());
    assert_eq!(
        Some(60),
        s.ids
            .indexes
            .values()
            .chain(s.ids.keys.values())
            .max()
            .copied()
    );
}

#[test]
fn the_engine_fields_of_catalog_and_sys_start_after_the_prelude() {
    let s = build_src(&[]).unwrap();
    let catalog_min = s
        .ids
        .fields
        .iter()
        .filter(|((t, _, _), _)| s.module(s.ty(*t).module).name != "prelude")
        .map(|(_, id)| *id)
        .min();
    assert_eq!(Some(88), catalog_min);
}
