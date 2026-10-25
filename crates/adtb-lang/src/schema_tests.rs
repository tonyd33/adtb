use super::*;
use crate::parse::parse_schema;

const RUNNING: &str = r#"
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
"#;

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
