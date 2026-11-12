use std::collections::BTreeMap;

use super::*;
use crate::ids::Space;
use crate::parse::parse_migration;

/// A schema object described by the names a schema gives it and its position among clauses.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Desc {
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
    fn space(&self) -> Space {
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

fn type_name(s: &Schema, def: TypeId) -> String {
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
fn by_desc(s: &Schema, what: &str) -> BTreeMap<Desc, u64> {
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
#[ignore = "a clause element keyed by one resolved copy of its field loses the unique's id when a move down gives the listed constructor a new copy"]
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
#[ignore = "a clause element keyed by its field's name under a set scope loses the index's id when the field is renamed"]
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
fn an_operator_on_a_constructor_of_a_scope_listing_every_constructor_is_refused_because_the_scope_is_read_as_unscoped()
 {
    let base = source(
        "module auth
type K = A | B
type Inner = A { z: Text } | B { z: Text }
type T { id: Key ts } = A { z: Text } | B { z: Text }
relation ts : T
  key id generate sequence
  index (z) in (A | B)",
    );
    let operators = [
        "split constructor T.A into A1 { z: Text } | A2 { z: Text }
  by match
    | A a -> A1 { z: a.z }",
        "merge constructor T.(A | B) into M { k: K, z: Text }
  by match
    | A a -> M { k: A, z: a.z }
    | B b -> M { k: B, z: b.z }",
        "nest constructor T.(A | B) into N { inner: Inner }",
    ];
    for op in operators {
        assert_eq!(
            Some(vec!["`A` is not a constructor of `T`".to_string()]),
            migrate(&base, &format!("module auth\n{op}")).err(),
            "{op}"
        );
    }
}
