use std::collections::{BTreeMap, BTreeSet};

use super::ids::{Desc, by_desc, type_name};
use super::revert::headed;
use super::*;
use crate::ids::Space;

/// The shape a generated source varies by.
#[derive(Clone, Copy, Debug)]
struct Shape {
    /// Relations are declared before the types of their rows.
    rel_first: bool,
    /// Role `support` holds privileges on `ts`, its set `S`, a field list and `f`.
    granted: bool,
    /// `ts.D` and `P.E` are declared and not enabled.
    declared: bool,
}

/// The order shapes are tried in: the first two set each bit both ways.
const SHAPES: [u64; 8] = [7, 0, 5, 2, 1, 6, 3, 4];

impl Shape {
    fn of(bits: u64) -> Shape {
        Shape {
            rel_first: bits & 1 != 0,
            granted: bits & 2 != 0,
            declared: bits & 4 != 0,
        }
    }

    fn text(self) -> String {
        let types = "type U = { id: Key us, label: Text }
type Label = new Text
type P = A | B
type Q = Q1 | Q2
type K = A | B
type Inner = A { x: Text, z: Text, q: Q } | B { x: Text, z: Text, q: Q }
type T { id: Key ts, n: Text, o: Text, h: Key us }
  = A { x: Text, z: Text, q: Q }
  | B { x: Text, z: Text, q: Q }
  | C { z: Text, y: Text }
type V = { id: Key vs, w: Text }
type Badge = { id: I64, name: Text }
";
        let relations = "relation us : U
  key id generate sequence
  unique (label) in Named
relation ts : T
  key id generate sequence
  unique (x) in A
  unique (x) in B
  unique (n) in (A | B)
  index (n) in (A | B)
  index (z) in S
  default A.x = \"a\"
  computed tag: Text = \"t\"
relation vs : V key id generate sequence
relation badges : Badge
  key id
  unique (name)
";
        let (first, second) = if self.rel_first {
            (relations, types)
        } else {
            (types, relations)
        };
        format!(
            "module auth
{first}{second}set S of ts = A | B
set R of ts = A | B | C
set Named of us = U
fn f (t: T) -> Text = match t
  | A a -> a.x
  | B b -> b.x
  | C c -> c.z
fn spare () -> I64 = 1
aggregate longest (x: Text) -> Text =
  state {{ best: Text }}
  init {{ best: \"\" }}
  step (s, x) -> if scalar_len(x) > scalar_len(s.best) then {{ best: x }} else s
  merge (a, b) -> if scalar_len(b.best) > scalar_len(a.best) then b else a
  finish s -> s.best
constraint unlabelled = from u in us where u.label == \"\" select u
check o_set on ts (scalar_len(o) > 0)
fn labels () -> Set ({{ label: Text }}) = from u in us select {{ label: u.label }}
store labels
  key label
fn names () -> Set ({{ label: Text }}) = from u in us select {{ label: u.label }}
"
        )
    }

    /// Returns the source of the shape: its text, then the migrations that give it an empty
    /// module, grants and declared constructors.
    fn source(self) -> Source {
        let mut s = source(&self.text());
        s.packages.insert(
            "geo-1.2.adtx".into(),
            "type Cell = { lat: F64, lon: F64 }\nfn scale (x: F64) -> F64 = x * 2.0".into(),
        );
        let mut setup = vec!["module m\nadd module m".to_string()];
        if self.granted {
            setup.push(
                "module auth
grant read ts to support
grant insert ts.S to support
grant update ts.(A | B) (A.x, n) to support
grant call fn f to support"
                    .into(),
            );
        }
        if self.declared {
            setup.push(
                "module auth
add constructor T.D { w: Text }
  arm f: D d -> d.w
add constructor P.E"
                    .into(),
            );
        }
        for text in setup {
            s = migrate(&s, &text).unwrap_or_else(|e| panic!("{self:?}: {text}\nrefused: {e:?}"));
        }
        s
    }

    /// Returns the grant a structural operator states, when the shape grants.
    fn granted_line(self, line: &str) -> &str {
        if self.granted { line } else { "" }
    }

    /// Returns the arm of a `match` over `T` for `D`, when the shape declares it.
    fn d_arm(self, arm: &str) -> &str {
        if self.declared { arm } else { "" }
    }

    /// Every case of every invertible operator kind over the shape.
    fn cases(self) -> Vec<Case> {
        let d = |arm: &'static str| self.d_arm(arm);
        let (merge, nest) = (self.merge(), self.nest());
        let mut out = vec![
            Case::new("module m2\nadd module m2"),
            Case::new("module m\ndrop module m"),
            Case::new("install geo \"1.2\" from \"geo-1.2.adtx\""),
            Case::new("uninstall geo").after("install geo \"1.2\" from \"geo-1.2.adtx\""),
            Case::new("add type W = W1 | W2"),
            Case::new("add type Rec = { a: Text, b: I64 }"),
            Case::new("drop type P"),
            Case::new("drop type Label"),
            Case::new("rename type T to T2"),
            Case::new("rename type P to P2"),
            Case::new(
                "add type W = { id: Key ws, a: Text }
add relation ws : W
  key id generate sequence
  unique (a)",
            ),
            Case::new("drop relation vs\ndrop type V"),
            Case::new("rename relation ts to ts2"),
            Case::new("rename relation us to us2"),
            Case::new("add set S2 of ts = B | C"),
            Case::new("drop set R of ts"),
            Case::new("rename set S of ts to S3"),
            Case::new("add constructor T.E { w: Text }\n  arm f: E e -> e.w"),
            Case::new("add constructor P.F"),
            Case::new("remove constructor P.B").after("deprecate P.B"),
            Case::new("rename constructor T.A to A2"),
            Case::new("rename constructor P.A to A3"),
            Case::new(SPLIT).after("drop default ts.A.x"),
            Case::new(SPLIT).labelled("default A.x kept"),
            Case::new(SPLIT)
                .after(&format!("drop default ts.A.x\n{CHECK_A}"))
                .labelled("check on ts.A"),
            Case::new(&merge).after("drop default ts.A.x"),
            Case::new(&merge)
                .after(&format!("drop default ts.A.x\n{CHECK_A}"))
                .labelled("check on ts.A"),
            Case::new(&nest).after("drop index (z) in ts.S\ndrop default ts.A.x"),
            Case::new("unnest constructor T.N\n  arm f: A a -> a.x\n  arm f: B b -> b.x").after(
                &format!(
                    "drop index (z) in ts.S\ndrop default ts.A.x\n{NEST}{}",
                    self.granted_line("\ngrant update ts.N to support")
                ),
            ),
            Case::new("add field T.C.w: Text fill \"w\""),
            Case::new("add field T.m: Text fill \"m\""),
            Case::new("alter field T.h on delete cascade"),
            Case::new("drop field T.C.y").fill("T.C.y = \"y\""),
            Case::new("drop field T.B.q")
                .refused("the inverse of `drop field T.B.q` needs a fill: `fill T.B.q = e`"),
            Case::new("rename field T.A.x to x2"),
            Case::new("rename field T.n to n2"),
            Case::new("rename field U.label to name"),
            Case::new(&format!(
                "move field T.C.y to T fill match\n  | A a -> \"\"\n  | B b -> \"\"{}",
                d("\n  | D d -> \"\"")
            )),
            Case::new("move field T.n to T.(A | B)").fill(&format!(
                "T.A.n = match\n    | C c -> \"\"{}",
                d("\n    | D d -> \"\"")
            )),
            Case::new("add unique (z) in ts.C"),
            Case::new("drop unique (x) in ts.A"),
            Case::new("add index (z) in ts.B"),
            Case::new("add interval index (label) in us"),
            Case::new("drop index (n) in ts.(A | B)"),
            Case::new("drop index (z) in ts.S"),
            Case::new("rekey relation badges key name"),
            Case::new("add default ts.B.x = \"b\""),
            Case::new("add default ts.n = \"n\""),
            Case::new("drop default ts.A.x"),
            Case::new("define fn g (x: Text) -> Text = x"),
            Case::new("redefine fn spare () -> I64 = 2"),
            Case::new("drop fn spare"),
            Case::new("add constraint blank = from t in ts where t.n == \"\" select t"),
            Case::new("drop constraint unlabelled"),
            Case::new("add check z_set on ts.B (scalar_len(z) > 0)"),
            Case::new("drop check o_set"),
            Case::new("store names\n  key label"),
            Case::new("unstore labels"),
            Case::new("add computed ts.width: I64 = scalar_len(self.n)"),
            Case::new("redefine computed ts.tag: Text = \"u\""),
            Case::new("drop computed ts.tag"),
            Case::new(
                "define aggregate shortest (x: Text) -> Text =
  state { best: Text }
  init { best: \"\" }
  step (s, x) -> if scalar_len(x) < scalar_len(s.best) then { best: x } else s
  merge (a, b) -> if scalar_len(b.best) < scalar_len(a.best) then b else a
  finish s -> s.best",
            ),
            Case::new("drop aggregate longest"),
            Case::new("enable ts.B").after("deprecate ts.B"),
            Case::new("enable P.B").after("deprecate P.B"),
            Case::new("disable ts.B"),
            Case::new("disable P.B"),
            Case::new("deprecate ts.A"),
            Case::new("deprecate P.A"),
            Case::new("grant read us to support"),
            Case::new("grant delete ts.(A | B) to support"),
            Case::new("grant call fn spare to support"),
            Case::new("revoke read us from support").after("grant read us to support"),
            Case::new("grant read us to support with grant option"),
            Case::new("revert {first}").after("add type W = W1 | W2"),
            Case::new("revert {first}").after("rename field T.A.x to x2"),
        ];
        out.extend(self.removing());
        out.extend(self.structural());
        out.extend(self.products());
        out.extend(between());
        out.extend(restoring());
        out.extend(added_and_renamed());
        if self.declared {
            out.push(Case::new("enable ts.D"));
            out.push(Case::new("enable P.E"));
        }
        if self.granted {
            out.push(Case::new("drop set YS of ys\ndrop relation ys\ndrop type Y").after(Y));
            out.push(Case::new("drop fn f"));
            out.push(Case::new("drop set S of ts").after("drop index (z) in ts.S"));
            out.push(Case::new("revoke read ts from support"));
            out.push(Case::new("revoke update ts.(A | B) (A.x, n) from support"));
        }
        out
    }

    /// Returns the merge of `T.A` and `T.B` into `M`.
    fn merge(self) -> String {
        format!(
            "merge constructor T.(A | B) into M {{ k: K, x: Text, z: Text, q: Q }}
  by match
    | A a -> M {{ k: A, x: a.x, z: a.z, q: a.q }}
    | B b -> M {{ k: B, x: b.x, z: b.z, q: b.q }}
  arm f: M m -> m.x{}",
            self.granted_line("\ngrant update ts.M to support")
        )
    }

    /// Returns the nest of `T.A` and `T.B` into `N`.
    fn nest(self) -> String {
        format!(
            "{NEST}{}",
            self.granted_line("\ngrant update ts.N to support")
        )
    }

    /// The cases whose revert inverts a structural operator after a later migration renames an
    /// object its inverse names, or reuses the name of one.
    fn structural(self) -> Vec<Case> {
        let renamed = |text: &str, then: &str, expect: &str| {
            Case::new(text).then(then, Some(expect)).labelled(NAMED)
        };
        let reused = |text: &str, then: &str, expect: &str| {
            Case::new(text)
                .then(then, Some(expect))
                .labelled(NAME_REUSED)
        };
        let (merge, nest) = (self.merge(), self.nest());
        let unnest = "unnest constructor T.N\n  arm f: A a -> a.x\n  arm f: B b -> b.x";
        let nested = format!("drop index (z) in ts.S\ndrop default ts.A.x\n{nest}");
        let up = format!(
            "move field T.C.y to T fill match\n  | A a -> \"\"\n  | B b -> \"\"{}",
            self.d_arm("\n  | D d -> \"\"")
        );
        let down_fill = format!(
            "T.A.n = match\n    | C c -> \"\"{}",
            self.d_arm("\n    | D d -> \"\"")
        );
        let split_renames = "rename type T to T9
rename relation ts to ts9
rename set S of ts9 to S9
rename field T9.n to n9
rename constructor Q.Q1 to Q9";
        let merge_renames = "rename type T to T9
rename relation ts to ts9
rename set S of ts9 to S9
rename field T9.n to n9
rename constructor K.A to A7
rename type Q to Q9";
        let nest_renames = "rename type T to T9
rename relation ts to ts9
rename field T9.n to n9
rename type Inner to Inner9";
        vec![
            renamed(SPLIT, split_renames, split_renames).after("drop default ts.A.x"),
            reused(SPLIT, Q1_AGAIN, Q1_AGAIN).after("drop default ts.A.x"),
            reused(SPLIT, T_AGAIN, T_AGAIN)
                .labelled(TYPE_REUSED)
                .after("drop default ts.A.x"),
            renamed(&merge, merge_renames, merge_renames).after("drop default ts.A.x"),
            reused(&merge, KK_AGAIN, KK_AGAIN).after("drop default ts.A.x"),
            reused(&merge, T_AGAIN, T_AGAIN)
                .labelled(TYPE_REUSED)
                .after("drop default ts.A.x"),
            renamed(&nest, nest_renames, nest_renames)
                .after("drop index (z) in ts.S\ndrop default ts.A.x"),
            reused(&nest, INNER_AGAIN, INNER_AGAIN)
                .after("drop index (z) in ts.S\ndrop default ts.A.x"),
            renamed(unnest, nest_renames, nest_renames).after(&nested),
            reused(unnest, INNER_AGAIN, INNER_AGAIN).after(&nested),
            renamed(
                &up,
                "rename constructor T.C to C9\nrename field T.y to y9",
                "rename constructor T.C to C9\nrename field T.C9.y to y9",
            ),
            reused(
                &up,
                "rename field T.y to y9\nadd field T.y: Text fill \"v\"",
                "rename field T.C.y to y9\nadd field T.y: Text fill \"v\"",
            ),
            renamed(
                "move field T.n to T.(A | B)",
                "rename type T to T9\nrename constructor T9.A to A9",
                "rename type T to T9\nrename constructor T9.A to A9",
            )
            .fill(&down_fill.replacen("T.A.n", "T9.A9.n", 1)),
            reused("move field T.n to T.(A | B)", T_AGAIN, T_AGAIN)
                .labelled(TYPE_REUSED)
                .fill(&down_fill.replacen("T.A.n", "T9.A.n", 1)),
            renamed(
                "add constructor T.E { w: Text }\n  arm f: E e -> e.w",
                "rename type T to T9",
                "rename type T to T9",
            ),
            reused(
                "add constructor T.E { w: Text }\n  arm f: E e -> e.w",
                T_AGAIN,
                T_AGAIN,
            )
            .labelled(TYPE_REUSED),
            renamed(
                "add constructor V.V2 { w2: Text }",
                "rename relation vs to vs9",
                "rename relation vs to vs9",
            ),
            renamed(
                "remove constructor P.B",
                "rename constructor P.A to A9",
                "rename constructor P.A to A9",
            )
            .after("deprecate P.B"),
            reused(
                "remove constructor P.B",
                "rename constructor P.A to A9\nadd constructor P.A",
                "rename constructor P.A to A9\nadd constructor P.A",
            )
            .after("deprecate P.B"),
            renamed(
                &self.remove_c(),
                "rename type T to T9\nrename constructor T9.B to B9\nrename set R of ts to R9",
                "rename type T to T9\nrename constructor T9.B to B9\nrename set R of ts to R9",
            )
            .after(&self.keeping_s("deprecate ts.C")),
            reused(
                &self.remove_c(),
                "rename set R of ts to R9\nadd set R of ts = A",
                "rename set R of ts to R9\nadd set R of ts = A",
            )
            .after(&self.keeping_s("deprecate ts.C")),
            Case::new("add constructor V.V2 { w2: Text }")
                .then(
                    "add field V.V.e: Text fill \"e\"\nadd field V.V.g: Text fill \"g\"",
                    Some("add field V.e: Text fill \"e\"\nadd field V.g: Text fill \"g\""),
                )
                .labelled(CTOR_FIELD_ADDED),
            Case::new("add constructor V.V2 { w2: Text }")
                .then(
                    "add field V.e: Text fill \"e\"",
                    Some("add field V.e: Text fill \"e\""),
                )
                .labelled(HEADER_FIELD_ADDED),
        ]
    }

    /// The cases whose revert inverts a nest after a later migration renames constructors of the
    /// nested type.
    fn products(self) -> Vec<Case> {
        let nest = self.nest();
        let swap = "rename constructor Inner.A to Z
rename constructor Inner.B to A
rename constructor Inner.Z to B";
        vec![
            Case::new(&nest)
                .after("drop index (z) in ts.S\ndrop default ts.A.x")
                .then(
                    "rename constructor Inner.A to A9",
                    Some("rename constructor Inner.A to A9"),
                )
                .labelled(PRODUCT_RENAMED),
            Case::new(&nest)
                .after("drop index (z) in ts.S\ndrop default ts.A.x")
                .then(swap, Some(swap))
                .labelled(PRODUCTS_SWAPPED),
        ]
    }

    /// Returns the removal of `T.C` with `f` redefined without its arm.
    fn remove_c(self) -> String {
        format!(
            "remove constructor T.C
redefine fn f (t: T) -> Text = match t
  | A a -> a.x
  | B b -> b.x
  | G g -> \"\"{}",
            self.d_arm("\n  | D d -> d.w")
        )
    }

    /// Returns `after` preceded by adding constructor `G` of `T`.
    fn keeping_s(self, after: &str) -> String {
        format!("add constructor T.G\n  arm f: G g -> \"\"\n{after}")
    }

    /// The cases removing constructor `C` of `T`.
    fn removing(self) -> Vec<Case> {
        let remove = self.remove_c();
        vec![
            Case::new(&remove).after(&self.keeping_s("deprecate ts.C")),
            Case::new(&remove)
                .after(&self.keeping_s(
                    "drop set R of ts
add type Z { id: Key zs } = C | Z2
add relation zs : Z key id generate sequence
deprecate ts.C",
                ))
                .then("deprecate zs.C", Some("deprecate zs.C"))
                .labelled(OTHER_TYPE),
            Case::new(&remove)
                .after(&self.keeping_s(&format!(
                    "add constraint cz = from t in ts where match t
  | A a -> a.x == \"\"
  | B b -> false
  | C c -> c.z == \"\"
  | G g -> false{}
  select t
deprecate ts.C",
                    self.d_arm("\n  | D d -> false")
                )))
                .labelled(CONSTRAINT_ARM),
            Case::new(&remove)
                .after(&self.keeping_s(&format!(
                    "add constraint cz = from t in ts
  where (match t | A a -> a.x == \"\" | B b -> false | C c -> c.z == \"\" | G g -> false{d})
    and (match t | A a -> false | B b -> b.x == \"\" | C c -> c.y == \"\" | G g -> false{d})
  select t
deprecate ts.C",
                    d = self.d_arm(" | D d -> false")
                )))
                .labelled(TWO_MATCHES),
        ]
    }
}

/// A relation of a sum with every kind of clause, a set, a declared and a deprecated
/// constructor, and grants on the relation and the set.
const Y: &str = "add type Y { id: Key ys } = Y1 { a: Text } | Y2 { b: Text }
add relation ys : Y
  key id generate sequence
  unique (a) in Y1
  default Y1.a = \"a\"
  computed c: Text = \"c\"
add set YS of ys = Y1 | Y2
add constructor Y.Y3
deprecate ys.Y2
grant read ys to support
grant insert ys.YS to support";

/// The cases of migrations that add an object and rename it, and of fills.
fn added_and_renamed() -> Vec<Case> {
    vec![
        Case::new("add type X = X1 | X2\nrename type X to Y"),
        Case::new(
            "add type W = { id: Key ws, a: Text }
add relation ws : W key id generate sequence
rename relation ws to ws2
rename type W to W2",
        ),
        Case::new("add constructor P.F\nrename constructor P.F to G"),
        Case::new("add type X = X1 | X2\nrename type X to Y\nadd type X = Z1 | Z2"),
        Case::new("drop field T.C.y\nrename constructor T.C to C2").fill("T.C2.y = \"y\""),
        Case::new("drop field T.C.y")
            .fill("T.C.y = \"y\"")
            .fill("T.C.typo = \"y\"")
            .refused("no inverse of migration {case} needs `fill T.C.typo`"),
    ]
}

/// The cases of renames and set operators whose revert follows a later migration that renames
/// the object or what holds it, reuses a name, or drops the object.
fn between() -> Vec<Case> {
    vec![
        Case::new("rename type P to P2")
            .then(
                "rename type P2 to P3\nadd type P2 = Z1 | Z2",
                Some("add type P2 = Z1 | Z2"),
            )
            .labelled(AGAIN),
        Case::new("rename relation us to us2")
            .then(
                &format!("rename relation us2 to us3\n{US2}"),
                Some(US2),
            )
            .labelled(AGAIN),
        Case::new("rename set S of ts to S3")
            .then(
                "rename relation ts to ts2
rename set S3 of ts2 to S4
add set S3 of ts2 = C",
                Some("rename relation ts to ts2\nadd set S3 of ts2 = C"),
            )
            .labelled(AGAIN),
        Case::new("rename set S of ts to S3")
            .then("add set S of ts = C", None)
            .refused(
                "the inverse `rename set S3 of auth::ts to S` does not apply: `ts` already has a constructor or named set `S`",
            )
            .labelled(REUSED),
        Case::new("rename set S of ts to S3")
            .then("drop index (z) in ts.S3\ndrop set S3 of ts", None)
            .refused(
                "the inverse of `rename set S of ts to S3` names a set a later migration dropped",
            )
            .labelled(DROPPED),
        Case::new("rename constructor P.A to A3")
            .then(
                "rename type P to P2
rename constructor P2.A3 to A4
add constructor P2.A3",
                Some("rename type P to P2\nadd constructor P2.A3"),
            )
            .labelled(AGAIN),
        Case::new("rename field T.A.x to x2")
            .then(
                "rename constructor T.A to A9
rename field T.A9.x2 to x3
add field T.A9.x2: Text fill \"v\"",
                Some("rename constructor T.A to A9\nadd field T.A9.x2: Text fill \"v\""),
            )
            .labelled(AGAIN),
        Case::new("rename field T.n to n2")
            .then("rename type T to T9", Some("rename type T to T9"))
            .labelled(PARENT),
        Case::new("rename field U.label to name")
            .then(
                "rename type U to U9
rename field U9.name to title
add field U9.name: Text fill \"v\"",
                Some("rename type U to U9\nadd field U9.name: Text fill \"v\""),
            )
            .labelled(AGAIN),
        Case::new("add set S2 of ts = B | C")
            .then("rename relation ts to ts2", Some("rename relation ts to ts2"))
            .labelled(PARENT),
        Case::new("add set S2 of ts = B | C")
            .then("rename set S2 of ts to S5", None)
            .refused(
                "`revert {case}` is refused while migration {then} depends on it: it names set `auth::ts.S2`",
            )
            .labelled(RENAMED),
        Case::new("drop set R of ts")
            .then(
                "rename relation ts to ts2\nrename constructor T.C to C2",
                Some("rename relation ts to ts2\nrename constructor T.C to C2"),
            )
            .labelled(PARENT),
        Case::new("drop set R of ts")
            .then("add set R of ts = A", None)
            .refused(
                "the inverse `add set R of ts = A | B | C` does not apply: module `auth` already declares set `ts.R`",
            )
            .labelled(REUSED),
        Case::new(
            "drop set R of ts
add set S6 of ts = A
rename set S6 of ts to R
drop set R of ts",
        )
        .labelled(RECREATED),
        Case::new(
            "drop type P
add type P = A | B
rename constructor P.A to A5
drop type P",
        )
        .labelled(RECREATED),
        Case::new("drop relation m::ms\ndrop type m::M")
            .after(
                "module m
add type M { id: Key ms } = M1 | M2
add relation ms : M key id generate sequence
deprecate ms.M2",
            )
            .labelled(OTHER_MODULE),
        Case::new("deprecate P.A")
            .then("drop type P", None)
            .refused("the inverse of `deprecate P.A` names a constructor a later migration dropped")
            .labelled(DROPPED),
        Case::new("deprecate vs.V")
            .then("drop relation vs\ndrop type V", None)
            .refused("the inverse of `deprecate vs.V` names a relation a later migration dropped")
            .labelled(DROPPED),
        Case::new("grant read vs to support")
            .then("drop relation vs\ndrop type V", None)
            .refused(
                "the inverse of `grant read vs to support` names a relation a later migration dropped",
            )
            .labelled(DROPPED),
        Case::new("revoke read vs from support")
            .after("grant read vs to support")
            .then("drop relation vs\ndrop type V", None)
            .refused(
                "the inverse of `revoke read vs from support` names a relation a later migration dropped",
            )
            .labelled(DROPPED),
    ]
}

/// The cases whose revert restores a text naming an object a later migration renames, takes the
/// name of, or drops.
fn restoring() -> Vec<Case> {
    let renamed = |text: &str, then: &str| Case::new(text).then(then, Some(then)).labelled(NAMED);
    vec![
        renamed("drop type Inner", "rename type Q to Q9"),
        renamed(
            "drop relation badges",
            "rename type Badge to Badge9\nrename field Badge9.name to title",
        ),
        renamed("drop fn kk", "rename constructor K.A to A7").after(KK),
        renamed(
            "drop constraint unlabelled",
            "rename relation us to us9\nrename field U.label to title",
        ),
        renamed(
            "drop check o_set",
            "rename relation ts to ts9\nrename field T.o to o9",
        ),
        renamed(
            "redefine fn names () -> Set ({ label: Text }) = from u in us select { label: \"x\" }",
            "rename relation us to us9\nrename field U.label to title",
        ),
        renamed(
            "drop unique (x) in ts.A",
            "rename relation ts to ts9\nrename constructor T.A to A9\nrename field T.A9.x to x9",
        ),
        renamed("drop index (z) in ts.S", "rename set S of ts to S9"),
        renamed("add unique (z) in ts.C", "rename constructor T.C to C9"),
        renamed("add index (z) in ts.B", "rename relation ts to ts9"),
        renamed(
            "add interval index (label) in us",
            "rename field U.label to title",
        ),
        renamed(
            "drop default ts.A.x",
            "rename constructor T.A to A9\nrename field T.A9.x to x9",
        ),
        renamed(
            "add default ts.B.x = \"b\"",
            "rename relation ts to ts9\nrename field T.B.x to x9",
        ),
        renamed("drop computed ts.width", "rename relation ts to ts9")
            .after("add computed ts.width: I64 = scalar_len(self.n)"),
        renamed(
            "redefine computed ts.tag: Text = \"u\"",
            "rename relation ts to ts9",
        ),
        renamed(
            "rekey relation badges key name",
            "rename field Badge.id to ident",
        ),
        renamed(
            "add field T.C.w: Text fill \"w\"",
            "rename constructor T.C to C9",
        ),
        renamed(
            "alter field T.h on delete cascade",
            "rename field T.h to h9",
        ),
        renamed("drop field T.C.y", "rename field T.C.z to z9").fill("T.C.y = \"y\""),
        renamed("drop field T.A.q", "rename type Q to Q9").fill("T.A.q = Q1"),
        renamed("revert {first}", "rename constructor T.C to C9").after("add unique (z) in ts.C"),
        renamed("revert {first}", "rename field T.B.x to x9").after("add default ts.B.x = \"b\""),
        Case::new("drop fn kk")
            .after(KK)
            .then(KK_AGAIN, Some(KK_AGAIN))
            .labelled(NAME_REUSED),
        Case::new("drop unique (x) in ts.A")
            .then(X_AGAIN, Some(X_AGAIN))
            .labelled(NAME_REUSED),
        Case::new("revert {first}")
            .after("add unique (z) in ts.C")
            .then(C_AGAIN, Some(C_AGAIN))
            .labelled(NAME_REUSED),
        Case::new("unstore labels")
            .then(LABELS_AGAIN, None)
            .refused("the inverse of `unstore labels` names a function a later migration dropped")
            .labelled(DROPPED),
        Case::new("drop unique (k) in ts.A")
            .after("add field T.A.k: Text fill \"k\"\nadd unique (k) in ts.A")
            .then("drop field T.A.k", None)
            .refused(
                "the inverse of `drop unique (k) in ts.A` names a field a later migration dropped",
            )
            .labelled(DROPPED),
        Case::new("drop index (z) in ts.S")
            .then("rename field T.A.z to z9", None)
            .refused(
                "in the inverse of `drop index (z) in ts.S`, the name `z` stood for objects a later migration renamed apart",
            )
            .labelled(APART),
        Case::new("drop index (z) in ts.S")
            .then("drop field T.B.z", None)
            .refused(
                "the inverse of `drop index (z) in ts.S` names a field a later migration dropped",
            )
            .labelled(DROPPED),
        Case::new("drop index (z) in ts.S")
            .then("drop field T.B.z\nadd field T.B.z: Text fill \"q\"", None)
            .refused(
                "the inverse of `drop index (z) in ts.S` names a field a later migration dropped",
            )
            .labelled(DROPPED),
        Case::new("drop fn g")
            .after(G_SHADOWING)
            .then("rename field Inner.A.x to x9", Some("rename field Inner.A.x to x9"))
            .labelled(PUN_SHADOWING),
        Case::new("drop fn g")
            .after(G)
            .then("rename field Inner.A.x to x9", Some("rename field Inner.A.x to x9"))
            .labelled(PUN),
        Case::new("drop relation vs\ndrop type V")
            .then(VS_AGAIN, None)
            .refused("module `auth` already declares relation `vs`")
            .labelled(NAME_REUSED),
        Case::new("revert {first}")
            .after(ZYS)
            .after("add constructor Zy.Z3 { w: Text }\n  arm zys.zzk: Z3 p -> p.w")
            .then(ZZK_AGAIN, None)
            .refused(
                "the inverse of `revert {first}` names a computed field a later migration dropped",
            )
            .labelled(DROPPED),
        Case::new("drop type Wrap")
            .after("add type Wrap = { l: Label }")
            .then("drop type Label", None)
            .refused("the inverse of `drop type Wrap` names a type a later migration dropped")
            .labelled(DROPPED),
    ]
}

/// A function over `K` matching each of its constructors.
const KK: &str = "define fn kk (k: K) -> I64 = match k
  | A a -> 1
  | B b -> 2";

/// A relation with a computed field matching on its row.
const ZYS: &str = "add type Zy { id: Key zys } = Z1 { a: Text } | Z2 { b: Text }
add relation zys : Zy key id generate sequence
add computed zys.zzk: Text = match self
  | Z1 p -> p.a
  | Z2 p -> p.b";

/// A new computed field taking the name of `zys.zzk`.
const ZZK_AGAIN: &str = "drop computed zys.zzk
add computed zys.zzk: Text = match self
  | Z1 p -> p.a
  | Z2 p -> p.b";

/// A new relation taking the name of `vs`, over a new type.
const VS_AGAIN: &str = "add type V3 = { id: Key vs }
add relation vs : V3 key id generate sequence";

/// The label of a case whose revert restores a pun of a field a later migration renames.
const PUN: &str = "punned field renamed";

/// The label of a case whose revert restores a pun shadowing a parameter, of a field a later
/// migration renames.
const PUN_SHADOWING: &str = "punned field shadowing a parameter renamed";

/// A function over `Inner` binding field `x` of `A` by a pun that shadows a parameter.
const G_SHADOWING: &str = "define fn g (i: Inner, x: Text) -> Text = match i
  | A { x } -> x
  | B b -> b.x";

/// A function over `Inner` binding field `x` of `A` by a pun.
const G: &str = "define fn g (i: Inner) -> Text = match i
  | A { x } -> x
  | B b -> b.x";

/// A rename of `T` and a new type taking its name.
const T_AGAIN: &str = "rename type T to T9\nadd type T = Z1 | Z2";

/// A rename of `Inner` and a new type taking its name.
const INNER_AGAIN: &str = "rename type Inner to Inner9\nadd type Inner = Z1 | Z2";

/// A rename of `Q.Q1` and a new constructor of `Q` taking its name.
const Q1_AGAIN: &str = "rename constructor Q.Q1 to Q9\nadd constructor Q.Q1";

/// A rename of `K` and a new type taking its name and its constructors' names.
const KK_AGAIN: &str = "rename type K to K9\nadd type K = A | B";

/// A rename of `T.A.x` and a new field taking its name.
const X_AGAIN: &str = "rename field T.A.x to x9\nadd field T.A.x: Text fill \"v\"";

/// A rename of `T.C` and a new constructor of `T` taking its name.
const C_AGAIN: &str = "rename constructor T.C to C9
add constructor T.C { z: Text }
  arm f: C c -> c.z";

/// A new function taking the name of `labels`.
const LABELS_AGAIN: &str = "drop fn labels
define fn labels () -> Set ({ label: Text }) = from u in us select { label: u.label }";

/// The label of a case whose revert restores a text naming by one name two objects a later
/// migration gives different names.
const APART: &str = "named objects renamed apart";

/// The label of a case whose revert restores a text naming an object a later migration renames.
const NAMED: &str = "named object renamed";

/// The label of a case whose revert restores a text naming an object a later migration renames,
/// a new object taking its name.
const NAME_REUSED: &str = "named object's name reused";

/// A relation `us2` of a type of its own.
const US2: &str = "add type U2 = { id: Key us2 }
add relation us2 : U2 key id generate sequence";

/// The label of a case whose revert inverts a structural operator over `T` after a later
/// migration renames `T`, a new type taking its name.
const TYPE_REUSED: &str = "type's name reused";

/// The label of a case whose revert puts back an arm of a constraint.
const CONSTRAINT_ARM: &str = "constraint arm";

/// The label of a case whose revert puts back an arm in each of two matches of a constraint.
const TWO_MATCHES: &str = "constraint arms in two matches";

/// The label of a case whose revert follows a field added to the record's constructor.
const CTOR_FIELD_ADDED: &str = "constructor field added";

/// The label of a case whose revert follows a field added to the header.
const HEADER_FIELD_ADDED: &str = "header field added";

/// The label of a case whose revert follows a rename of a constructor of the nested type.
const PRODUCT_RENAMED: &str = "product renamed";

/// The label of a case whose revert follows a swap of the names of the nested type's
/// constructors.
const PRODUCTS_SWAPPED: &str = "products swapped";

/// The label of a case whose revert follows a rename of its object, a later name of it reused.
const AGAIN: &str = "renamed again, name reused";

/// The label of a case whose revert follows a rename of its object.
const RENAMED: &str = "object renamed";

/// The label of a case whose revert follows a rename of what holds its object.
const PARENT: &str = "owner renamed";

/// The label of a case whose revert follows a new object taking the name it restores.
const REUSED: &str = "name taken";

/// The label of a case dropping an object whose name an object it adds then takes.
const RECREATED: &str = "name re-created";

/// The label of a case whose revert follows a change of the state of a constructor of the same
/// name in another type.
const OTHER_TYPE: &str = "same name in another type";

/// The label of a case whose migration drops objects of another module.
const OTHER_MODULE: &str = "objects of another module";

/// The label of a case whose revert follows a drop of its object.
const DROPPED: &str = "object dropped";

const SPLIT: &str = "split constructor T.A into A1 { x: Text, z: Text } | A2 { x: Text, z: Text }
  by match
    | A { q: Q1, x, z } -> A1 { x: x, z: z }
    | A { q: Q2, x, z } -> A2 { x: x, z: z }
  arm f: A1 a -> a.x
  arm f: A2 a -> a.x";

const CHECK_A: &str = "add check x_set on ts.A (scalar_len(x) > 0)";

const NEST: &str = "nest constructor T.(A | B) into N { inner: Inner }
  arm f: N n -> match n.inner | A a -> a.x | B b -> b.x";

/// One operator migration of the search, after the migration it needs first if any.
struct Case {
    /// The migrations applied before the case's, in order.
    first: Vec<String>,
    text: String,
    /// A migration applied after the case's and before its revert.
    then: Option<String>,
    /// The migration that takes the source before the case to the one its revert restores, when
    /// `then` changes what the revert restores.
    expect: Option<String>,
    /// The `fill` clauses its revert states.
    fills: Vec<String>,
    /// The reason the spec states for refusing its revert, when it is refused, `{case}` standing
    /// for the number of the case's migration, `{then}` for the one after it and `{first}` for the
    /// one before it.
    refused: Option<&'static str>,
    /// What tells the case apart from another of the same operators.
    label: Option<&'static str>,
}

impl Case {
    fn new(text: &str) -> Case {
        Case {
            first: Vec::new(),
            text: headed(text),
            then: None,
            expect: None,
            fills: Vec::new(),
            refused: None,
            label: None,
        }
    }

    fn after(mut self, first: &str) -> Case {
        self.first.push(headed(first));
        self
    }

    /// Returns the case with `then` applied before its revert, which then restores the source
    /// before the case with `expect` applied.
    fn then(mut self, then: &str, expect: Option<&str>) -> Case {
        self.then = Some(headed(then));
        self.expect = expect.map(headed);
        self
    }

    fn refused(mut self, reason: &'static str) -> Case {
        self.refused = Some(reason);
        self
    }

    fn fill(mut self, fill: &str) -> Case {
        self.fills.push(fill.to_string());
        self
    }

    fn labelled(mut self, label: &'static str) -> Case {
        self.label = Some(label);
        self
    }

    /// Returns the case's name: the first line of its operators, with the first line of the last
    /// migration it needs first for a `{first}` in it, then its label.
    fn name(&self) -> String {
        let line = |t: &str| t.lines().nth(1).unwrap_or_default().to_string();
        let mut first = line(&self.text);
        if let Some(before) = self.first.last()
            && first.contains("{first}")
        {
            first = first.replace("{first}", &format!("`{}`", line(before)));
        }
        match self.label {
            Some(l) => format!("{first} [{l}]"),
            None => first,
        }
    }
}

/// Returns the name of the `catalog::Operator` constructor that logs an operator of `kind`.
fn kind_name(kind: &OperatorKind) -> &'static str {
    match kind {
        OperatorKind::AddModule(_) => "AddModule",
        OperatorKind::DropModule(_) => "DropModule",
        OperatorKind::AddType(_) => "AddType",
        OperatorKind::DropType(_) => "DropType",
        OperatorKind::RenameType(..) => "RenameType",
        OperatorKind::AddRelation(_) => "AddRelation",
        OperatorKind::DropRelation(_) => "DropRelation",
        OperatorKind::RenameRelation(..) => "RenameRelation",
        OperatorKind::AddSet(_) => "AddSet",
        OperatorKind::DropSet(..) => "DropSet",
        OperatorKind::RenameSet(..) => "RenameSet",
        OperatorKind::AddConstructor { .. } => "AddConstructor",
        OperatorKind::RemoveConstructor(_) => "RemoveConstructor",
        OperatorKind::RenameConstructor(..) => "RenameConstructor",
        OperatorKind::SplitConstructor { .. } => "Split",
        OperatorKind::MergeConstructor { .. } => "Merge",
        OperatorKind::NestConstructor { .. } => "Nest",
        OperatorKind::UnnestConstructor(..) => "Unnest",
        OperatorKind::AddField { .. } => "AddField",
        OperatorKind::AlterField { .. } => "AlterField",
        OperatorKind::DropField(_) => "DropField",
        OperatorKind::RenameField(..) => "RenameField",
        OperatorKind::MoveField { .. } => "MoveField",
        OperatorKind::AddUnique(..) => "AddUnique",
        OperatorKind::DropUnique(..) => "DropUnique",
        OperatorKind::AddIndex { .. } => "AddIndex",
        OperatorKind::DropIndex { .. } => "DropIndex",
        OperatorKind::Rekey { .. } => "Rekey",
        OperatorKind::AddDefault { .. } => "AddDefault",
        OperatorKind::DropDefault { .. } => "DropDefault",
        OperatorKind::DefineFn(_) => "DefineFn",
        OperatorKind::RedefineFn(_) => "RedefineFn",
        OperatorKind::DropFn(_) => "DropFn",
        OperatorKind::AddConstraint(_) => "AddConstraint",
        OperatorKind::DropConstraint(_) => "DropConstraint",
        OperatorKind::AddCheck(_) => "AddCheck",
        OperatorKind::DropCheck(_) => "DropCheck",
        OperatorKind::Store(_) => "Store",
        OperatorKind::Unstore(_) => "Unstore",
        OperatorKind::AddComputed { .. } => "AddComputed",
        OperatorKind::RedefineComputed { .. } => "RedefineComputed",
        OperatorKind::DropComputed(..) => "DropComputed",
        OperatorKind::DefineAggregate(_) => "DefineAggregate",
        OperatorKind::DropAggregate(_) => "DropAggregate",
        OperatorKind::Enable(_) => "Enable",
        OperatorKind::Disable(_) => "Disable",
        OperatorKind::Deprecate(_) => "Deprecate",
        OperatorKind::Revert { .. } => "Revert",
        OperatorKind::Script(_) => "Script",
        OperatorKind::Install { .. } => "Install",
        OperatorKind::Uninstall(_) => "Uninstall",
        OperatorKind::AdoptTzdb { .. } => "AdoptTzdb",
        OperatorKind::Grant(_) => "Grant",
        OperatorKind::Revoke(_) => "Revoke",
    }
}

/// The operator kinds outside the law: `script`, `revert` and `adopt tzdb`.
const UNINVERTED: [&str; 3] = ["Script", "Revert", "AdoptTzdb"];

/// Returns every operator kind but [`UNINVERTED`], by the `catalog::Operator` constructor that
/// logs it.
fn invertible() -> BTreeSet<&'static str> {
    let (.., ctors) = crate::ids::TYPES
        .iter()
        .find(|(m, t, ..)| *m == "catalog" && *t == "Operator")
        .expect("the catalog declares `Operator`");
    ctors
        .iter()
        .map(|(c, _)| *c)
        .filter(|c| !UNINVERTED.contains(c))
        .collect()
}

/// What of a source a revert must restore.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Aspect {
    /// The operator is refused.
    Unapplied,
    /// The revert is refused.
    Refused,
    /// The declarations and their texts.
    Decls,
    /// The user modules and the installed ones.
    Modules,
    /// The stage of each constructor relation's write state.
    States,
    /// The stage of each value-level constructor's write state.
    ValueStates,
    /// The privileges each role holds, by the names of what they name.
    Grants,
    /// The catalog id of each object.
    Ids,
    /// A migration that applied does not resolve.
    Unresolved,
}

/// Returns the declarations of `s` by their meaning, as [`meaning::decls`] gives them.
fn decls(s: &Source) -> Vec<(String, String)> {
    meaning::decls(&s.decls)
}

/// A schema object with a catalog id: a clause of a relation by the relation and the clause's
/// text, and for a default the field it fills; any other by its description.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Object {
    Named(Desc),
    Clause(Space, String, String, Option<Desc>),
}

impl Object {
    fn space(&self) -> Space {
        match self {
            Object::Named(d) => d.space(),
            Object::Clause(space, ..) => *space,
        }
    }
}

/// The texts of the clauses of a relation declaration: its uniques and indexes, its defaults
/// and its computed fields, each in written order.
#[derive(Default)]
struct Clauses {
    indexes: Vec<String>,
    defaults: Vec<String>,
    computed: Vec<String>,
}

fn clauses(s: &Source, relation: &str) -> Option<Clauses> {
    let (m, name) = relation.split_once("::").expect("a qualified relation");
    let i = s.find("relation", m, name)?;
    let (_, all) = meaning::relation_clauses(&s.decls[i].text);
    let mut out = Clauses::default();
    for clause in all {
        match clause.split(' ').next() {
            Some("unique" | "interval" | "index") => out.indexes.push(clause),
            Some("default") => out.defaults.push(clause),
            Some("computed") => out.computed.push(clause),
            _ => {}
        }
    }
    Some(out)
}

/// Returns every object of `s` that has a catalog id, with its id, a clause after the first of
/// the same text numbered in written order.
fn objects(s: &Source, what: &str) -> BTreeMap<Object, u64> {
    let schema = s
        .schema()
        .unwrap_or_else(|e| panic!("{what}: builds: {e:?}"));
    let mut texts: BTreeMap<String, Option<Clauses>> = BTreeMap::new();
    let mut out = BTreeMap::new();
    for (d, id) in by_desc(&schema, what) {
        let mut clause = |r: &str, pick: fn(&Clauses) -> &Vec<String>, at: usize| {
            let c = texts.entry(r.to_string()).or_insert_with(|| clauses(s, r));
            c.as_ref().map(|c| {
                pick(c)
                    .get(at)
                    .unwrap_or_else(|| panic!("{what}: `{r}` has no clause {at} of {d:?}"))
                    .clone()
            })
        };
        let user = match &d {
            Desc::Index(r, at) => clause(r, |c| &c.indexes, *at)
                .map(|t| Object::Clause(Space::Index, r.clone(), t, None)),
            Desc::Default(r, f) => texts
                .entry(r.to_string())
                .or_insert_with(|| clauses(s, r))
                .as_ref()
                .map(|c| {
                    let value = default_value(&c.defaults, f)
                        .unwrap_or_else(|| panic!("{what}: `{r}` has no default of {f:?}"));
                    Object::Clause(Space::Default, r.clone(), value, Some((**f).clone()))
                }),
            Desc::Computed(r, at) => clause(r, |c| &c.computed, *at)
                .map(|t| Object::Clause(Space::Computed, r.clone(), t, None)),
            _ => None,
        };
        let mut o = user.unwrap_or_else(|| Object::Named(d.clone()));
        if let Object::Clause(_, _, text, _) = &mut o {
            let written = text.clone();
            let mut n = 1;
            while out.contains_key(&o) {
                n += 1;
                if let Object::Clause(_, _, text, _) = &mut o {
                    *text = format!("{written} #{n}");
                }
            }
        }
        if let Some(other) = out.insert(o.clone(), id) {
            panic!("{what}: {o:?} holds {other} and {id}");
        }
    }
    out
}

/// Returns the value of the clause of `defaults`, `default` clauses with their tokens joined by
/// spaces, that fills field `f`.
fn default_value(defaults: &[String], f: &Desc) -> Option<String> {
    let Desc::Field(_, ctor, name) = f else {
        return None;
    };
    defaults.iter().find_map(|d| {
        let (target, value) = d.strip_prefix("default ")?.split_once(" = ")?;
        let fills = match target.split_once(" . ") {
            Some((c, x)) => Some(c) == ctor.as_deref() && x == name,
            None => target == name,
        };
        fills.then(|| value.to_string())
    })
}

/// Returns object `o` with the number a twin clause takes cut.
fn unnumbered(o: &Object) -> Object {
    match o {
        Object::Clause(space, r, text, d) => {
            let text = match text.rsplit_once(" #") {
                Some((t, n)) if n.parse::<usize>().is_ok() => t,
                _ => text,
            };
            Object::Clause(*space, r.clone(), text.to_string(), d.clone())
        }
        o => o.clone(),
    }
}

/// Returns the ids of the objects of `m` that `d` describes with the number a twin takes cut,
/// in written order.
fn twins(m: &BTreeMap<Object, u64>, d: &Object) -> Vec<u64> {
    let mut found: Vec<(usize, u64)> = m
        .iter()
        .filter(|(o, _)| unnumbered(o) == *d)
        .map(|(o, &id)| {
            let n = match o {
                Object::Clause(_, _, text, _) => text
                    .rsplit_once(" #")
                    .and_then(|(_, n)| n.parse().ok())
                    .unwrap_or(1),
                _ => 1,
            };
            (n, id)
        })
        .collect();
    found.sort_unstable();
    found.into_iter().map(|(_, id)| id).collect()
}

/// Returns object `o` with the constructor of a field cut, so a header field and a field of a
/// constructor of one type and name compare equal.
fn unowned(o: &Object) -> Object {
    match o {
        Object::Named(Desc::Field(t, _, f)) => {
            Object::Named(Desc::Field(t.clone(), None, f.clone()))
        }
        o => o.clone(),
    }
}

/// The objects a privilege names, with ids replaced by the objects that hold them, and lists
/// of constructors and fields as sets.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Privilege {
    Read(Target, BTreeSet<Option<Object>>, bool),
    Insert(Target),
    Update(Target, BTreeSet<Option<Object>>, bool),
    Delete(Target),
    Retag(
        Box<(Target, Target)>,
        BTreeSet<(Option<Object>, Option<Object>)>,
    ),
    CallFn(Option<Object>),
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Target {
    Whole(Option<Object>),
    Ctors(Option<Object>, BTreeSet<Option<Object>>),
    Set(Option<Object>, Option<Object>),
}

/// A privilege as a role holds it: role, privilege, grant option and dormancy.
type Held = (String, Privilege, bool, bool);

fn grants(s: &Source, objects: &BTreeMap<Object, u64>) -> Vec<Held> {
    let by_id: BTreeMap<(Space, u64), &Object> = objects
        .iter()
        .map(|(o, id)| ((o.space(), *id), o))
        .collect();
    let of = |space: Space, id: u64| by_id.get(&(space, id)).map(|o| (*o).clone());
    let shared = |id: &u64| of(Space::Shared, *id);
    let target = |t: &GrantTarget| {
        let rel = shared(&t.relation);
        match &t.sel {
            GrantSel::Whole => Target::Whole(rel),
            GrantSel::Ctors(cs) => Target::Ctors(rel, cs.iter().map(shared).collect()),
            GrantSel::Set(set) => Target::Set(rel, of(Space::Set, *set)),
        }
    };
    let fields = |fs: &GrantFields| fs.ids.iter().map(|id| of(Space::Field, *id)).collect();
    let mut out: Vec<Held> = s
        .grants
        .iter()
        .map(|g| {
            let p = match &g.privilege {
                Granted::Read(t, fs) => Privilege::Read(target(t), fields(fs), fs.listed),
                Granted::Insert(t) => Privilege::Insert(target(t)),
                Granted::Update(t, fs) => Privilege::Update(target(t), fields(fs), fs.listed),
                Granted::Delete(t) => Privilege::Delete(target(t)),
                Granted::Retag(a, b, except) => Privilege::Retag(
                    Box::new((target(a), target(b))),
                    except.iter().map(|(x, y)| (shared(x), shared(y))).collect(),
                ),
                Granted::CallFn(f) => Privilege::CallFn(of(Space::Function, *f)),
            };
            (g.role.clone(), p, g.grant_option, g.dormant)
        })
        .collect();
    out.sort();
    out
}

/// Returns how the ids of `now`, after a revert of the operator that took `was` to `mid`,
/// differ from those of `was`: an object `mid` holds the id of keeps it, the objects `mid`
/// lacks are created again by the inverse with ids its space gave out after `mid`, a type before
/// its constructors and constructors in listed order, and an object a later migration created,
/// whose id in `was` `base` does not hold, keeps the id `mid` gives it. A field `mid` describes
/// under another part of its type (a header field the revert makes a record field) is matched
/// by type and name among the objects whose ids a later migration gave: ids `mid` holds and
/// `cased`, the source after the case, and `base` do not. Twins, clauses of one text, are
/// compared as a sequence in written order: those of `now` that `base` lacks against those of
/// `mid` a later migration gave.
fn id_changes(
    was: &Source,
    mid: &Source,
    now: &Source,
    [base, cased]: [&Source; 2],
) -> Vec<(Aspect, String)> {
    let (ow, om, on) = (
        objects(was, "before"),
        objects(mid, "after the operator"),
        objects(now, "after the revert"),
    );
    let held_base: BTreeSet<(Space, u64)> = objects(base, "before the case")
        .iter()
        .map(|(o, id)| (o.space(), *id))
        .collect();
    let held = |m: &BTreeMap<Object, u64>| -> BTreeSet<(Space, u64)> {
        m.iter().map(|(o, id)| (o.space(), *id)).collect()
    };
    let (held_was, held_mid) = (held(&ow), held(&om));
    let held_cased = held(&objects(cased, "after the case"));
    let later = |o: &Object, id: u64| {
        !held_cased.contains(&(o.space(), id)) && !held_base.contains(&(o.space(), id))
    };
    let mut out = Vec::new();
    let mut twinned = BTreeSet::new();
    for (o, &id) in &ow {
        let got = on.get(o).copied();
        if !held_base.contains(&(o.space(), id)) {
            let d = unnumbered(o);
            if [&ow, &om, &on].iter().any(|m| twins(m, &d).len() > 1) {
                if twinned.insert(d.clone()) {
                    let want: Vec<u64> = twins(&om, &d)
                        .into_iter()
                        .filter(|&i| later(&d, i))
                        .collect();
                    let now: Vec<u64> = twins(&on, &d)
                        .into_iter()
                        .filter(|&i| !held_base.contains(&(d.space(), i)))
                        .collect();
                    if now != want {
                        out.push(format!(
                            "{d:?}, twins created by a later migration, hold {now:?} in written order, not {want:?}"
                        ));
                    }
                }
                continue;
            }
            let want = match om.get(o) {
                Some(&m) => Some(m),
                None => {
                    let mut moved = om
                        .iter()
                        .filter(|(m, i)| unowned(m) == unowned(o) && later(m, **i));
                    match (moved.next(), moved.next()) {
                        (Some((_, &i)), None) => Some(i),
                        _ => None,
                    }
                }
            };
            if want.is_none() || got != want {
                out.push(format!("{o:?}, created by a later migration, keeps no id"));
            }
        } else if held_mid.contains(&(o.space(), id)) {
            if got != Some(id) {
                out.push(format!("{o:?} keeps no id"));
            }
        } else if got.is_some_and(|g| {
            held_was.contains(&(o.space(), g)) || held_mid.contains(&(o.space(), g))
        }) {
            out.push(format!("{o:?} is created again with an id held before"));
        } else if got.is_none() {
            out.push(format!("{o:?} is gone"));
        }
    }
    for o in on.keys() {
        if !ow.contains_key(o) {
            out.push(format!("{o:?} is new"));
        }
    }
    let mut fresh: BTreeMap<Space, Vec<u64>> = BTreeMap::new();
    for (o, &id) in &on {
        if !held_was.contains(&(o.space(), id)) && !held_mid.contains(&(o.space(), id)) {
            fresh.entry(o.space()).or_default().push(id);
        }
    }
    for (space, mut ids) in fresh {
        ids.sort_unstable();
        let first = mid.next.get(&space).copied().unwrap_or(1024);
        let end = now.next.get(&space).copied().unwrap_or(1024);
        if ids.iter().any(|id| !(first..end).contains(id)) {
            out.push(format!(
                "{space:?}: the ids created again are not the next ones"
            ));
        }
    }
    out.extend(ctor_order(now, &on, &held_was, &held_mid));
    out.into_iter().map(|l| (Aspect::Ids, l)).collect()
}

/// Returns each type of `now` whose constructors, created again, do not take ids in listed
/// order after the type's.
fn ctor_order(
    now: &Source,
    on: &BTreeMap<Object, u64>,
    held_was: &BTreeSet<(Space, u64)>,
    held_mid: &BTreeSet<(Space, u64)>,
) -> Vec<String> {
    let schema = now.schema().unwrap_or_else(|e| panic!("builds: {e:?}"));
    let fresh = |id: u64| {
        !held_was.contains(&(Space::Shared, id)) && !held_mid.contains(&(Space::Shared, id))
    };
    let mut out = Vec::new();
    for (i, t) in schema.types.iter().enumerate() {
        let name = type_name(&schema, TypeId(i as u32));
        let ty = on.get(&Object::Named(Desc::Type(name.clone()))).copied();
        let ctors: Vec<u64> = t
            .ctors()
            .iter()
            .filter_map(|c| on.get(&Object::Named(Desc::Ctor(name.clone(), c.name.clone()))))
            .copied()
            .filter(|&id| fresh(id))
            .collect();
        let after_type = ty.is_none_or(|ty| !fresh(ty) || ctors.iter().all(|&c| c > ty));
        if !after_type || ctors.windows(2).any(|w| w[0] > w[1]) {
            out.push(format!(
                "{name}: its constructors are created again out of order"
            ));
        }
    }
    out
}

/// Returns each difference of `now`, the revert of the operator that took `was` to `mid`, from
/// `was`, the objects whose ids `was` holds and `base` does not being those a later migration
/// created.
fn differences(
    was: &Source,
    mid: &Source,
    now: &Source,
    sources: [&Source; 2],
) -> Vec<(Aspect, String)> {
    let mut out = Vec::new();
    let (dw, dn) = (decls(was), decls(now));
    for (m, t) in dw.iter().filter(|d| !dn.contains(d)) {
        out.push((Aspect::Decls, format!("-[{m}] {t}")));
    }
    for (m, t) in dn.iter().filter(|d| !dw.contains(d)) {
        out.push((Aspect::Decls, format!("+[{m}] {t}")));
    }
    if (&was.modules, &was.installed) != (&now.modules, &now.installed) {
        out.push((
            Aspect::Modules,
            format!(
                "{:?} {:?} -> {:?} {:?}",
                was.modules, was.installed, now.modules, now.installed
            ),
        ));
    }
    let (sw, sn) = (named_states(was), named_states(now));
    for (aspect, a, b) in [
        (Aspect::States, &sw.0, &sn.0),
        (Aspect::ValueStates, &sw.1, &sn.1),
    ] {
        let keys: BTreeSet<&StateNames> = a.keys().chain(b.keys()).collect();
        for k in keys {
            let (x, y) = (a.get(k).map(|s| s.stage), b.get(k).map(|s| s.stage));
            if x != y {
                out.push((aspect, format!("{k:?} {x:?} -> {y:?}")));
            }
        }
    }
    let (gw, gn) = (
        grants(was, &objects(was, "before")),
        grants(now, &objects(now, "after the revert")),
    );
    for g in gw.iter().filter(|g| !gn.contains(g)) {
        out.push((Aspect::Grants, format!("-{g:?}")));
    }
    for g in gn.iter().filter(|g| !gw.contains(g)) {
        out.push((Aspect::Grants, format!("+{g:?}")));
    }
    out.extend(id_changes(was, mid, now, sources));
    out
}

/// A disagreement the search finds today: the case it shows in, the operator kind it is
/// filed under, and each difference it shows with the bits of every shape it shows in.
struct Known {
    kind: &'static str,
    case: &'static str,
    differences: &'static [(&'static str, &'static [u64])],
}

/// The disagreements main shows.
const KNOWN: &[Known] = &[];

/// Returns each object an operator of `logged` resolves to, `ops` in order, that neither the
/// source before the operator nor the next source after it that builds holds, `next` being the
/// source after the migration.
fn outside(logged: &Logged, next: &Source, ops: &[ResolvedOp]) -> Vec<String> {
    let mut sources = logged.states().expect("a logged migration replays");
    sources.push(next.clone());
    let held = |s: &Source| -> BTreeSet<Reached> {
        s.ids
            .iter()
            .map(|(k, &id)| Reached::Object(k.kind().space(), id))
            .chain(s.modules.iter().cloned().map(Reached::Module))
            .collect()
    };
    let mut out = Vec::new();
    for (k, op) in ops.iter().enumerate() {
        let after = sources[k + 1..]
            .iter()
            .find(|s| s.schema().is_ok())
            .expect("the source after the migration builds");
        let around: BTreeSet<Reached> = held(&sources[k]).union(&held(after)).cloned().collect();
        for o in op.operands() {
            if !around.contains(&o) {
                let shown = logged.text()[logged.operators()[k].span.range()]
                    .lines()
                    .next()
                    .unwrap_or_default();
                out.push(format!(
                    "`{shown}` resolves to {o:?}, held by no source around it"
                ));
            }
        }
    }
    out
}

/// The messages of a refused migration.
#[derive(Debug, PartialEq)]
struct Refused(Vec<String>);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.join("\n"))
    }
}

/// What the search found of one case over every shape.
#[derive(Default)]
struct Outcome {
    /// The kinds of its operators.
    kinds: BTreeSet<&'static str>,
    /// Each difference, as its aspect and what differs, with the shapes it shows in.
    differences: BTreeMap<String, Vec<u64>>,
}

/// What the search over the first `count` shapes found.
#[derive(Default)]
struct Found {
    /// Each case by name.
    cases: BTreeMap<String, Outcome>,
    /// The kind of every invertible operator that applied, with the shape it applied in.
    reached: BTreeSet<(&'static str, u64)>,
    /// A case whose setup migration is refused, with the refusal.
    unapplied: Vec<String>,
    /// A case whose revert is refused for the reason the spec states: its shape, its name and
    /// the reason.
    stated: Vec<(u64, String, String)>,
    /// Why each migration the case being run applied does not resolve.
    unresolved: Vec<(Aspect, String)>,
    /// The cases the first pass ran, over every shape.
    ran: usize,
    /// The cases of the first pass whose revert is refused for a reason the spec states.
    first_stated: usize,
}

/// Returns whether shape `k` of the first `count` runs case `i` of its cases first: every case
/// when `every`, else case `i` of the cases every shape runs when `i % count == k`, and each case a
/// shape's bits add.
fn runs_first(k: usize, count: usize, every: bool, i: usize) -> bool {
    every || i >= CASES.0 || i % count == k
}

/// Returns the kinds of the operators of `case`.
fn case_kinds(case: &Case) -> BTreeSet<&'static str> {
    let text = case.text.replace("{first}", "0001");
    let m = parse_migration(&text, Options::default())
        .ok()
        .expect("the case parses");
    m.operators.iter().map(|o| kind_name(&o.kind)).collect()
}

/// Run in shape `k` of the first `count` the cases [`runs_first`] gives it. Then run again each
/// case every shape runs whose kinds that shape has not reached.
fn search(k: usize, count: usize, every: bool) -> Found {
    let bits = SHAPES[k];
    let shape = Shape::of(bits);
    let (start, cases) = (shape.source(), shape.cases());
    let mut found = Found::default();
    let mut ran = BTreeSet::new();
    for (i, case) in cases.iter().enumerate() {
        if runs_first(k, count, every, i) {
            ran.insert(i);
            found.ran += 1;
            found.case(bits, &start, case);
        }
    }
    found.first_stated = found.stated.len();
    for (i, case) in cases.iter().enumerate().take(CASES.0) {
        let unreached = case_kinds(case)
            .iter()
            .any(|k| !UNINVERTED.contains(k) && !found.reached.contains(&(*k, bits)));
        if unreached && !ran.contains(&i) {
            found.case(bits, &start, case);
        }
    }
    found
}

impl Found {
    fn case(&mut self, bits: u64, start: &Source, case: &Case) {
        let what = format!("{bits:03b} `{}`", case.name());
        self.unresolved.clear();
        let mut was = start.clone();
        for first in &case.first {
            match self.migrate(&was, first) {
                Ok(s) => was = s,
                Err(e) => {
                    self.unapplied
                        .push(format!("{what}: `{first}` refused: {e:?}"));
                    return;
                }
            }
        }
        let before = format!("{:04}", was.log.last().map_or(0, Logged::number));
        let text = case.text.replace("{first}", &before);
        let m = parse_migration(&text, Options::default())
            .ok()
            .expect("the case parses");
        let kinds: BTreeSet<&'static str> =
            m.operators.iter().map(|o| kind_name(&o.kind)).collect();
        let mid = match self.migrate(&was, &text) {
            Ok(s) => s,
            Err(e) => {
                let diffs = e.into_iter().map(|e| (Aspect::Unapplied, e)).collect();
                self.record(case, bits, kinds, diffs);
                return;
            }
        };
        self.reached.extend(
            kinds
                .iter()
                .filter(|k| !UNINVERTED.contains(k))
                .map(|&k| (k, bits)),
        );
        let number = mid.log.last().expect("the case is logged").number();
        let base = was.clone();
        let cased = mid.clone();
        let (mid, target) = match (&case.then, &case.expect) {
            (None, _) => (mid, was),
            (Some(then), expect) => {
                let applied = self.migrate(&mid, then).and_then(|m| {
                    let target = match expect {
                        Some(e) => migrate(&was, e)?,
                        None => was,
                    };
                    Ok((m, target))
                });
                match applied {
                    Ok(both) => both,
                    Err(e) => {
                        self.unapplied.push(format!(
                            "{what}: `{then}` or its expectation refused: {e:?}"
                        ));
                        return;
                    }
                }
            }
        };
        let fills: String = case.fills.iter().map(|f| format!("\n  fill {f}")).collect();
        let revert = format!("module auth\nrevert {number:04}{fills}");
        let reason = case.refused.map(|r| {
            r.replace("{case}", &format!("{number:04}"))
                .replace("{first}", &before)
                .replace("{then}", &format!("{:04}", number + 1))
        });
        let got = self.migrate(&mid, &revert);
        let diffs = match (got, reason) {
            (got, Some(reason)) => {
                let want = Refused(vec![reason.clone()]);
                adtb_testkit::refused_as(got.map_err(Refused), &want, &reason);
                self.stated.push((bits, case.name(), reason));
                Vec::new()
            }
            (Ok(now), None) => differences(&target, &mid, &now, [&base, &cased]),
            (Err(e), None) => e.into_iter().map(|e| (Aspect::Refused, e)).collect(),
        };
        self.record(case, bits, kinds, diffs);
    }

    /// Apply migration `text` to `src`, recording in `unresolved` why it does not resolve when
    /// it applies, each object an operator resolves to that neither the source before it nor
    /// the next that builds after it holds, and each name of its text outside its operators.
    fn migrate(&mut self, src: &Source, text: &str) -> Result<Source, Vec<String>> {
        let next = migrate(src, text)?;
        let logged = next.log.last().expect("a migration is logged");
        match logged.names() {
            Ok(names) => {
                let ops: Vec<Span> = logged.operators().iter().map(|o| o.span).collect();
                self.unresolved.extend(
                    names
                        .spans()
                        .filter(|s| !ops.iter().any(|o| o.start <= s.start && s.end <= o.end))
                        .map(|s| {
                            let at = format!("a name at {s:?} lies outside every operator");
                            (Aspect::Unresolved, at)
                        }),
                );
            }
            Err(Failed::Refused(ds)) => self
                .unresolved
                .extend(ds.into_iter().map(|d| (Aspect::Unresolved, d.message))),
            Err(Failed::Broken(d)) => panic!("{}", d.message),
        }
        match next.resolved(logged.number()) {
            Err(Failed::Refused(ds)) => self
                .unresolved
                .extend(ds.into_iter().map(|d| (Aspect::Unresolved, d.message))),
            Err(Failed::Broken(d)) => panic!("{}", d.message),
            Ok(ops) => self.unresolved.extend(
                outside(logged, &next, &ops)
                    .into_iter()
                    .map(|o| (Aspect::Unresolved, o)),
            ),
        }
        Ok(next)
    }

    fn record(
        &mut self,
        case: &Case,
        bits: u64,
        kinds: BTreeSet<&'static str>,
        diffs: Vec<(Aspect, String)>,
    ) {
        let entry = self.cases.entry(case.name()).or_default();
        entry.kinds.extend(kinds);
        for (aspect, text) in diffs.into_iter().chain(self.unresolved.drain(..)) {
            entry
                .differences
                .entry(format!("{aspect:?}: {text}"))
                .or_default()
                .push(bits);
        }
    }
}

/// Returns, for case `name` and what the search found of it in the shape of bits `bits`, why its
/// known row does not hold, when it does not: the row's differences in that shape are the case's.
fn unknown(name: &str, bits: u64, o: &Outcome) -> Option<String> {
    let got: BTreeSet<&str> = o.differences.keys().map(String::as_str).collect();
    let known = KNOWN.iter().find(|k| k.case == name);
    if let Some(k) = known
        && !o.kinds.contains(k.kind)
    {
        return Some(format!(
            "`{name}` is known under {}, not one of its kinds {:?}",
            k.kind, o.kinds
        ));
    }
    let want: BTreeSet<&str> = known
        .into_iter()
        .flat_map(|k| k.differences)
        .filter(|(_, shapes)| shapes.contains(&bits))
        .map(|&(d, _)| d)
        .collect();
    (got != want).then(|| {
        format!(
            "`{name}` in {bits:03b} differs otherwise than its known row:\n  got {got:#?}\n  known {want:#?}"
        )
    })
}

/// The cases each shape runs with no bit set, and those the `declared` and `granted` bits add.
const CASES: (usize, usize, usize) = (166, 2, 5);

/// The cases of each shape whose revert is refused for a reason the spec states.
const REFUSED: usize = 18;

/// The number of shapes the search runs in: every shape under the `deep` profile, else the first
/// two, which set each bit both ways.
fn shapes_searched() -> usize {
    adtb_testkit::profile::count(2).min(SHAPES.len() as u64) as usize
}

/// Run the search in shape `k`: unless the profile is `deep`, each case every shape runs runs in
/// one of the first two shapes, and again in the other when that shape has not reached one of
/// its kinds; `deep` runs every case in every shape. Every invertible kind applies in the shape,
/// and each case's revert restores the source before it or is refused for the reason it states.
fn search_shape(k: usize) {
    let count = shapes_searched();
    if k >= count {
        return;
    }
    let every = adtb_testkit::profile::every_case();
    let found = search(k, count, every);
    let mut failures = found.unapplied.clone();
    for (name, o) in &found.cases {
        failures.extend(unknown(name, SHAPES[k], o));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    let bits = SHAPES[k];
    let cases = Shape::of(bits).cases();
    let first: Vec<&Case> = cases
        .iter()
        .enumerate()
        .filter(|&(i, _)| runs_first(k, count, every, i))
        .map(|(_, c)| c)
        .collect();
    assert_eq!(first.len(), found.ran);
    assert_eq!(
        first.iter().filter(|c| c.refused.is_some()).count(),
        found.first_stated,
        "{:?}",
        found.stated
    );
    let reachable: BTreeSet<(&str, u64)> = invertible().into_iter().map(|k| (k, bits)).collect();
    assert_eq!(
        reachable, found.reached,
        "the search applies every invertible kind in the shape"
    );
}

/// A shard searches each shape, and the shards together run each case every shape runs in one of
/// the shapes searched, every one of them under `deep`, and each case a shape's bits add in that
/// shape; the known rows and the stated refusals are cases of the search.
#[test]
fn the_search_shards_run_every_case() {
    assert_eq!(
        (0..SHAPES.len()).collect::<Vec<_>>(),
        SHARDS,
        "a test searches each shape"
    );
    let count = shapes_searched();
    let every = adtb_testkit::profile::every_case();
    let mut shared: BTreeMap<usize, usize> = BTreeMap::new();
    let mut names = BTreeSet::new();
    let mut refused = 0;
    for (k, bits) in SHAPES.into_iter().enumerate().take(count) {
        let shape = Shape::of(bits);
        let cases = shape.cases();
        let added = usize::from(shape.declared) * CASES.1 + usize::from(shape.granted) * CASES.2;
        assert_eq!(CASES.0 + added, cases.len(), "{bits:03b}");
        for (i, case) in cases.iter().enumerate() {
            names.insert(case.name());
            if i < CASES.0 && k == 0 && case.refused.is_some() {
                refused += 1;
            }
            match (i < CASES.0, runs_first(k, count, every, i)) {
                (true, true) => *shared.entry(i).or_default() += 1,
                (false, false) => panic!("{bits:03b} skips its own case {i}"),
                _ => {}
            }
        }
    }
    let runs_of_each = if every { count } else { 1 };
    assert_eq!(
        (0..CASES.0)
            .map(|i| (i, runs_of_each))
            .collect::<BTreeMap<_, _>>(),
        shared
    );
    assert_eq!(REFUSED, refused);
    for k in KNOWN {
        assert!(
            names.contains(k.case),
            "known `{}` is no case of the search",
            k.case
        );
    }
}

/// Declare a test searching each shape `k` named, and `SHARDS`, the shapes they search.
macro_rules! shards {
    ($($name:ident = $k:literal),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                search_shape($k);
            }
        )*
        const SHARDS: &[usize] = &[$($k),*];
    };
}

shards! {
    every_invertible_operator_and_its_revert_restore_the_source_before_it_in_shape_0 = 0,
    every_invertible_operator_and_its_revert_restore_the_source_before_it_in_shape_1 = 1,
    every_invertible_operator_and_its_revert_restore_the_source_before_it_in_shape_2 = 2,
    every_invertible_operator_and_its_revert_restore_the_source_before_it_in_shape_3 = 3,
    every_invertible_operator_and_its_revert_restore_the_source_before_it_in_shape_4 = 4,
    every_invertible_operator_and_its_revert_restore_the_source_before_it_in_shape_5 = 5,
    every_invertible_operator_and_its_revert_restore_the_source_before_it_in_shape_6 = 6,
    every_invertible_operator_and_its_revert_restore_the_source_before_it_in_shape_7 = 7,
}

/// Returns each script under the engine's script corpus, as its path and text.
fn scripts() -> Vec<(std::path::PathBuf, String)> {
    let mut out = Vec::new();
    let mut dirs = vec![std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../adtb-sim/scripts"
    ))];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).expect("the script directory reads") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "test") {
                let text = std::fs::read_to_string(&path).expect("a script reads");
                out.push((path, text));
            }
        }
    }
    out.sort();
    out
}

/// Returns the migrations of the `migrate` directives of `script` that apply to the version
/// before them, in order, each starting at its `module` line.
fn script_migrations(script: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut inside = false;
    for line in script.lines() {
        if line == "migrate" || (line.starts_with("migrate ") && !line.starts_with("migrate at ")) {
            inside = true;
            continue;
        }
        match line.strip_prefix("  ") {
            Some(body) if inside => {
                if body.starts_with("module ") {
                    out.push(String::new());
                }
                if let Some(m) = out.last_mut() {
                    m.push_str(body);
                    m.push('\n');
                }
            }
            _ => inside = false,
        }
    }
    out
}

/// How many threads the law search over the cases and the script corpus runs on.
const SEARCHING: usize = 4;

/// What the laws over one migration saw.
#[derive(Default)]
struct Seen {
    applied: usize,
    refused: usize,
    revert_steps: usize,
}

/// Returns `text` as its tokens joined by single spaces.
fn token_form(text: &str) -> String {
    tokens(text)
        .into_iter()
        .map(|(_, r)| &text[r])
        .collect::<Vec<_>>()
        .join(" ")
}

/// Apply migration `text` to `src` and check the laws a migration keeps, counting what it saw in
/// `seen`. Returns the source it leaves, when it applies.
///
/// The laws:
/// - Ids are fresh: `next` of each space never falls, and an id the result holds that `src` did
///   not is drawn from `src`'s `next` of its space and is below the result's.
/// - Each step of an applied `revert` states its row maps and resolves against the step before
///   it, and its last step leaves the source the revert leaves.
/// - Each recheck failure in text an operator copied points to text whose tokens are those of
///   the span of the migration it names.
/// - Each refusal of an operator lies within that operator's span.
fn lawful(src: &Source, text: &str, seen: &mut Seen) -> Option<Source> {
    let m = parse_migration(text, Options::default()).ok().ok()?;
    match src.record(&m, text, false) {
        Ok((next, _, recorded)) => {
            seen.applied += 1;
            for (space, &n) in &src.next {
                assert!(
                    next.next.get(space).is_some_and(|&m| m >= n),
                    "{text}\nnext of {space:?} falls"
                );
            }
            let held: BTreeSet<(Space, u64)> = src
                .ids
                .iter()
                .map(|(k, &id)| (k.kind().space(), id))
                .collect();
            for (k, &id) in &next.ids {
                let space = k.kind().space();
                if id < crate::ids::FIRST_LOG_ID || held.contains(&(space, id)) {
                    continue;
                }
                let from = src
                    .next
                    .get(&space)
                    .copied()
                    .unwrap_or(crate::ids::FIRST_LOG_ID);
                let to = next
                    .next
                    .get(&space)
                    .copied()
                    .unwrap_or(crate::ids::FIRST_LOG_ID);
                assert!(
                    from <= id && id < to,
                    "{text}\n{k:?} takes id {id} outside {from}..{to}"
                );
            }
            for (op, steps) in m.operators.iter().zip(recorded.steps()) {
                if !matches!(op.kind, OperatorKind::Revert { .. }) {
                    continue;
                }
                let steps = steps
                    .as_ref()
                    .unwrap_or_else(|ds| panic!("{text}\nan applied revert's steps fail: {ds:?}"));
                seen.revert_steps += steps.len();
                if m.operators.len() == 1 {
                    let last = &steps.last().expect("a revert takes a step").after;
                    assert_eq!(
                        (
                            &next.ids,
                            next.decls
                                .iter()
                                .map(|d| token_form(&d.text))
                                .collect::<Vec<_>>()
                        ),
                        (
                            &last.ids,
                            last.decls
                                .iter()
                                .map(|d| token_form(&d.text))
                                .collect::<Vec<_>>()
                        ),
                        "{text}\nthe last step leaves the revert's result"
                    );
                }
            }
            Some(next)
        }
        Err(Failed::Broken(d)) => panic!("{text}\nthe model fails: {}", d.message),
        Err(Failed::Refused(r)) => {
            seen.refused += 1;
            for d in &r.operators {
                assert!(
                    m.operators
                        .iter()
                        .any(|op| op.span.start <= d.span.start && d.span.end <= op.span.end),
                    "{text}\nrefusal outside every operator: {d:?}"
                );
            }
            None
        }
    }
}

/// Every migration of the unlabelled cases of the search in its first shape, and every migration
/// of the engine's script corpus, each script's applied in order from an empty source, keeps the
/// laws [`lawful`] states.
#[test]
fn the_migrations_of_the_cases_and_the_script_corpus_keep_their_laws() {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let shapes: Vec<(Source, Vec<Case>)> = SHAPES
        .into_iter()
        .take(1)
        .map(|bits| (Shape::of(bits).source(), Shape::of(bits).cases()))
        .collect();
    let mut cases: Vec<(&Source, &Case)> = shapes
        .iter()
        .flat_map(|(start, cases)| cases.iter().map(move |c| (start, c)))
        .collect();
    let mut seen_cases = BTreeSet::new();
    cases.retain(|(_, c)| c.label.is_none() && seen_cases.insert((&c.first, &c.text)));
    let mut scripts: Vec<Vec<String>> = scripts()
        .into_iter()
        .map(|(path, script)| {
            let migrations = script_migrations(&script);
            assert!(
                !script.contains("\nmigrate\n") || !migrations.is_empty(),
                "{path:?} reads as no migration"
            );
            migrations
        })
        .collect();
    scripts.sort_by_key(|s| std::cmp::Reverse(s.len()));
    let applied: Mutex<BTreeMap<String, Option<Source>>> = Mutex::default();
    let next_job = AtomicUsize::new(0);
    let jobs = cases.len() + scripts.len();
    let work = || {
        let mut seen = Seen::default();
        loop {
            let job = next_job.fetch_add(1, Ordering::Relaxed);
            if job >= jobs {
                return seen;
            }
            if let Some(&(start, case)) = job.checked_sub(scripts.len()).and_then(|k| cases.get(k))
            {
                let mut was = start.clone();
                for first in &case.first {
                    was = lawful(&was, first, &mut seen).expect("a case's first migration applies");
                }
                let before = format!("{:04}", was.log.last().map_or(0, Logged::number));
                lawful(&was, &case.text.replace("{first}", &before), &mut seen);
                continue;
            }
            let mut src = Source::new(Vec::new()).expect("an empty source builds");
            let mut prefix = String::new();
            for text in &scripts[job] {
                prefix.push_str(text);
                prefix.push('\0');
                let held = applied
                    .lock()
                    .expect("the applied prefixes")
                    .get(&prefix)
                    .cloned();
                src = match held {
                    Some(next) => next.unwrap_or(src),
                    None => {
                        let next = lawful(&src, text, &mut seen);
                        applied
                            .lock()
                            .expect("the applied prefixes")
                            .insert(prefix.clone(), next.clone());
                        next.unwrap_or(src)
                    }
                };
            }
        }
    };
    let mut seen = Seen::default();
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..SEARCHING).map(|_| scope.spawn(work)).collect();
        for w in workers {
            let s = w.join().expect("a searching thread");
            seen.applied += s.applied;
            seen.refused += s.refused;
            seen.revert_steps += s.revert_steps;
        }
    });
    assert!(
        seen.applied > 0 && seen.refused > 0 && seen.revert_steps > 0,
        "the search reaches each law: {} applied, {} refused, {} revert steps",
        seen.applied,
        seen.refused,
        seen.revert_steps,
    );
}

/// Returns each operator that is refused alone on `start`, in module `auth`, with no recheck
/// failure, with the messages of its refusal and whether a refusal of it poisons the name it
/// adds, as an added type, function or relation variable's does: each operator of the unlabelled
/// cases of `shape` that need no migration before them, a merge stating none of the grants it
/// cannot carry, and added declarations with several failures.
fn failing_alone(start: &Source, shape: Shape) -> Vec<(String, Vec<String>, bool)> {
    let merge = shape.merge();
    let merge = merge.trim_end_matches("\ngrant update ts.M to support");
    let mut texts: Vec<String> = vec![
        merge.to_string(),
        "add type Bad = { a: Nope, b: Nada }".to_string(),
        "add type Worse = W1 { a: Nope } | W2 { b: Nada, c: Gone }".to_string(),
    ];
    for case in shape.cases() {
        if case.label.is_some() || !case.first.is_empty() || !case.text.starts_with("module auth\n")
        {
            continue;
        }
        let m = parse_migration(&case.text, Options::default())
            .ok()
            .expect("the case parses");
        texts.extend(
            m.operators
                .iter()
                .map(|op| case.text[op.span.range()].to_string())
                .filter(|t| !t.contains("{first}")),
        );
    }
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for op in texts {
        if !seen.insert(op.clone()) {
            continue;
        }
        let text = format!("module auth\n{op}\n");
        let Ok(m) = parse_migration(&text, Options::default()).ok() else {
            continue;
        };
        match start.migrate(&m, &text) {
            Err(Failed::Broken(d)) => panic!("{text}\nthe model fails: {}", d.message),
            Err(Failed::Refused(r)) if r.recheck.is_empty() => {
                assert!(!r.operators.is_empty(), "{text}\nrefused listing nothing");
                let kind = &m.operators[0].kind;
                let poisons = expected::poisons(kind).is_some()
                    || matches!(kind, OperatorKind::AddRelation(_));
                let why = r.operators.into_iter().map(|d| d.message).collect();
                out.push((op, why, poisons));
            }
            Ok(_) | Err(Failed::Refused(_)) => {}
        }
    }
    out
}

/// How many migrations of operators each refused alone the refusal list law checks.
const COMBINED: usize = 300;

/// A migration of 2 to 4 operators, each refused alone on the source of the first shape with
/// `ts.A`'s default dropped and its delete granted, is
/// refused listing the refusals of each, in order, each within its operator's span: every
/// operator that does not apply, every grant a merge cannot carry and every failure of an added
/// declaration's own text. At most one added declaration whose refusal poisons its name is
/// among them, at any position, and no operator naming a constructor a merge before it replaces.
#[test]
fn a_migration_of_operators_each_refused_alone_lists_every_refusal_in_order() {
    let shape = Shape::of(SHAPES[0]);
    let start = migrate(
        &shape.source(),
        "module auth\ndrop default ts.A.x\ngrant delete ts.A to support",
    )
    .expect("the start applies");
    let pool = failing_alone(&start, shape);
    assert!(
        pool.iter()
            .any(|(op, why, _)| op.starts_with("merge ") && why.len() > 1),
        "the pool holds a merge refused for several grants: {pool:?}"
    );
    assert!(
        pool.iter()
            .any(|(op, why, _)| op.starts_with("add type") && why.len() > 1),
        "the pool holds an added declaration refused several times: {pool:?}"
    );
    assert!(pool.len() >= 8, "the pool holds few operators: {pool:?}");
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut draw = |n: usize| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as usize % n
    };
    let failure = |p: usize| match &pool[p] {
        (op, why, _) if op.starts_with("merge ") && why.len() > 1 => "a merge's grants",
        (_, why, _) if why.len() > 1 => "a declaration's several failures",
        _ => "one failure",
    };
    let mut failures = BTreeSet::new();
    let mut poisoned_at = BTreeSet::new();
    let mut ran = 0;
    for i in 0..COMBINED {
        let k = 2 + i % 3;
        let mut picked: Vec<usize> = Vec::new();
        while picked.len() < k {
            let j = draw(pool.len());
            let poisoning = pool[j].2 && picked.iter().any(|&p| pool[p].2);
            let merged = ["ts.A", "ts.B"].iter().any(|c| pool[j].0.contains(c))
                && picked.iter().any(|&p| pool[p].0.starts_with("merge "));
            if !picked.contains(&j) && !poisoning && !merged {
                picked.push(j);
            }
        }
        failures.extend(picked.iter().map(|&p| failure(p)));
        if let Some(at) = picked.iter().position(|&p| pool[p].2) {
            poisoned_at.insert(match at {
                0 => "first",
                _ if at == k - 1 => "last",
                _ => "between",
            });
        }
        let mut text = String::from("module auth\n");
        let mut spans = Vec::new();
        let mut want = Vec::new();
        for &p in &picked {
            let start = text.len();
            text.push_str(&pool[p].0);
            spans.push(start..text.len());
            text.push('\n');
            want.extend(pool[p].1.iter().cloned().map(|w| (spans.len() - 1, w)));
        }
        let m = parse_migration(&text, Options::default())
            .ok()
            .expect("the combined migration parses");
        let r = match start.migrate(&m, &text) {
            Ok(_) => panic!("{text}\napplies"),
            Err(Failed::Broken(d)) => panic!("{text}\nthe model fails: {}", d.message),
            Err(Failed::Refused(r)) => r,
        };
        assert_eq!(
            want.iter().map(|(at, _)| *at).collect::<Vec<_>>(),
            r.blame(),
            "{text}\nthe operator each refusal names"
        );
        let got: Vec<(usize, String)> = r
            .operators
            .into_iter()
            .map(|d| {
                let at = spans
                    .iter()
                    .position(|s| s.contains(&(d.span.start as usize)))
                    .unwrap_or_else(|| panic!("{text}\nrefusal outside every operator: {d:?}"));
                (at, d.message)
            })
            .collect();
        assert_eq!(want, got, "{text}");
        assert!(r.recheck.is_empty(), "{text}\nrechecked: {:?}", r.recheck);
        ran += 1;
    }
    assert_eq!(COMBINED, ran);
    assert_eq!(
        BTreeSet::from([
            "a declaration's several failures",
            "a merge's grants",
            "one failure"
        ]),
        failures,
        "the draws reach each kind of failure"
    );
    assert_eq!(
        BTreeSet::from(["between", "first", "last"]),
        poisoned_at,
        "the draws place a poisoning declaration at each position"
    );
}
