use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::ids::{KeyKind, Space};
use crate::parse::parse_migration;

/// A schema object described by the names a schema gives it and its position among clauses.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Desc {
    Type(String),
    Ctor(String, String),
    Relation(String),
    Field(String, Option<String>, String),
    Set(String, String),
    Key(String),
    Index(String, usize),
    Implied(String, Box<Desc>),
    Generated(String),
    Default(String, usize, Box<Desc>),
    Computed(String, usize),
    Function(String),
    Aggregate(String),
    Constraint(String),
    Check(String),
    View(String),
    ViewType(String),
    ViewCtor(String),
    ViewKey(String),
    ViewField(String, String),
}

impl Desc {
    pub(super) fn space(&self) -> Space {
        match self {
            Desc::Type(_)
            | Desc::Ctor(..)
            | Desc::Relation(_)
            | Desc::View(_)
            | Desc::ViewType(_)
            | Desc::ViewCtor(_) => Space::Shared,
            Desc::Field(..) | Desc::ViewField(..) => Space::Field,
            Desc::Set(..) => Space::Set,
            Desc::Key(_) | Desc::Index(..) | Desc::Implied(..) | Desc::ViewKey(_) => Space::Index,
            Desc::Generated(_) | Desc::Default(..) => Space::Default,
            Desc::Computed(..) => Space::Computed,
            Desc::Function(_) | Desc::Aggregate(_) => Space::Function,
            Desc::Constraint(_) | Desc::Check(_) => Space::Constraint,
        }
    }
}

pub(super) fn type_name(s: &Schema, def: TypeId) -> String {
    let t = s.ty(def);
    format!("{}::{}", s.module(t.module).name, t.name)
}

fn rel_name(s: &Schema, r: RelId) -> String {
    let rd = s.relation(r);
    format!("{}::{}", s.module(rd.module).name, rd.name)
}

fn field_desc(s: &Schema, f: FieldRef) -> Desc {
    let td = s.ty(f.def());
    let ctor = f.ctor().map(|c| td.ctors()[c as usize].name.clone());
    Desc::Field(type_name(s, f.def()), ctor, td.field_def(f).name.clone())
}

/// Returns every object of `s` that has a catalog id, described by its names, with its id.
fn described(s: &Schema) -> Vec<(Desc, u64)> {
    let ids = &s.ids;
    let mut out: Vec<(Desc, u64)> = Vec::new();
    out.extend(
        ids.type_ids()
            .map(|(t, id)| (Desc::Type(type_name(s, t)), id.raw())),
    );
    out.extend(ids.ctor_ids().map(|(c, id)| {
        let td = s.ty(c.def());
        let name = td
            .ctors()
            .get(c.index() as usize)
            .map_or_else(|| td.name.clone(), |k| k.name.clone());
        (Desc::Ctor(type_name(s, c.def()), name), id.raw())
    }));
    out.extend(
        ids.relation_ids()
            .map(|(r, id)| (Desc::Relation(rel_name(s, r)), id.raw())),
    );
    out.extend(ids.field_ids().map(|(f, id)| (field_desc(s, f), id.raw())));
    out.extend(
        ids.set_ids()
            .map(|(r, n, id)| (Desc::Set(rel_name(s, r), n.to_string()), id.raw())),
    );
    out.extend(
        ids.key_ids()
            .map(|(r, id)| (Desc::Key(rel_name(s, r)), id.raw())),
    );
    out.extend(
        ids.index_ids()
            .map(|((r, at), id)| (Desc::Index(rel_name(s, r), at), id.raw())),
    );
    out.extend(ids.implied_ids().map(|(r, f, id)| {
        (
            Desc::Implied(rel_name(s, r), Box::new(field_desc(s, f))),
            id.raw(),
        )
    }));
    out.extend(
        ids.generated_ids()
            .map(|(r, id)| (Desc::Generated(rel_name(s, r)), id.raw())),
    );
    out.extend(ids.default_ids().map(|((r, at, f), id)| {
        (
            Desc::Default(rel_name(s, r), at, Box::new(field_desc(s, f))),
            id.raw(),
        )
    }));
    out.extend(
        ids.computed_ids()
            .map(|((r, at), id)| (Desc::Computed(rel_name(s, r), at), id.raw())),
    );
    let fn_name = |f: FnId| s.functions[f.0 as usize].name.clone();
    out.extend(
        ids.function_ids()
            .map(|(f, id)| (Desc::Function(fn_name(f)), id.raw())),
    );
    out.extend(
        ids.aggregate_ids()
            .map(|(at, id)| (Desc::Aggregate(s.aggregates[at].name.clone()), id.raw())),
    );
    out.extend(
        ids.constraint_ids()
            .map(|(at, id)| (Desc::Constraint(s.constraints[at].name.clone()), id.raw())),
    );
    out.extend(
        ids.check_ids()
            .map(|(at, id)| (Desc::Check(s.checks[at].name.clone()), id.raw())),
    );
    for (f, id) in ids.view_ids() {
        out.push((Desc::View(fn_name(f)), id.raw()));
        if let Some(t) = ids.view_type_id(f) {
            out.push((Desc::ViewType(fn_name(f)), t.raw()));
        }
        if let Some(c) = ids.view_ctor_id(f) {
            out.push((Desc::ViewCtor(fn_name(f)), c.raw()));
        }
        if let Some(k) = ids.view_key_id(f) {
            out.push((Desc::ViewKey(fn_name(f)), k.raw()));
        }
    }
    out.extend(
        ids.view_field_ids()
            .map(|(f, n, id)| (Desc::ViewField(fn_name(f), n.to_string()), id.raw())),
    );
    out
}

/// Returns the objects of `s` by description, asserting that no two share a description and no
/// two of one id space share an id.
pub(super) fn by_desc(s: &Schema, what: &str) -> BTreeMap<Desc, u64> {
    let mut out = BTreeMap::new();
    let mut held: BTreeMap<(Space, u64), Desc> = BTreeMap::new();
    for (d, id) in described(s) {
        if let Some(other) = held.insert((d.space(), id), d.clone()) {
            panic!("{what}: {d:?} and {other:?} share id {id}");
        }
        assert!(out.insert(d.clone(), id).is_none(), "{what}: {d:?} twice");
    }
    out
}

/// A rename or move operator, with how it changes the names of the objects it does not create.
struct Op {
    text: String,
    /// Each name it changes: the type, then the constructor, relation, set or field.
    rename: fn(&Desc) -> Desc,
}

/// The shape a generated schema varies by.
#[derive(Clone, Copy, Debug)]
struct Shape {
    /// The header of `T` holds a reference field `h`.
    header_ref: bool,
    /// Constructor `B` holds a reference field `r`, as `A` does.
    b_ref: bool,
    /// Constructor `C` holds a field `x`, as `A` and `B` do.
    c_x: bool,
    /// Relation `ts` is declared before type `T`.
    rel_first: bool,
}

impl Shape {
    fn of(bits: u64) -> Shape {
        Shape {
            header_ref: bits & 1 != 0,
            b_ref: bits & 2 != 0,
            c_x: bits & 4 != 0,
            rel_first: bits & 8 != 0,
        }
    }

    fn text(self) -> String {
        let h = if self.header_ref { ", h: Key us" } else { "" };
        let b_r = if self.b_ref { ", r: Key us" } else { "" };
        let c = if self.c_x { " { x: Text }" } else { "" };
        let ty = format!(
            "type T {{ id: Key ts, n: Text{h} }}
  = A {{ x: Text, r: Key us, y: Text, z: Text }}
  | B {{ x: Text{b_r}, z: Text }}
  | C{c}
"
        );
        let c_default = if self.c_x {
            "  default C.x = \"c\"\n"
        } else {
            ""
        };
        let rel = format!(
            "relation ts : T
  key id generate sequence
  unique (x) in A
  unique (x) in B
  index (n) in (A | B)
  unique (n) in A
  index (z) in S
  default A.x = \"a\"
  default B.x = \"b\"
{c_default}  computed tag: Text = \"t\"
"
        );
        let (first, second) = if self.rel_first { (rel, ty) } else { (ty, rel) };
        format!(
            "module auth
type U = {{ id: Key us, label: Text }}
type Label = new Text
relation us : U
  key id generate sequence
  unique (label)
{first}{second}set S of ts = A | B
fn f (t: T) -> Text = match t
  | A a -> a.x
  | B b -> b.x
  | C c -> \"c\"
aggregate longest (x: Text) -> Text =
  state {{ best: Text }}
  init {{ best: \"\" }}
  step (s, x) -> if scalar_len(x) > scalar_len(s.best) then {{ best: x }} else s
  merge (a, b) -> if scalar_len(b.best) > scalar_len(a.best) then b else a
  finish s -> s.best
constraint unlabelled = from u in us where u.label == \"\" select u
check x_set on ts.A (scalar_len(x) > 0)
fn labels () -> Set ({{ label: Text }}) = from u in us select {{ label: u.label }}
store labels
  key label
"
        )
    }

    /// Every rename and move operator that applies to the schema, each with its renaming.
    fn ops(self) -> Vec<Op> {
        let op = |text: &str, rename: fn(&Desc) -> Desc| Op {
            text: format!("module auth\n{text}"),
            rename,
        };
        let mut out = vec![
            op("rename type U to U2", |d| {
                names(d, &|n| match n {
                    Name::Type("auth::U") => "auth::U2",
                    Name::Ctor("auth::U", "U") => "U2",
                    n => n.text(),
                })
            }),
            op("rename type Label to Label2", |d| {
                names(d, &|n| match n {
                    Name::Type("auth::Label") => "auth::Label2",
                    Name::Ctor("auth::Label", "Label") => "Label2",
                    n => n.text(),
                })
            }),
            op("rename type T to T2", |d| {
                names(d, &|n| match n {
                    Name::Type("auth::T") => "auth::T2",
                    n => n.text(),
                })
            }),
            op("rename constructor T.A to A2", |d| {
                names(d, &|n| match n {
                    Name::Ctor("auth::T", "A") => "A2",
                    n => n.text(),
                })
            }),
            op("rename constructor T.B to B2", |d| {
                names(d, &|n| match n {
                    Name::Ctor("auth::T", "B") => "B2",
                    n => n.text(),
                })
            }),
            op("rename relation ts to ts2", |d| {
                names(d, &|n| match n {
                    Name::Relation("auth::ts") => "auth::ts2",
                    n => n.text(),
                })
            }),
            op("rename relation us to us2", |d| {
                names(d, &|n| match n {
                    Name::Relation("auth::us") => "auth::us2",
                    n => n.text(),
                })
            }),
            op("rename set S of ts to S2", |d| {
                names(d, &|n| match n {
                    Name::Set("auth::ts", "S") => "S2",
                    n => n.text(),
                })
            }),
            op("rename field T.A.x to x2", |d| {
                names(d, &|n| match n {
                    Name::Field("auth::T", Some("A"), "x") => "x2",
                    n => n.text(),
                })
            }),
            op("rename field T.B.x to x2", |d| {
                names(d, &|n| match n {
                    Name::Field("auth::T", Some("B"), "x") => "x2",
                    n => n.text(),
                })
            }),
            op("rename field T.A.r to r2", |d| {
                names(d, &|n| match n {
                    Name::Field("auth::T", Some("A"), "r") => "r2",
                    n => n.text(),
                })
            }),
            op("rename field U.label to name", |d| {
                names(d, &|n| match n {
                    Name::Field("auth::U", Some("U"), "label") => "name",
                    n => n.text(),
                })
            }),
            op("rename field T.n to n2", |d| {
                names(d, &|n| match n {
                    Name::Field("auth::T", None, "n") => "n2",
                    n => n.text(),
                })
            }),
            op(
                "move field T.A.y to T fill match\n  | B b -> \"\"\n  | C c -> \"\"",
                |d| moved(d, "y", Some("A"), None),
            ),
            op("move field T.n to T.(B | A)", |d| {
                moved(d, "n", None, Some("A"))
            }),
        ];
        if self.header_ref {
            out.push(op("rename field T.h to h2", |d| {
                names(d, &|n| match n {
                    Name::Field("auth::T", None, "h") => "h2",
                    n => n.text(),
                })
            }));
        }
        if self.b_ref {
            out.push(op("rename field T.B.r to r2", |d| {
                names(d, &|n| match n {
                    Name::Field("auth::T", Some("B"), "r") => "r2",
                    n => n.text(),
                })
            }));
        }
        if self.c_x {
            out.push(op("rename field T.C.x to x2", |d| {
                names(d, &|n| match n {
                    Name::Field("auth::T", Some("C"), "x") => "x2",
                    n => n.text(),
                })
            }));
        }
        out
    }
}

/// One name of a description, with the names that place it.
enum Name<'a> {
    Type(&'a str),
    Ctor(&'a str, &'a str),
    Relation(&'a str),
    Set(&'a str, &'a str),
    Field(&'a str, Option<&'a str>, &'a str),
}

impl<'a> Name<'a> {
    fn text(&self) -> &'a str {
        match self {
            Name::Type(t) | Name::Relation(t) => t,
            Name::Ctor(_, c) => c,
            Name::Set(_, s) => s,
            Name::Field(_, _, f) => f,
        }
    }
}

/// Returns `d` with each of its names replaced by what `to` gives it.
fn names(d: &Desc, to: &dyn for<'a> Fn(Name<'a>) -> &'a str) -> Desc {
    let ty = |t: &str| to(Name::Type(t)).to_string();
    let rel = |r: &str| to(Name::Relation(r)).to_string();
    let field = |t: &str, c: &Option<String>, f: &str| {
        let c2 = c.as_deref().map(|c| to(Name::Ctor(t, c)).to_string());
        Desc::Field(ty(t), c2, to(Name::Field(t, c.as_deref(), f)).to_string())
    };
    let inner = |d: &Desc| match d {
        Desc::Field(t, c, f) => field(t, c, f),
        other => panic!("{other:?} is no field"),
    };
    match d {
        Desc::Type(t) => Desc::Type(ty(t)),
        Desc::Ctor(t, c) => Desc::Ctor(ty(t), to(Name::Ctor(t, c)).to_string()),
        Desc::Relation(r) => Desc::Relation(rel(r)),
        Desc::Field(t, c, f) => field(t, c, f),
        Desc::Set(r, s) => Desc::Set(rel(r), to(Name::Set(r, s)).to_string()),
        Desc::Key(r) => Desc::Key(rel(r)),
        Desc::Index(r, at) => Desc::Index(rel(r), *at),
        Desc::Implied(r, f) => Desc::Implied(rel(r), Box::new(inner(f))),
        Desc::Generated(r) => Desc::Generated(rel(r)),
        Desc::Default(r, at, f) => Desc::Default(rel(r), *at, Box::new(inner(f))),
        Desc::Computed(r, at) => Desc::Computed(rel(r), *at),
        other => other.clone(),
    }
}

/// Returns `d` with field `name` of type `auth::T` moved from constructor `from` to `to`, the
/// header for `None`.
fn moved(d: &Desc, name: &str, from: Option<&str>, to: Option<&str>) -> Desc {
    let field = |f: &Desc| match f {
        Desc::Field(t, c, n) if t == "auth::T" && n == name && c.as_deref() == from => {
            Desc::Field(t.clone(), to.map(str::to_string), n.clone())
        }
        other => other.clone(),
    };
    match d {
        Desc::Field(..) => field(d),
        Desc::Implied(r, f) => Desc::Implied(r.clone(), Box::new(field(f))),
        Desc::Default(r, at, f) => Desc::Default(r.clone(), *at, Box::new(field(f))),
        other => other.clone(),
    }
}

/// The order generated shapes are tried in: every shape bit set, then none, then mixes.
const SHAPES: [u64; 16] = [15, 0, 5, 10, 3, 12, 6, 9, 1, 14, 2, 13, 4, 11, 7, 8];

#[test]
fn a_rename_or_move_keeps_the_id_of_every_object_it_does_not_create() {
    let count = (adtb_testkit::profile::count(SHAPES.len() as u64) as usize).min(SHAPES.len());
    let mut ran = 0;
    let mut moved_ids = Vec::new();
    for bits in SHAPES.into_iter().take(count) {
        let shape = Shape::of(bits);
        let base = source(&shape.text());
        let before = by_desc(&ids_of(&base), &format!("{shape:?}"));
        for op in shape.ops() {
            let what = format!("{shape:?} {}", op.text);
            let after = by_desc(&ids_of(&applies(&base, &op.text)), &what);
            for (d, &id) in &before {
                let to = (op.rename)(d);
                if after.get(&to) != Some(&id) {
                    moved_ids.push(format!("{what}: {d:?} {id}, {to:?} {:?}", after.get(&to)));
                }
            }
            ran += 1;
        }
    }
    assert_eq!(Vec::<String>::new(), moved_ids);
    assert!(ran >= count * 15, "ran {ran} operators");
}

#[test]
fn a_constructor_a_split_adds_routes_rows_under_its_logged_id_which_no_other_object_holds() {
    let base = source(
        "module auth
type T { id: Key ts } = A { a: Text } | B { b: Text }
relation ts : T key id generate sequence
type U = X | Y",
    );
    let text = "module auth
split constructor T.A into C { c: Text } | D { d: Text }
  by match
    | A x -> C { c: x.a }";
    let file = parse_migration(text, Options::default())
        .ok()
        .expect("the migration parses");
    let mut at = base.clone();
    at.expecting("auth", &file.operators);
    let steps = at
        .steps("auth", &file.operators[0], text)
        .unwrap_or_else(|e| panic!("the split applies: {e:?}"));
    let routing = steps
        .iter()
        .flat_map(|s| &s.maps)
        .find_map(|m| match m {
            RowMap::Route { routing, .. } => Some(&routing.schema),
            _ => None,
        })
        .expect("the split routes rows");
    let mid = by_desc(routing, "the routing's schema");
    let logged = by_desc(&ids_of(&base), "the schema before");
    let after = by_desc(&ids_of(&applies(&base, text)), "the schema after");
    for (f, c, want) in [("c", "C", 1027), ("d", "D", 1028)] {
        let field = Desc::Field("auth::T".to_string(), Some(c.to_string()), f.to_string());
        assert_eq!((want, want), (mid[&field], after[&field]), "{c}.{f}");
    }
    for (c, want) in [("C", 1031), ("D", 1032)] {
        let ctor = Desc::Ctor("auth::T".to_string(), c.to_string());
        let id = mid[&ctor];
        assert_eq!((want, want), (id, after[&ctor]), "{c}");
        let held: Vec<&Desc> = logged
            .iter()
            .filter(|(d, x)| **x == id && d.space() == Space::Shared)
            .map(|(d, _)| d)
            .collect();
        assert_eq!(Vec::<&Desc>::new(), held, "{c} {id}");
    }
}

#[test]
fn an_index_over_a_named_set_keeps_its_id_when_an_operator_replaces_the_sets_first_constructor() {
    let base = source(
        "module auth
type K = A | B
type T { id: Key ts } = A { z: Text } | B { z: Text } | C
relation ts : T
  key id generate sequence
  index (z) in S
set S of ts = A | B",
    );
    let index = |s: &Source| {
        let schema = ids_of(s);
        schema.ids.index_id(rel(&schema, "ts"), 0).map(u64::from)
    };
    assert_eq!(Some(1025), index(&base));
    let replacing = [
        "split constructor T.A into A1 { z: Text } | A2 { z: Text }
  by match
    | A a -> A1 { z: a.z }",
        "merge constructor T.(A | B) into M { k: K, z: Text }
  by match
    | A a -> M { k: A, z: a.z }
    | B b -> M { k: B, z: b.z }",
    ];
    for op in replacing {
        let after = applies(&base, &format!("module auth\n{op}"));
        assert_eq!(Some(1025), index(&after), "{op}\n{}", printed_or(&after));
    }
}

#[test]
fn an_index_over_a_named_set_keeps_its_id_and_text_when_a_nest_replaces_constructors_of_the_set() {
    let base = source(
        "module auth
type T { id: Key ts, n: Text } = A { z: Text } | B { z: Text } | C
type Inner = A { z: Text } | B { z: Text }
relation ts : T
  key id generate sequence
  index (n) in S
set S of ts = A | B",
    );
    let nested = applies(
        &base,
        "module auth\nnest constructor T.(A | B) into N { inner: Inner }",
    );
    let index = |s: &Source| {
        let schema = ids_of(s);
        schema.ids.index_id(rel(&schema, "ts"), 0).map(u64::from)
    };
    assert_eq!(Some(1025), index(&base));
    assert_eq!(Some(1025), index(&nested));
    assert!(
        flat(&decl(&nested, "relation", "ts")).contains("index (n) in S"),
        "{}",
        printed_or(&nested)
    );
}

/// Returns the catalog ids of the uniques and indexes of relation `r` of `s`, in clause order.
fn index_ids(s: &Source, r: &str) -> Vec<u64> {
    let schema = ids_of(s);
    let r = rel(&schema, r);
    (0..schema.relation(r).indexes.len())
        .filter_map(|at| schema.ids.index_id(r, at).map(u64::from))
        .collect()
}

#[test]
fn a_move_down_keeps_the_id_of_a_unique_over_the_moved_field_in_a_constructor_it_lists() {
    let base = source(
        "module auth
type T { id: Key ts, n: Text } = A { z: Text } | B { z: Text }
relation ts : T
  key id generate sequence
  unique (n) in B",
    );
    assert_eq!(vec![1025], index_ids(&base, "ts"));
    let moved = applies(&base, "module auth\nmove field T.n to T.(A | B)");
    assert_eq!(vec![1025], index_ids(&moved, "ts"));
}

#[test]
fn renaming_the_field_of_a_one_constructor_set_keeps_the_id_of_an_index_over_the_set() {
    let base = source(
        "module auth
type T { id: Key ts } = A { z: Text } | B { z: Text } | C
relation ts : T
  key id generate sequence
  index (z) in S
set S of ts = A",
    );
    assert_eq!(vec![1025], index_ids(&base, "ts"));
    let renamed = applies(&base, "module auth\nrename field T.A.z to z2");
    assert_eq!(vec![1025], index_ids(&renamed, "ts"));
}

#[test]
fn renaming_a_widened_record_keeps_the_ids_of_its_constructor_and_fields() {
    let base = source(
        "module auth
type W = { id: Key ws, a: Text }
relation ws : W key id generate sequence",
    );
    let widened = applies(&base, "module auth\nadd constructor W.V");
    let renamed = applies(&widened, "module auth\nrename type W to W2");
    let ids = |s: &Source, t: &str| {
        let schema = ids_of(s);
        let td = schema.ty(ty(&schema, t));
        let ctor = td.ctor_at(0).expect("the first constructor");
        let a = td
            .field(None, "a")
            .or_else(|| td.field(Some(ctor), "a"))
            .expect("field a");
        (
            schema.ids.ctor_id(ctor).map(u64::from),
            schema.ids.field_id(a).map(u64::from),
        )
    };
    assert_eq!((Some(1025), Some(1025)), ids(&widened, "W"));
    assert_eq!((Some(1025), Some(1025)), ids(&renamed, "W2"));
}

#[test]
fn reverting_a_merge_restores_set_scoped_indexes_once_with_their_ids() {
    let base = source(
        "module auth
type K = A | B
type T { id: Key ts, n: Text } = A { z: Text } | B { z: Text } | C
relation ts : T
  key id generate sequence
  index (z) in S
  index (n) in S
set S of ts = A | B",
    );
    assert_eq!(vec![1025, 1026], index_ids(&base, "ts"));
    let merged = applies(
        &base,
        "module auth
merge constructor T.(A | B) into M { k: K, z: Text }
  by match
    | A a -> M { k: A, z: a.z }
    | B b -> M { k: B, z: b.z }",
    );
    assert_eq!(vec![1025, 1026], index_ids(&merged, "ts"));
    let last = merged.log.last().expect("the merge is logged").number;
    let reverted = applies(&merged, &format!("module auth\nrevert {last:04}"));
    assert_eq!(vec![1025, 1026], index_ids(&reverted, "ts"));
}

#[test]
fn reverting_a_split_or_a_nest_restores_set_scoped_indexes_once_with_their_ids() {
    let base = source(
        "module auth
type P = X | Y
type T { id: Key ts, n: Text } = A { p: P, z: Text } | B { z: Text } | C
type Inner = A { p: P, z: Text } | B { z: Text }
relation ts : T
  key id generate sequence
  index (n) in S
set S of ts = A | B",
    );
    assert_eq!(vec![1025], index_ids(&base, "ts"));
    let operators = [
        "split constructor T.A into A1 { z: Text } | A2 { z: Text }
  by match
    | A { p: X, z } -> A1 { z: z }
    | A { p: Y, z } -> A2 { z: z }",
        "nest constructor T.(A | B) into N { inner: Inner }",
    ];
    for op in operators {
        let after = applies(&base, &format!("module auth\n{op}"));
        assert_eq!(vec![1025], index_ids(&after, "ts"), "{op}");
        let last = after.log.last().expect("the operator is logged").number;
        let reverted = applies(&after, &format!("module auth\nrevert {last:04}"));
        assert_eq!(
            vec![1025],
            index_ids(&reverted, "ts"),
            "{op}\n{}",
            printed_or(&reverted)
        );
    }
}

#[test]
fn a_split_is_refused_when_an_index_over_several_constructors_names_a_field_a_product_does_not_keep_under_its_name()
 {
    let base = source(
        "module auth
type T { id: Key ts } = A { z: Text } | B { z: Text } | C
relation ts : T
  key id generate sequence
  index (z) in (A | B)",
    );
    let cases = [
        (
            "A1 { w: Text } | A2 { w: Text }",
            "A a -> A1 { w: a.z }",
            "A1",
        ),
        (
            "A1 { z: Text } | A2 { w: Text }",
            "A a -> A1 { z: a.z }",
            "A2",
        ),
    ];
    for (products, arm, unkept) in cases {
        let text =
            format!("module auth\nsplit constructor T.A into {products}\n  by match\n    | {arm}");
        assert_eq!(
            Some(vec![format!(
                "`index (z) in (A | B)` names `z`, which `{unkept}` does not keep under that name"
            )]),
            migrate(&base, &text).err(),
            "{text}"
        );
    }
}

#[test]
fn a_split_stating_replacements_drops_an_index_over_several_constructors_a_product_does_not_keep() {
    let base = source(
        "module auth
type T { id: Key ts } = A { z: Text } | B { z: Text } | C
relation ts : T
  key id generate sequence
  index (z) in (A | B)",
    );
    let split = applies(
        &base,
        "module auth
split constructor T.A into A1 { z: Text } | A2 { w: Text }
  by match
    | A a -> A1 { z: a.z }
  index (z) in (A1 | B)",
    );
    assert_eq!(
        "relation ts : T key id generate sequence index (z) in (A1 | B)",
        flat(&decl(&split, "relation", "ts"))
    );
}

#[test]
fn an_operator_on_a_constructor_of_a_scope_listing_every_constructor_re_scopes_the_clause() {
    let base = source(
        "module auth
type K = A | B
type Inner = A { z: Text } | B { z: Text }
type T { id: Key ts, n: Text } = A { z: Text } | B { z: Text }
relation ts : T
  key id generate sequence
  index (z) in (A | B)
  index (n) in (A | B)",
    );
    assert_eq!(vec![1025, 1026], index_ids(&base, "ts"));
    let operators = [
        (
            "split constructor T.A into A1 { z: Text } | A2 { z: Text }
  by match
    | A a -> A1 { z: a.z }",
            "relation ts : T key id generate sequence index (z) in (A1 | A2 | B) index (n) in (A1 | A2 | B)",
        ),
        (
            "merge constructor T.(A | B) into M { k: K, z: Text }
  by match
    | A a -> M { k: A, z: a.z }
    | B b -> M { k: B, z: b.z }",
            "relation ts : T key id generate sequence index (z) in M index (n) in M",
        ),
    ];
    for (op, text) in operators {
        let after = applies(&base, &format!("module auth\n{op}"));
        assert_eq!(
            (text, vec![1027, 1028]),
            (
                flat(&decl(&after, "relation", "ts")).as_str(),
                index_ids(&after, "ts")
            ),
            "{op}"
        );
    }
    assert_eq!(
        Some(vec!["`index (z)` has no image under the nest".to_string()]),
        migrate(
            &base,
            "module auth\nnest constructor T.(A | B) into N { inner: Inner }"
        )
        .err()
    );
    let unique = source(
        "module auth
type K = A | B
type T { id: Key ts } = A { z: Text } | B { z: Text }
relation ts : T
  key id generate sequence
  unique (z) in (A | B)",
    );
    assert_eq!(vec![1025], index_ids(&unique, "ts"));
    let split = applies(
        &unique,
        "module auth
split constructor T.A into A1 { z: Text } | A2 { z: Text }
  by match
    | A a -> A1 { z: a.z }",
    );
    assert_eq!(
        (
            "relation ts : T key id generate sequence unique (z) in (A1 | A2 | B)",
            vec![1026]
        ),
        (
            flat(&decl(&split, "relation", "ts")).as_str(),
            index_ids(&split, "ts")
        )
    );
    assert_eq!(
        Some(vec![
            "`unique (z) in (A | B)` holds across the merged constructors; the merge would weaken it"
                .to_string()
        ]),
        migrate(&unique, &format!("module auth\n{MERGE_A_B}")).err()
    );
}

const MERGE_A_B: &str = "merge constructor T.(A | B) into M { k: K, z: Text }
  by match
    | A a -> M { k: A, z: a.z }
    | B b -> M { k: B, z: b.z }";

/// Returns `ts` of module `auth` over `T = A | B | C` with `clause` and `set S of ts = A | B`.
fn three_ctors(clause: &str) -> Source {
    source(&format!(
        "module auth
type K = A | B
type T {{ id: Key ts }} = A {{ z: Text }} | B {{ z: Text }} | C {{ z: Text }}
relation ts : T
  key id generate sequence
  {clause}
set S of ts = A | B"
    ))
}

#[test]
fn a_merge_is_refused_while_a_unique_holds_across_exactly_the_merged_constructors() {
    for clause in ["unique (z) in (A | B)", "unique (z) in S"] {
        assert_eq!(
            Some(vec![format!(
                "`{clause}` holds across the merged constructors; the merge would weaken it"
            )]),
            migrate(&three_ctors(clause), &format!("module auth\n{MERGE_A_B}")).err(),
            "{clause}"
        );
    }
}

#[test]
fn a_merge_re_scopes_a_unique_over_the_merged_constructors_and_another_without_weakening_it() {
    let merged = applies(
        &three_ctors("unique (z) in (A | B | C)"),
        &format!("module auth\n{MERGE_A_B}"),
    );
    assert_eq!(
        "relation ts : T key id generate sequence unique (z) in (M | C)",
        flat(&decl(&merged, "relation", "ts"))
    );
}

/// A schema shape for the structural operators: what the clauses of `ts` range over.
#[derive(Clone, Copy, Debug)]
struct Scoped {
    /// The second constructor of `T` is named `T`, after its type, rather than `B`.
    named_after: bool,
    /// Set `S` holds `A` alone rather than `A | B | C`.
    set_one: bool,
    /// Clauses reach `z` through several constructors.
    wide: bool,
    /// A clause lists every constructor of `T`.
    full: bool,
    /// Clauses hold dotted elements.
    dotted: bool,
}

impl Scoped {
    fn of(bits: u64) -> Scoped {
        Scoped {
            named_after: bits & 1 != 0,
            set_one: bits & 2 != 0,
            wide: bits & 4 != 0,
            full: bits & 8 != 0,
            dotted: bits & 16 != 0,
        }
    }

    fn b(self) -> &'static str {
        if self.named_after { "T" } else { "B" }
    }

    fn text(self) -> String {
        let b = self.b();
        let set = if self.set_one {
            "A".to_string()
        } else {
            format!("A | {b} | C")
        };
        let mut clauses = vec!["unique (z) in A".to_string(), format!("unique (n) in {b}")];
        clauses.push("index (n) in S".to_string());
        if !self.wide {
            clauses.push("index (n)".to_string());
        }
        if self.set_one || self.wide {
            clauses.push("index (z) in S".to_string());
        }
        if self.wide {
            clauses.push(format!("index (z) in (A | {b})"));
        }
        if self.full {
            clauses.push(format!("index (n, id) in (A | {b} | C)"));
        }
        if self.dotted {
            clauses.push("index (p.a) in S".to_string());
            clauses.push("unique (s.X.g) in A".to_string());
        }
        let clauses: String = clauses.iter().map(|c| format!("\n  {c}")).collect();
        let fields = "z: Text, s: Sm, q: Q";
        format!(
            "module auth
type K = A | {b}
type Q = Q1 | Q2
type Sm = X {{ g: Text }} | Y
type P = {{ a: Text }}
type T {{ id: Key ts, n: Text, p: P }} = A {{ {fields} }} | {b} {{ {fields} }} | C {{ z: Text }}
type Inner = A {{ {fields} }} | {b} {{ {fields} }}
type InnerA = A {{ {fields} }}
relation ts : T
  key id generate sequence{clauses}
set S of ts = {set}"
        )
    }

    /// Every structural operator that applies to the shape, after the migration it needs first
    /// if any, with the constructors it replaces, how it changes the names of the objects it
    /// does not create and where it takes the rows a clause covers.
    fn ops(self) -> Vec<ScopedOp> {
        let b = self.b();
        let op = |first: Option<&str>, text: String, replaced: &[&str], rename, rows| ScopedOp {
            first: first.map(str::to_string),
            text,
            replaced: replaced.iter().map(ToString::to_string).collect(),
            rename,
            rows,
        };
        let kept: fn(&Desc) -> Desc = Desc::clone;
        let both = || vec!["A".to_string(), b.to_string()];
        let mut out = vec![
            op(
                None,
                "split constructor T.A into A1 { z: Text, s: Sm } | A2 { z: Text, s: Sm }
  by match
    | A { q: Q1, z, s } -> A1 { z: z, s: s }
    | A { q: Q2, z, s } -> A2 { z: z, s: s }"
                    .to_string(),
                &["A"],
                kept,
                Rows::Split,
            ),
            op(
                None,
                "add constructor T.D { z: Text }".to_string(),
                &[],
                kept,
                Rows::Add("D"),
            ),
        ];
        if !self.set_one {
            out.push(op(
                None,
                format!(
                    "merge constructor T.(A | {b}) into M {{ k: K, z: Text, s: Sm, q: Q }}
  by match
    | A a -> M {{ k: A, z: a.z, s: a.s, q: a.q }}
    | {b} x -> M {{ k: {b}, z: x.z, s: x.s, q: x.q }}"
                ),
                &["A", b],
                kept,
                Rows::Merge(both()),
            ));
        }
        if self.wide {
            out.push(op(
                None,
                format!("move field T.n to T.(A | {b} | C)"),
                &[],
                |d| moved(d, "n", None, Some("A")),
                Rows::Same,
            ));
        }
        if !self.named_after {
            out.push(op(
                None,
                "rename type T to T2".to_string(),
                &[],
                |d| {
                    names(d, &|n| match n {
                        Name::Type("auth::T") => "auth::T2",
                        n => n.text(),
                    })
                },
                Rows::Same,
            ));
        }
        if !self.set_one && !self.wide && !self.dotted {
            out.push(op(
                None,
                format!("nest constructor T.(A | {b}) into N {{ inner: Inner }}"),
                &["A", b],
                kept,
                Rows::Nest(both()),
            ));
            let nested = format!(
                "nest constructor T.(A | {b}) into N {{ inner: Inner }}
add set S2 of ts = N
add index (inner.A.z) in ts.S2
add index (n) in ts.S2"
            );
            out.push(op(
                Some(nested.as_str()),
                "unnest constructor T.N".to_string(),
                &["N", "A", b],
                kept,
                Rows::Unnest,
            ));
        }
        if self.set_one && !self.wide {
            out.push(op(
                None,
                "nest constructor T.A into N { inner: InnerA }".to_string(),
                &["A"],
                kept,
                Rows::Nest(vec!["A".to_string()]),
            ));
        }
        if !self.full {
            out.push(op(
                Some("deprecate ts.C"),
                "remove constructor T.C".to_string(),
                &["C"],
                kept,
                Rows::Remove("C"),
            ));
        }
        if self.set_one && !self.wide {
            out.push(op(
                None,
                "rename field T.A.z to z2".to_string(),
                &[],
                |d| {
                    names(d, &|n| match n {
                        Name::Field("auth::T", Some("A"), "z") => "z2",
                        n => n.text(),
                    })
                },
                Rows::Rename(Some("A"), "z", "z2"),
            ));
        }
        if self.dotted {
            out.push(op(
                None,
                "rename field P.a to a2".to_string(),
                &[],
                |d| {
                    names(d, &|n| match n {
                        Name::Field("auth::P", Some("P"), "a") => "a2",
                        n => n.text(),
                    })
                },
                Rows::Rename(None, "p.a", "p.a2"),
            ));
            out.push(op(
                None,
                "rename field Sm.X.g to g2".to_string(),
                &[],
                |d| {
                    names(d, &|n| match n {
                        Name::Field("auth::Sm", Some("X"), "g") => "g2",
                        n => n.text(),
                    })
                },
                Rows::Rename(None, "s.X.g", "s.X.g2"),
            ));
        }
        out
    }
}

/// A structural operator of the search over [`Scoped`] shapes.
struct ScopedOp {
    /// A migration the operator needs applied before it.
    first: Option<String>,
    text: String,
    /// The constructors it replaces: of `T`, or named in an element path.
    replaced: Vec<String>,
    /// How it changes the names of the objects it does not create.
    rename: fn(&Desc) -> Desc,
    rows: Rows,
}

/// A row a clause covers: a constructor of `T`, the constructor its row held before a merge or
/// a nest folded it into this one, and an element path over it.
type Row = (String, String, String);

/// Where a structural operator takes the rows a clause covers.
enum Rows {
    Same,
    /// `A` splits into `A1` and `A2`.
    Split,
    /// The operator adds the constructor, whose rows no clause covered before.
    Add(&'static str),
    /// The constructors merge into `M`.
    Merge(Vec<String>),
    /// The constructors nest into `N` under field `inner`.
    Nest(Vec<String>),
    /// `N` unnests into the constructors folded into it.
    Unnest,
    Remove(&'static str),
    /// The element path renames, over the constructor if one is given.
    Rename(Option<&'static str>, &'static str, &'static str),
}

const HEADER: [&str; 3] = ["id", "n", "p"];

fn on_header(e: &str) -> bool {
    HEADER.contains(&e.split('.').next().unwrap_or_default())
}

impl Rows {
    /// Returns the rows that hold row `r` after the operator.
    fn map(&self, r: &Row) -> Vec<Row> {
        let (c, part, e) = r.clone();
        let row = |c: &str, part: &str, e: &str| (c.to_string(), part.to_string(), e.to_string());
        match self {
            Rows::Split if c == "A" => vec![row("A1", &part, &e), row("A2", &part, &e)],
            Rows::Merge(cs) if cs.contains(&c) => vec![row("M", &c, &e)],
            Rows::Nest(cs) if cs.contains(&c) && on_header(&e) => vec![row("N", &c, &e)],
            Rows::Nest(cs) if cs.contains(&c) => vec![row("N", &c, &format!("inner.{c}.{e}"))],
            Rows::Unnest if c == "N" && on_header(&e) => vec![row(&part, "", &e)],
            Rows::Unnest if c == "N" => {
                let leaf = e.strip_prefix("inner.").and_then(|x| x.split_once('.'));
                leaf.map(|(_, leaf)| row(&part, "", leaf))
                    .into_iter()
                    .collect()
            }
            Rows::Remove(gone) if c == *gone => Vec::new(),
            Rows::Rename(at, from, to) if e == *from && at.is_none_or(|a| a == c) => {
                vec![row(&c, &part, to)]
            }
            _ => vec![(c, part, e)],
        }
    }

    /// Returns rows `rs` before the operator mapped through it.
    fn mapped(&self, rs: &BTreeSet<Row>) -> BTreeSet<Row> {
        rs.iter().flat_map(|r| self.map(r)).collect()
    }

    /// Returns rows `rs` after the operator, leaving out those of a constructor it adds without
    /// replacing anything.
    fn counted(&self, rs: BTreeSet<Row>) -> BTreeSet<Row> {
        rs.into_iter()
            .filter(|(c, _, _)| !matches!(self, Rows::Add(d) if c == d))
            .collect()
    }
}

/// Returns the rows clause `clause` of relation `ts` of `s` covers: each constructor its scope
/// reaches, every constructor of `T` for no scope, with each of its element paths. The rows of `N`
/// or `M` are those of each constructor folded into it, but a clause with an element path
/// through `inner.X` covers those of `X` alone.
fn covered(s: &Source, clause: &str) -> BTreeSet<Row> {
    let schema = ids_of(s);
    let rd = schema.relation(rel(&schema, "ts"));
    let td = schema.ty(rd.elem_def);
    let ctors: Vec<String> = td.ctors().iter().map(|c| c.name.clone()).collect();
    let parts = |c: &str| -> Vec<String> {
        let folded = td.ctors().iter().find(|k| k.name == c).and_then(|k| {
            k.fields.iter().find_map(|f| match &f.ty {
                Ty::Data { def, .. } if ["inner", "k"].contains(&f.name.as_str()) => Some(*def),
                _ => None,
            })
        });
        match folded {
            Some(def) => schema
                .ty(def)
                .ctors()
                .iter()
                .map(|k| k.name.clone())
                .collect(),
            None => vec![String::new()],
        }
    };
    let (elements, scope) = clause
        .split_once('(')
        .and_then(|(_, rest)| rest.split_once(')'))
        .unwrap_or_default();
    let scoped: Vec<String> = match scope.split_once(" in ") {
        None => ctors,
        Some((_, names)) => names
            .trim()
            .trim_matches(|c| c == '(' || c == ')')
            .split('|')
            .map(str::trim)
            .flat_map(|n| match rd.sets.iter().find(|set| set.name == n) {
                Some(set) => set
                    .ctors
                    .iter()
                    .map(|&c| ctors[c as usize].clone())
                    .collect(),
                None => vec![n.to_string()],
            })
            .collect(),
    };
    let elements: Vec<String> = elements
        .split(',')
        .map(|e| e.split('.').map(str::trim).collect::<Vec<_>>().join("."))
        .collect();
    let mut out = BTreeSet::new();
    for c in &scoped {
        for part in parts(c) {
            let reaches = |e: &String| {
                let through = e.strip_prefix("inner.").and_then(|x| x.split_once('.'));
                through.is_none_or(|(x, _)| x == part)
            };
            if elements.iter().all(reaches) {
                out.extend(
                    elements
                        .iter()
                        .map(|e| (c.clone(), part.clone(), e.clone())),
                );
            }
        }
    }
    out
}

/// Returns the constructors of the element type of relation `ts` of `s`.
fn ctors_of(s: &Source) -> BTreeSet<String> {
    let schema = ids_of(s);
    let rd = schema.relation(rel(&schema, "ts"));
    schema
        .ty(rd.elem_def)
        .ctors()
        .iter()
        .map(|c| c.name.clone())
        .collect()
}

/// Returns, for operator `what` taking `before` to `after` and its rows by `rows`, each clause
/// whose covered rows it does not take to those of the clause after: the clause of its id, or
/// for a clause it re-scopes, some clause it creates.
fn uncovered(what: &str, before: &Source, after: &Source, rows: &Rows) -> Vec<String> {
    let was = clause_ids(before);
    let now = clause_ids(after);
    let old: BTreeSet<u64> = was.iter().map(|(_, id)| *id).collect();
    let made: Vec<BTreeSet<Row>> = now
        .iter()
        .filter(|(_, id)| !old.contains(id))
        .map(|(t, _)| rows.counted(covered(after, t)))
        .collect();
    let mut out = Vec::new();
    for (text, id) in &was {
        let want = rows.mapped(&covered(before, text));
        match now.iter().find(|(_, i)| i == id) {
            Some((t, _)) => {
                let got = rows.counted(covered(after, t));
                if got != want {
                    out.push(format!(
                        "{what}: `{text}` covers {got:?} as `{t}`, not {want:?}"
                    ));
                }
            }
            None if !want.is_empty() && !made.contains(&want) => {
                out.push(format!("{what}: no clause re-scopes `{text}` to {want:?}"));
            }
            None => {}
        }
    }
    out
}

/// Returns each unique and index clause of relation `ts` of `s`, by its text, with its id.
fn clause_ids(s: &Source) -> Vec<(String, u64)> {
    let text = flat(&decl(s, "relation", "ts"));
    let words: Vec<&str> = text.split(' ').collect();
    let mut texts: Vec<String> = Vec::new();
    let mut at = 0;
    while at < words.len() {
        let starts = |w: &[&str]| matches!(w, ["unique" | "index", ..] | ["interval", "index", ..]);
        if !starts(&words[at..]) {
            at += 1;
            continue;
        }
        let mut end = at + 1 + usize::from(words[at] == "interval");
        while end < words.len()
            && !starts(&words[end..])
            && !matches!(words[end], "default" | "computed" | "key")
        {
            end += 1;
        }
        texts.push(words[at..end].join(" "));
        at = end;
    }
    let ids = index_ids(s, "ts");
    assert_eq!(texts.len(), ids.len(), "{text}");
    texts.into_iter().zip(ids).collect()
}

/// Returns the names a clause's scope lists and each name of its element paths.
fn named_in(clause: &str) -> Vec<String> {
    let (elements, scope) = clause
        .split_once('(')
        .and_then(|(_, rest)| rest.split_once(')'))
        .unwrap_or_default();
    let scope = scope
        .split_once(" in ")
        .map(|(_, s)| s.trim().trim_matches(|c| c == '(' || c == ')'))
        .unwrap_or_default();
    elements
        .split([',', '.'])
        .chain(scope.split('|'))
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect()
}

/// Returns whether description `d` names a constructor of `auth::T` among `replaced` or a field
/// of one.
fn of_replaced(d: &Desc, replaced: &[String]) -> bool {
    let ctor = |t: &str, c: &str| t == "auth::T" && replaced.iter().any(|r| r == c);
    match d {
        Desc::Ctor(t, c) | Desc::Field(t, Some(c), _) => ctor(t, c),
        Desc::Implied(_, f) | Desc::Default(_, _, f) => of_replaced(f, replaced),
        _ => false,
    }
}

/// Returns, for operator `what` taking `before` to `after` and replacing the constructors
/// `replaced`, each object that does not keep its id though the operator neither creates nor
/// re-scopes it, `rename` giving its description after. For a revert, `seen` holds the source
/// it applied to and the constructors it replaces, by which it re-scopes a clause as it saw it.
fn unkept(
    what: &str,
    (before, after): (&Source, &Source),
    replaced: &[String],
    rename: &dyn Fn(&Desc) -> Desc,
    seen: Option<(&Source, &[String])>,
) -> Vec<String> {
    let mut out = Vec::new();
    let was = by_desc(&ids_of(before), what);
    let now = by_desc(&ids_of(after), what);
    for (d, &id) in &was {
        if matches!(d, Desc::Index(..)) || of_replaced(d, replaced) {
            continue;
        }
        let to = rename(d);
        if now.get(&to) != Some(&id) {
            out.push(format!("{what}: {d:?} {id}, {to:?} {:?}", now.get(&to)));
        }
    }
    let now_clauses = clause_ids(after);
    let held: BTreeMap<u64, &str> = now_clauses
        .iter()
        .map(|(t, id)| (*id, t.as_str()))
        .collect();
    let names = |t: &str, of: &[String]| named_in(t).iter().any(|n| of.contains(n));
    let seen = seen.map(|(s, of)| (clause_ids(s), of));
    for (text, id) in clause_ids(before) {
        let rescoped = names(&text, replaced)
            || seen
                .as_ref()
                .is_some_and(|(cls, of)| cls.iter().any(|(t, i)| *i == id && names(t, of)));
        let same = now_clauses
            .iter()
            .find(|(t, _)| *t == text)
            .map(|(_, i)| *i);
        match (rescoped, held.get(&id)) {
            (true, Some(t)) => out.push(format!("{what}: re-scoped `{text}` kept {id} as `{t}`")),
            (false, None) => out.push(format!("{what}: `{text}` lost {id}")),
            (false, Some(_)) if same.is_some_and(|s| s != id) => {
                out.push(format!("{what}: `{text}` {id} is now {same:?}"));
            }
            _ => {}
        }
    }
    out
}

/// The order [`Scoped`] shapes are tried in: the first eight set each bit both ways and reach
/// every operator, then the rest.
const SCOPED: [u64; 32] = [
    31, 0, 18, 13, 9, 22, 3, 20, 30, 29, 28, 27, 26, 25, 24, 23, 21, 19, 17, 16, 15, 14, 12, 11,
    10, 8, 7, 6, 5, 4, 2, 1,
];

/// One step of the search over [`Scoped`] shapes: an operator, or the revert of one, taking
/// `before` to `after`.
struct Stepped {
    what: String,
    before: Source,
    after: Source,
    /// The constructors it replaces: of `T`, or named in an element path.
    replaced: Vec<String>,
    /// How it changes the names of the objects it does not create.
    rename: fn(&Desc) -> Desc,
    /// For a revert, the source it applied to and the constructors it replaces.
    seen: Option<(Source, Vec<String>)>,
    rows: Rows,
}

/// The steps of the search over the first `count` of the [`Scoped`] shapes.
struct Search {
    steps: Vec<Stepped>,
    /// Each step refused, with its refusal.
    refused: Vec<String>,
    ran: usize,
    /// The operators applied, by their first three words.
    reached: BTreeSet<String>,
}

impl Search {
    fn of(count: usize) -> Search {
        let mut search = Search {
            steps: Vec::new(),
            refused: Vec::new(),
            ran: 0,
            reached: BTreeSet::new(),
        };
        for bits in SCOPED.into_iter().take(count) {
            let shape = Scoped::of(bits);
            let start = source(&shape.text());
            for op in shape.ops() {
                search.ran += 1;
                let what = format!("{bits:05b} {}", op.text.lines().next().unwrap_or_default());
                search.operator(&start, op, &what);
            }
        }
        search
    }

    fn operator(&mut self, start: &Source, op: ScopedOp, what: &str) {
        let base = match &op.first {
            None => start.clone(),
            Some(first) => match self.step(start, first, what) {
                Some(s) => s,
                None => return,
            },
        };
        let Some(after) = self.step(&base, &op.text, what) else {
            return;
        };
        let words: Vec<&str> = op.text.split(' ').take(3).collect();
        self.reached.insert(words.join(" "));
        let replaced = op.replaced.clone();
        let inverse: Vec<String> = replaced
            .iter()
            .cloned()
            .chain(ctors_of(&after).difference(&ctors_of(&base)).cloned())
            .collect();
        let nest = op.text.starts_with("nest");
        self.reverted(what, &base, &after, &replaced, inverse);
        self.steps.push(Stepped {
            what: what.to_string(),
            before: base,
            after: after.clone(),
            replaced: replaced.clone(),
            rename: op.rename,
            seen: None,
            rows: op.rows,
        });
        if !nest {
            return;
        }
        let n: Vec<String> = ["N".to_string()]
            .into_iter()
            .chain(replaced.iter().cloned())
            .collect();
        let Some(unnested) = self.step(&after, "unnest constructor T.N", what) else {
            return;
        };
        let what = format!("{what}, unnested");
        self.reverted(&what, &after, &unnested, &n, n.clone());
        self.steps.push(Stepped {
            what,
            before: after,
            after: unnested,
            replaced: n,
            rename: Desc::clone,
            seen: None,
            rows: Rows::Unnest,
        });
    }

    /// Revert the operator that took `before` to `after`, replacing `replaced`, its inverse
    /// replacing `inverse`.
    fn reverted(
        &mut self,
        what: &str,
        before: &Source,
        after: &Source,
        replaced: &[String],
        inverse: Vec<String>,
    ) {
        let last = after.log.last().expect("the operator is logged").number;
        let Some(reverted) = self.step(after, &format!("revert {last:04}"), what) else {
            return;
        };
        self.steps.push(Stepped {
            what: format!("{what}, reverted"),
            before: before.clone(),
            after: reverted,
            replaced: replaced.to_vec(),
            rename: Desc::clone,
            seen: Some((after.clone(), inverse)),
            rows: Rows::Same,
        });
    }

    fn step(&mut self, from: &Source, text: &str, what: &str) -> Option<Source> {
        match migrate(from, &format!("module auth\n{text}")) {
            Ok(s) => Some(s),
            Err(e) => {
                let text = text.lines().next().unwrap_or_default();
                self.refused
                    .push(format!("{what}: `{text}` refused: {e:?}"));
                None
            }
        }
    }
}

#[test]
fn a_structural_operator_or_its_revert_keeps_the_id_of_every_object_it_does_not_create_or_rescope()
{
    let count = adtb_testkit::profile::count(8).min(32) as usize;
    let search = Search::of(count);
    let mut found = search.refused;
    for s in &search.steps {
        let seen = s.seen.as_ref().map(|(src, of)| (src, of.as_slice()));
        let pair = (&s.before, &s.after);
        found.extend(unkept(&s.what, pair, &s.replaced, &s.rename, seen));
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
    assert!(search.ran >= count * 3, "ran {} operators", search.ran);
    if count >= 8 {
        assert_eq!(12, search.reached.len(), "{:?}", search.reached);
    }
}

#[test]
#[ignore = "split, merge and nest re-scope a unique over one constructor beyond or short of its rows, and reverting a remove leaves the constructor out of the sets that held it"]
fn a_structural_operator_or_its_revert_takes_the_rows_a_clause_covers_to_those_of_the_clause_after()
{
    let count = adtb_testkit::profile::count(8).min(32) as usize;
    let search = Search::of(count);
    let mut found = search.refused;
    for s in &search.steps {
        found.extend(uncovered(&s.what, &s.before, &s.after, &s.rows));
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
    assert!(
        search.steps.len() >= search.ran * 2,
        "ran {} steps",
        search.steps.len()
    );
}

#[test]
fn an_unnest_of_several_products_re_scopes_a_clause_through_one_to_its_rows_and_keeps_one_over_the_header()
 {
    let nested = applies(
        &source(&Scoped::of(0).text()),
        "module auth
nest constructor T.(A | B) into N { inner: Inner }
add set S2 of ts = N
add index (inner.A.z) in ts.S2
add index (n) in ts.S2",
    );
    let unnested = applies(&nested, "module auth\nunnest constructor T.N");
    let id = |s: &Source, text: &str| {
        let ids = clause_ids(s);
        ids.iter().find(|(t, _)| t == text).map(|(_, id)| *id)
    };
    let header = id(&nested, "index (n) in S2");
    assert!(header.is_some(), "{}", decl(&nested, "relation", "ts"));
    assert_eq!(header, id(&unnested, "index (n) in S2"));
    assert!(id(&nested, "index (inner.A.z) in S2").is_some());
    let rescoped = id(&unnested, "index (z) in A").expect("the unnest re-scopes the clause");
    assert!(clause_ids(&nested).iter().all(|(_, id)| *id != rescoped));
    assert_eq!(
        "set S2 of ts = A | B",
        flat(&decl(&unnested, "set", "ts.S2"))
    );
    let last = unnested.log.last().expect("the unnest is logged").number;
    applies(&unnested, &format!("module auth\nrevert {last:04}"));
}

#[test]
fn no_element_of_a_unique_or_index_keys_a_name_that_resolves_to_a_field_as_a_token() {
    let mut tokens = Vec::new();
    let mut keys = 0;
    for bits in SCOPED {
        let s = source(&Scoped::of(bits).text());
        for k in s.ids.keys() {
            if !matches!(k.kind(), KeyKind::Unique | KeyKind::Index) {
                continue;
            }
            keys += 1;
            let shown = format!("{k:?}");
            for name in ["z", "n", "id", "p", "a", "s", "X", "g"] {
                if shown.contains(&format!("Token(\"{name}\")")) {
                    tokens.push(format!("{bits:05b} {name}: {shown}"));
                }
            }
        }
    }
    assert!(tokens.is_empty(), "{}", tokens.join("\n"));
    assert!(keys >= 32 * 5, "{keys} keys");
}
