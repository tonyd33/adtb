//! The version hash of every migration of a corpus that logs each operator, each clause and each
//! constructor of their nested operands.

use std::collections::BTreeSet;

use adtb_lang::evolve::Source;
use adtb_lang::schema::{Schema, ValueDef};
use adtb_lang::types::Ty;

use crate::heap::FieldType;
use crate::key::Value;

use super::{layout, oplog, version_hash};

const CAMPAIGN: [&str; 5] = [
    include_str!("../../../../apps/campaign/schema/migrations/0001.adt"),
    include_str!("../../../../apps/campaign/schema/migrations/0002.adt"),
    include_str!("../../../../apps/campaign/schema/migrations/0003.adt"),
    include_str!("../../../../apps/campaign/schema/migrations/0004.adt"),
    include_str!("../../../../apps/campaign/schema/migrations/0005.adt"),
];

const BASE: &str = "module law
add module law
add type U = { id: Key us, label: Text }
add relation us : U key id generate sequence
add type P = A | B
add type Q = Q1 | Q2
add type K = A | B
add type Inner = A { x: Text, z: Text, q: Q } | B { x: Text, z: Text, q: Q }
add type T { id: Key ts, n: Text, h: Key us }
  = A { x: Text, z: Text, q: Q }
  | B { x: Text, z: Text, q: Q }
  | C { z: Text, y: Text }
add relation ts : T key id generate sequence
add set S of ts = A | B
add unique (x) in ts.A
add default ts.A.x = \"a\"
add type Badge = { id: I64, name: Text }
add relation badges : Badge key id
add type Holder = { id: Key holders, badge: Key badges }
add relation holders : Holder key id generate sequence
define fn f (t: T) -> Text = match t
  | A a -> a.x
  | B b -> b.x
  | C c -> c.z
define fn spare () -> I64 = 1
";

/// One migration of module `law` after `BASE` for each operator, in log order.
const OPERATORS: &[&str] = &[
    "module m2\nadd module m2",
    "module m2\ndrop module m2",
    "module law\ninstall geo \"1.2\" from \"geo-1.2.adtx\"",
    "module law\nuninstall geo",
    "module law\nadd type W = W1 | W2\nrename type W to W3\ndrop type W3",
    "module law
add type V = { id: Key vs, w: Text }
add relation vs : V key id generate sequence
rename relation vs to vs2
drop relation vs2
drop type V",
    "module law\nadd set R of ts = A | B | C\nrename set R of ts to R2\ndrop set R2 of ts",
    "module law
add constructor P.F
add constructor P.G before A
add constructor P.H after B
rename constructor P.H to H2
enable P.H2
deprecate P.H2",
    "module law\nremove constructor P.H2",
    "module law\ndeprecate ts.C\nenable ts.C\nenable P.G\ndisable P.G\nenable P.F\ndeprecate P.F",
    "module law
add constraint blankish =
  from t in ts
  where match t
    | A a -> scalar_len(a.x) == 0
    | B b -> scalar_len(b.x) == 0
    | C c -> scalar_len(c.z) == 0
  select t
add computed ts.label: Text = match self
  | A a -> a.x
  | B b -> b.x
  | C c -> c.z
add constructor T.E { w: Text }
  arm f: E e -> e.w
  arm blankish: E e -> scalar_len(e.w) == 0
  arm ts.label: E e -> e.w
drop constraint blankish
drop computed ts.label",
    "module law
drop default ts.A.x
split constructor T.A into A1 { x: Text, z: Text } | A2 { x: Text, z: Text }
  by match
    | A { q: Q1, x, z } -> A1 { x: x, z: z }
    | A { q: Q2, x, z } -> A2 { x: x, z: z }
  unique (z) in A1
  index (z) in A2
  interval index (x) in A2
  arm f: A1 a -> a.x
  arm f: A2 a -> a.x",
    "module law
add type K2 = A1 | A2
merge constructor T.(A1 | A2) into A { k: K2, x: Text, z: Text }
  arm f: A a -> a.x",
    "module law
drop unique (k, x) in ts.A
drop unique (k, z) in ts.A
merge constructor T.(A | B) into M { k: K, x: Text, z: Text }
  by match
    | A a -> M { k: A, x: a.x, z: a.z }
    | B b -> M { k: B, x: b.x, z: b.z }
  arm f: M m -> m.x",
    "module law
add type Inner2 = C { z: Text, y: Text } | E { w: Text }
nest constructor T.(C | E) into N { inner: Inner2 }
  arm f: N n -> match n.inner | C c -> c.z | E e -> e.w",
    "module law\nunnest constructor T.N\n  arm f: C c -> c.z\n  arm f: E e -> e.w",
    "module law
add field T.C.w: Text fill \"w\"
add field T.m: Text fill \"m\"
alter field T.h on delete cascade
rename field T.C.w to w2
drop field T.C.w2",
    "module law\nmove field T.C.y to T fill match\n  | M m -> \"\"\n  | E e -> \"\"",
    "module law\nmove field T.y to T.C",
    "module law
add unique (z) in ts.C
drop unique (z) in ts.C
add index (n) in ts.(C | E)
add interval index (label) in us
drop index (n) in ts.(C | E)
rekey relation badges key name\n  carry Holder.badge",
    "module law
add default ts.C.z = \"c\"
drop default ts.C.z
define fn g (x: Text) -> Text = x
redefine fn g (x: Text) -> Text = x ++ \"!\"
drop fn g",
    "module law
define aggregate shortest (x: Text) -> Text =
  state { best: Text }
  init { best: \"\" }
  step (s, x) -> if scalar_len(x) < scalar_len(s.best) then { best: x } else s
  merge (a, b) -> if scalar_len(b.best) < scalar_len(a.best) then b else a
  finish s -> s.best
drop aggregate shortest",
    "module law
add constraint blank = from t in ts where t.n == \"\" select t
drop constraint blank
add check named on ts (scalar_len(n) > 0)
drop check named",
    "module law
define fn labels () -> Set ({ label: Text }) = from u in us select { label: u.label }
store labels
  key label",
    "module law\nunstore labels",
    "module law
add computed ts.width: I64 = scalar_len(self.n)
redefine computed ts.width: I64 = 2 * scalar_len(self.n)
drop computed ts.width",
    "module law
grant read us, call fn f to support
grant update ts.(M | E) (M.x, n) to support with grant option
revoke read us from support",
    "module law\nadopt tzdb \"2026b\" from \"tz-2026b.tzif\"",
    "module law\nscript\n  insert us { label: \"one\" }",
    "module law\ndrop field T.m",
    "module law\nrevert 0036\n  fill T.m = \"m\"",
];

/// The hex of the version hash after each migration of `CAMPAIGN`, then of `BASE` and
/// `OPERATORS`.
const HASHES: &[&str] = &[
    "fc0a74642b9f6711651848cdee8609dee7e5d09de4dca2f3c0f4b9e671a13b29",
    "eb05048d4b8c37e3ca32c34bff18c6535b91c4212e0806c5c4927cb0dec56e3a",
    "ce01c10b89dbbc26cf49b36dff134b84d52208fdf83e3a9f4ad359cc7fb20696",
    "55331b63f9c889135c04e7d2570d67e27d89050ac449f462cf659e85af6f79d3",
    "f5b8d6b627b62f78b82fa22f5cc73b2c69f430e7d20c7fb829dfe0e45dafb503",
    "dbbad85d4a60b0f7d84a222c30770210c9dc1b4fe77097a135b107fb52ae4cef",
    "c1ccda4dedf136713173783c6ea1a9c4b2d92f52ecbec33a819cb8a2712c0fe3",
    "0b251bd8cac91acfdaa9b5f071a1df8a5b1cbefd7789952e980eb19014150790",
    "7db936bef1a02d1965bb4cd6bdc0326b5a487943822430d6ac4734166c1981d3",
    "a120cd35c08a6035826714dc73b90141af28644dd437ed411046d3f055e554eb",
    "5eb0734e9e5017e0cb8ee1e8fd951ff29d1170e241b4407d97515bb3215b062c",
    "3240368a1ebe63a4b409e954c327897e41b881983712b51597c5922194830d52",
    "b62e359834ceb15ccce8f0ee5db52a01b1996d91ed2fb4015cc80f5697b75e0e",
    "38618ecb9f131abe710b51ac6ee128372dbd31a144f9272eb71d6f5dbd4f7380",
    "62dda785a4de364cb361ea0536f1b09e0356457dba2e3c6fc265dae7963a96d7",
    "68f6d58a2b80886dc554fd1a9aeaab714a15c074548c9a0e9a3e6c72d68e4d74",
    "8183a5f4f67e6db96c09d89134e289cfd6676f0f23e370f801e67357139647f9",
    "014996022d01915ce585ad952070f53cdff6c659c6df2104fa52619bf80cbc81",
    "8f2813d9beab237a21cea2b1f8e4ad4ede8f25aeb08ae8b4d2502e2b4d45e2e6",
    "4edb3a7796dc435d2b5a9b6fb9cda3f036b07576117edf50d351c8adf785ae17",
    "c079edb9cec983480654fcff87718a5552ef3fd298318ff092517e6a9d456fa4",
    "d2c6c46f5141cbbdfc757f7930936cbb4ca4ce638daa8aeea2ed364c3eeb84be",
    "e5920a6799e3a7eafc10699055b34a658575f50adb9dec1ef9cd45070b976bb6",
    "45b7d9f96a230163139ff16f861f99ac97b598e0e20fab20f79a0619e2cf88db",
    "9692ad2fbeeac56f8f388e8b72038318868b82a6dfda2a3cba540d661e88a585",
    "ae188d8b01efed5e2a7f4789efd8bc131f070fbac2666a35724bda8b962299be",
    "c05962d1a0c835a9a1703abeda448ce25faca0433966c15ffdd3eada0bb13e7f",
    "0c8b933a5217ab7b783067618532cfef0a61eacba78bb2ddf679df457aafd0fe",
    "56cea2cee373c224bbc7d91f766c5b390566ab6c5caa8f205a19b15bf49a46df",
    "a7190ab50554225fd1da574d9c1c11b5de9f27ab1bc417ffd581cf1ab9ea9cb1",
    "0421d3880790c96ed819d46304e1ce4a87f52f49d114c42dd6f19e24e9e30f96",
    "2c515741c1d07d382a1f6b7f478e056c9bd74a450ab867db3df344aab1e765d3",
    "689cc44b9aeee5ef6629d558ae87d3dd2228f3a6e1e64b8a8f602aa358b4366d",
    "eabedc5b06846ab9b2c8fcf3ae8f5f086765eee93b837c5ae8a3378983fe7d1b",
    "479a90e50ba23127d034a42162b51ed68743985d0be14deb0f864dd4e6a695f4",
    "23142a66a8e7b01400bec2cf6c8dd36a8845adf54cf9d6b4529992cb4229b92c",
    "b054526b7798d88d639688f18c39b24142b1b5ff3ada655714ca6bc82025db45",
];

fn migrated(from: &Source, text: &str) -> Source {
    let file = adtb_lang::parse::parse_migration(text, adtb_lang::parse::Options::default())
        .ok()
        .unwrap_or_else(|_| panic!("the migration parses: {text}"));
    from.migrate(&file, text).unwrap_or_else(|r| {
        panic!(
            "the migration applies: {text}\n{:?}\n{:?}",
            r.operators, r.recheck
        )
    })
}

fn hex(h: [u8; 32]) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

/// Returns the type of field `field` of the rows of `catalog::{relation}`.
fn field_ty(schema: &Schema, relation: &str, field: &str) -> Ty {
    let m = schema.module_named("catalog").expect("the catalog module");
    let Some(&ValueDef::Relation(r)) = schema.module(m).values.get(relation) else {
        panic!("the catalog declares relation `{relation}`")
    };
    layout::row(schema, r, 0)
        .into_iter()
        .find(|(n, _)| n == field)
        .unwrap_or_else(|| panic!("a row of `{relation}` has field `{field}`"))
        .1
}

/// Add the type and constructor names of each sum value `v` of type `ty` holds, itself included.
fn reach(schema: &Schema, ty: &Ty, v: &Value, out: &mut BTreeSet<(String, String)>) {
    let Value::Sum { ctor, fields } = v else {
        return;
    };
    let Ty::Data { def, .. } = ty else {
        panic!("{ty:?} holding {v:?} is a declared sum")
    };
    let declared = schema.ty(*def);
    out.insert((
        declared.name.clone(),
        declared.ctors()[*ctor as usize].name.clone(),
    ));
    let types = layout::ctors(schema, ty).swap_remove(*ctor as usize).1;
    for ((_, t), v) in types.iter().zip(fields) {
        reach(schema, t, v, out);
    }
}

/// Add the type and constructor names of every constructor of `ty`, when it is a sum, and of
/// every sum a field of one of them holds, at any depth.
fn every_ctor(schema: &Schema, ty: &Ty, out: &mut BTreeSet<(String, String)>) {
    let (FieldType::Sum(_), Ty::Data { def, .. }) = (layout::field_type(schema, ty), ty) else {
        return;
    };
    let declared = schema.ty(*def);
    for (c, (_, fields)) in declared.ctors().iter().zip(layout::ctors(schema, ty)) {
        if out.insert((declared.name.clone(), c.name.clone())) {
            for (_, t) in &fields {
                every_ctor(schema, t, out);
            }
        }
    }
}

#[test]
fn every_version_of_a_log_holding_each_operator_keeps_its_stored_hash() {
    let mut src = Source::default();
    src.packages.insert(
        "geo-1.2.adtx".into(),
        "type Cell = { lat: F64, lon: F64 }\nfn scale (x: F64) -> F64 = x * 2.0".into(),
    );
    src.packages.insert("tz-2026b.tzif".into(), "zones".into());
    for text in CAMPAIGN.iter().chain([&BASE]).chain(OPERATORS) {
        src = migrated(&src, text);
    }
    let schema = src.schema().expect("it builds");
    let (op_ty, clause_ty) = (
        field_ty(&schema, "migration_ops", "op"),
        field_ty(&schema, "migration_clauses", "body"),
    );
    let log = oplog::Log::new(&schema);
    let (mut h, mut got, mut reached) = ([0; 32], Vec::new(), BTreeSet::new());
    for l in &src.log {
        let ops = oplog::operators(&log, &src, l.number);
        for (op, clauses) in &ops {
            reach(&schema, &op_ty, op, &mut reached);
            for c in clauses {
                reach(&schema, &clause_ty, c, &mut reached);
            }
        }
        h = log.chained(h, &ops);
        got.push(hex(h));
    }
    assert_eq!(hex(version_hash(&schema, &src)), got[got.len() - 1]);
    let mut want = BTreeSet::new();
    every_ctor(&schema, &op_ty, &mut want);
    every_ctor(&schema, &clause_ty, &mut want);
    for never in ["PrimaryKey", "Implied"] {
        assert!(want.remove(&("IndexKind".to_owned(), never.to_owned())));
    }
    let missed: Vec<_> = want.difference(&reached).collect();
    assert!(missed.is_empty(), "the corpus logs no {missed:?}");
    assert_eq!(HASHES, got);
}
