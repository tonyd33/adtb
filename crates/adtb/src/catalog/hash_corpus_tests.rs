//! The version hash of every migration of a corpus that logs each operator, each clause and each
//! constructor of their nested operands.

use std::collections::BTreeSet;

use adtb_lang::evolve::Source;
use adtb_lang::schema::{Schema, ValueDef};
use adtb_lang::types::Ty;

use crate::heap::FieldType;
use crate::key::Value;

use super::{layout, oplog};

const SAMPLE_LOG: [&str; 5] = [
    include_str!("hash_corpus/0001.adt"),
    include_str!("hash_corpus/0002.adt"),
    include_str!("hash_corpus/0003.adt"),
    include_str!("hash_corpus/0004.adt"),
    include_str!("hash_corpus/0005.adt"),
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
drop unique (z) in ts.A1
drop index (z) in ts.A2
drop interval index (x) in ts.A2
merge constructor T.(A1 | A2) into A { k: K2, x: Text, z: Text }
  arm f: A a -> a.x",
    "module law
drop unique (x) in ts.A
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
    "module law\nadd set Named of us = U\nadd unique (label) in us.Named",
];

/// The hex of the version hash after each migration of `SAMPLE_LOG`, then of `BASE` and
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
    "aba4459c5e4f8071a9d276f5f4f56700ee79438250680a83feb71c74a5cd05f8",
    "f7770335b3e8935aacb7353d3432c7681aa1c6746a21422b45a7320d563d0932",
    "3f78a01a2c2c1604b846f8548351b7431ba5c5e689cf68fda165f27c98575c2d",
    "7f2d0e4a98f8178787a303140078b171c2f7dab66afebe73deac4d729fde0f39",
    "66d3a34384c1273ca3849e0270e91da005e9ccecb61ee55f0519b32b386ff84e",
    "33495f6947a9a917fefc1fef720cbfe9faf7a1acf5133031aee639ab6e82fb01",
    "8c4ab67305c8fdebeabf014e26b7ae8874dcf58cac67eaa9a17eb6c45ff3fecd",
    "bdc466207ff4fd71511890ac3588aa1b209f2e098d34b43c23076ea8aa25803f",
    "f54bfc2841b66d5d7f92d4d09f9468154899e4e2ed90c8eec7ef52d48aa1aa29",
    "5f9465789f5ee47bb10f10085b8f946364940f33bb352636f7779548dd75e42c",
    "6ec64d7b0c6a0a37c38d522362c8734f4b76cc12f00ce1b1d0cb30bac1e24dbf",
    "0af2684c4c637d6d7c4a78da98e8ec63166f88064fed27295653039999097399",
    "a2cdf638a371e4d7574f9f01846337ab760f1b3e49d08a26b2d73d68b7b30db7",
    "2561a5be6b5d69fd8074bdd3f71eeedb88cbec03ade535683129a878d536e152",
    "bf0d45b834eb480be45fb07c26f4d66639635d7ce12664a9d28feb95e056a700",
    "1f520b47708820a8a531f9fa754eb81d2d59ca418e4768c167839b126ede8f78",
    "71caad47400a537f6e712725681dfbeb878045f64ce17038a6a5fb87079862b8",
    "516cff03ab627b5246644d1356aeabe46fc7309bd2e644b587cf53c4573c48e0",
    "da91467d8fdd99e316de6da4715c99dd19854de6979d32553cea68c2ea31dac8",
    "be7795d20b005430a21e0d6a6752f0d4b235ec16460caa23b7ce88baf95b428f",
    "30c65d47466dccae36ef9bbbb47f8cab9737e88be47a17c1efb2b90fe374652d",
];

fn migrated(from: &Source, text: &str) -> Source {
    let file = adtb_lang::parse::parse_migration(text, adtb_lang::parse::Options::default())
        .ok()
        .unwrap_or_else(|_| panic!("the migration parses: {text}"));
    from.migrate(&file, text).unwrap_or_else(|f| match f {
        adtb_lang::evolve::Failed::Refused(r) => panic!(
            "the migration applies: {text}\n{:?}\n{:?}",
            r.operators, r.recheck
        ),
        adtb_lang::evolve::Failed::Broken(d) => panic!("the migration applies: {text}\n{d:?}"),
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
    for text in SAMPLE_LOG.iter().chain([&BASE]).chain(OPERATORS) {
        src = migrated(&src, text);
    }
    let schema = src.schema().expect("it builds");
    let (op_ty, clause_ty) = (
        field_ty(&schema, "migration_ops", "op"),
        field_ty(&schema, "migration_clauses", "body"),
    );
    let (mut got, mut reached) = (Vec::new(), BTreeSet::new());
    oplog::chain(&schema, &src, |ops, h| {
        for row in ops {
            reach(&schema, &op_ty, row.op(), &mut reached);
            for c in row.clauses() {
                reach(&schema, &clause_ty, c, &mut reached);
            }
        }
        got.push(hex(h));
    })
    .expect("every logged migration resolves");
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

#[test]
fn a_scope_naming_a_set_hashes_otherwise_than_one_listing_its_constructors() {
    let base = migrated(
        &migrated(&Source::default(), BASE),
        "module law\nadd set Only of ts = A",
    );
    let hash = |scope: &str| {
        let src = migrated(&base, &format!("module law\nadd unique (z) in ts.{scope}"));
        oplog::version_hash(&src).expect("the log resolves")
    };
    assert_ne!(hash("Only"), hash("(A)"));
}
