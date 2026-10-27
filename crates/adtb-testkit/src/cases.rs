//! Cases for differential testing, drawn type-directed: a schema of record and sum relations,
//! the writes that load it and a well-typed query over it, each a function of its seed alone.

use std::collections::BTreeMap;

use crate::Rng;

/// A schema, the writes that load a database of it and queries over that database.
#[derive(Clone, Debug)]
pub struct Case {
    /// The schema file of module `app`.
    pub schema: String,
    /// The migration that adds module `app` with the declarations of `schema`.
    pub migration: String,
    pub load: Vec<String>,
    pub queries: Vec<Query>,
}

impl Case {
    /// Returns the schema and each write as text, a write on a line.
    pub fn program(&self) -> String {
        let mut out = self.schema.clone();
        for w in &self.load {
            out.push_str(w);
            out.push('\n');
        }
        out
    }
}

/// A query without a predicate and its three restrictions by that predicate: to the bindings
/// where it is true, where it is false, and where it faults.
#[derive(Clone, Debug)]
pub struct Split {
    pub whole: String,
    pub parts: [String; 3],
}

/// A comprehension over the relations of a schema and a predicate of its bindings.
#[derive(Clone, Debug)]
pub struct Query {
    world: World,
    comp: Comp,
    /// The value grouped and the expression grouped by, when the query groups its bindings.
    group: Option<(E, E)>,
    p: E,
}

impl Query {
    pub fn text(&self) -> String {
        let mut r = Render::new(&self.world);
        match &self.group {
            None => r.comp(&self.comp),
            Some((value, by)) => {
                let gens = r.gens(&self.comp.gens);
                let filter = r.filter(&self.comp.filter);
                format!(
                    "(from {gens}{filter} group {} by {} into g \
                     select {{ key: g.key, n: count(t for t in g), s: sum(t for t in g) }})",
                    r.expr(value),
                    r.expr(by)
                )
            }
        }
    }

    /// Returns the query once per constructor its first generator ranges over, that generator
    /// ranging over the constructor's relation alone, when it ranges over a sum relation.
    pub fn by_ctor(&self) -> Option<Vec<String>> {
        let b = &self.comp.gens[0];
        if !self.world.rels[b.rel].sum || self.group.is_some() {
            return None;
        }
        let parts = b
            .ctors
            .iter()
            .map(|&c| {
                let mut r = Render::new(&self.world);
                r.ctors.insert(b.var.clone(), vec![c]);
                r.comp(&self.comp)
            })
            .collect();
        Some(parts)
    }

    /// Returns the query split by its predicate, when it does not group its bindings.
    pub fn by_predicate(&self) -> Option<Split> {
        if self.group.is_some() {
            return None;
        }
        let parts = self
            .restrictions()
            .map(|c| Render::new(&self.world).comp(&c));
        Some(Split {
            whole: self.text(),
            parts,
        })
    }

    /// Returns the count of the query's bindings and the counts of its restrictions by its
    /// predicate.
    pub fn counts(&self) -> Split {
        let count = |c: &Comp| {
            let mut r = Render::new(&self.world);
            let gens = r.gens(&c.gens);
            format!("count(1 for {gens}{})", r.filter(&c.filter))
        };
        Split {
            whole: count(&self.comp),
            parts: self.restrictions().map(|c| count(&c)),
        }
    }

    fn restrictions(&self) -> [Comp; 3] {
        [Tried::True, Tried::False, Tried::Faults].map(|t| {
            let tried = E::Tried(Box::new(self.p.clone()), t);
            let filter = match &self.comp.filter {
                Some(f) => E::Bin("and", Box::new(f.clone()), Box::new(tried)),
                None => tried,
            };
            Comp {
                filter: Some(filter),
                ..self.comp.clone()
            }
        })
    }
}

/// Returns the case of `seed`, holding `queries` queries.
pub fn case(seed: u64, queries: usize) -> Case {
    let mut rng = Rng::new(seed);
    let world = World::draw(&mut rng);
    let load = world.load(&mut rng);
    let decls = world.decls();
    let mut schema = String::from("module app\n");
    let mut migration = String::from("module app\nadd module app\n");
    for (verb, d) in &decls {
        schema.push_str(&format!("{d}\n"));
        migration.push_str(&format!("{verb} {d}\n"));
    }
    let queries = (0..queries)
        .map(|_| query(&mut rng, world.clone()))
        .collect();
    Case {
        schema,
        migration,
        load,
        queries,
    }
}

fn query(rng: &mut Rng, world: World) -> Query {
    let mut g = Gen {
        w: &world,
        scope: Vec::new(),
        next: 0,
    };
    let sel = g.select_ty(rng);
    let comp = g.comp(rng, &sel, 3, 2);
    let (p, group) = g.within(&comp.gens, |g| {
        let group = (rng.below(5) == 0).then(|| {
            let by = g.leaf_ty(rng);
            (g.expr(rng, &T::Int, 2), g.expr(rng, &by, 1))
        });
        (g.boolean(rng, 2), group)
    });
    Query {
        world,
        comp,
        group,
        p,
    }
}

/// The declarations of every case before its relations, each with the verb that adds it in a
/// migration. A literal of a sum type passes through a function of that type, so its type is the
/// whole sum and not its one constructor.
const PRELUDE: [(&str, &str); 3] = [
    ("add", "type Shade = Red | Green | Blue { n: I64 }"),
    ("define", "fn shade (s: Shade) -> Shade = s"),
    ("define", "fn opt (o: Option I64) -> Option I64 = o"),
];

/// The type of a field or an expression.
#[derive(Clone, Debug, PartialEq, Eq)]
enum T {
    Int,
    Text,
    Bool,
    Shade,
    OptInt,
    /// A key of the relation of this index.
    Key(usize),
    /// A row of the relation of this index, of one of these constructors.
    Row(usize, Vec<usize>),
    Rec(Vec<(String, T)>),
    Set(Box<T>),
}

impl T {
    fn name(&self) -> String {
        match self {
            T::Int => "I64".into(),
            T::Text => "Text".into(),
            T::Bool => "Bool".into(),
            T::Shade => "Shade".into(),
            T::OptInt => "Option I64".into(),
            T::Key(r) => format!("Key r{r}"),
            T::Row(..) | T::Rec(_) | T::Set(_) => unreachable!("a field type is a scalar or a key"),
        }
    }
}

#[derive(Clone, Debug)]
struct Field {
    name: String,
    ty: T,
}

#[derive(Clone, Debug)]
struct Ctor {
    name: String,
    fields: Vec<Field>,
}

/// A relation of a type of its own: a record, its one constructor named as the type and its
/// fields all header fields, or a sum of at least two constructors.
#[derive(Clone, Debug)]
struct Rel {
    name: String,
    ty: String,
    sum: bool,
    /// The header fields after the key `id`.
    header: Vec<Field>,
    ctors: Vec<Ctor>,
    /// The `index` clauses of the relation.
    indexes: Vec<String>,
}

#[derive(Clone, Debug)]
struct World {
    rels: Vec<Rel>,
}

impl World {
    fn draw(rng: &mut Rng) -> World {
        let n = 2 + rng.below(2) as usize;
        World {
            rels: (0..n).map(|i| Rel::draw(rng, i)).collect(),
        }
    }

    fn decls(&self) -> Vec<(&'static str, String)> {
        let mut out: Vec<(&str, String)> =
            PRELUDE.iter().map(|(v, d)| (*v, (*d).to_owned())).collect();
        for r in &self.rels {
            out.push(("add", r.type_decl()));
            out.push(("add", r.relation_decl()));
        }
        out
    }

    /// Returns the writes inserting rows of every constructor, each a key of the relation its
    /// key fields name.
    fn load(&self, rng: &mut Rng) -> Vec<String> {
        let mut keys: Vec<u64> = Vec::new();
        let mut out = Vec::new();
        for r in &self.rels {
            let refs_empty = r
                .header
                .iter()
                .any(|f| matches!(f.ty, T::Key(j) if keys[j] == 0));
            let mut n = 0;
            for c in &r.ctors {
                let rows = if refs_empty { 0 } else { rng.below(5) };
                for _ in 0..rows {
                    n += 1;
                    out.push(r.insert(rng, c, n, &keys));
                }
            }
            keys.push(n);
        }
        out
    }
}

impl Rel {
    fn draw(rng: &mut Rng, i: usize) -> Rel {
        let mut next = 0;
        let mut field = |rng: &mut Rng, refs: bool| {
            next += 1;
            let mut tys = vec![T::Int, T::Int, T::Text, T::Bool, T::Shade, T::OptInt];
            if refs {
                tys.extend((0..i).map(T::Key));
            }
            Field {
                name: format!("f{next}"),
                ty: rng.pick(&tys).clone(),
            }
        };
        let header: Vec<Field> = (0..=rng.below(2)).map(|_| field(rng, true)).collect();
        let sum = rng.below(3) != 0;
        let ty = format!("T{i}");
        let ctors = if sum {
            (0..2 + rng.below(2) as u8)
                .map(|k| Ctor {
                    name: format!("C{i}{}", char::from(b'a' + k)),
                    fields: (0..rng.below(3)).map(|_| field(rng, false)).collect(),
                })
                .collect()
        } else {
            vec![Ctor {
                name: ty.clone(),
                fields: Vec::new(),
            }]
        };
        let indexable = |f: &Field| matches!(f.ty, T::Int | T::Text | T::Key(_));
        let mut indexes = Vec::new();
        for f in header.iter().filter(|f| indexable(f)) {
            if rng.below(2) == 0 {
                indexes.push(format!("index ({})", f.name));
            }
        }
        for c in ctors.iter().filter(|_| sum) {
            for f in c.fields.iter().filter(|f| indexable(f)) {
                if rng.below(3) == 0 {
                    indexes.push(format!("index ({}) in {}", f.name, c.name));
                }
            }
        }
        Rel {
            name: format!("r{i}"),
            ty,
            sum,
            header,
            ctors,
            indexes,
        }
    }

    fn type_decl(&self) -> String {
        let fields = |id: bool, fs: &[Field]| {
            let mut out: Vec<String> = Vec::new();
            if id {
                out.push(format!("id: Key {}", self.name));
            }
            out.extend(fs.iter().map(|f| format!("{}: {}", f.name, f.ty.name())));
            format!("{{ {} }}", out.join(", "))
        };
        if !self.sum {
            return format!("type {} = {}", self.ty, fields(true, &self.header));
        }
        let mut out = format!("type {} {}", self.ty, fields(true, &self.header));
        for (k, c) in self.ctors.iter().enumerate() {
            out.push_str(if k == 0 { "\n  = " } else { "\n  | " });
            out.push_str(&c.name);
            if !c.fields.is_empty() {
                out.push(' ');
                out.push_str(&fields(false, &c.fields));
            }
        }
        out
    }

    fn relation_decl(&self) -> String {
        let mut out = format!(
            "relation {} : {} key id generate sequence",
            self.name, self.ty
        );
        for i in &self.indexes {
            out.push_str("\n  ");
            out.push_str(i);
        }
        out
    }

    /// Returns the insert of row `n` of constructor `c`, a key field drawing from the first
    /// `keys[j]` keys of relation `j`.
    fn insert(&self, rng: &mut Rng, c: &Ctor, n: u64, keys: &[u64]) -> String {
        let mut fields = vec![format!("id: {}.key({n})", self.name)];
        for f in self.header.iter().chain(&c.fields) {
            fields.push(format!("{}: {}", f.name, value(rng, &f.ty, keys)));
        }
        let target = if self.sum {
            format!("{}.{}", self.name, c.name)
        } else {
            self.name.clone()
        };
        format!("insert {target} {{ {} }}", fields.join(", "))
    }
}

fn int(rng: &mut Rng) -> String {
    let i = rng.below(7) as i64 - 2;
    if i < 0 {
        format!("({i})")
    } else {
        i.to_string()
    }
}

fn text(rng: &mut Rng) -> String {
    rng.pick(&[r#""""#, r#""a""#, r#""b""#, r#""ab""#, r#""é""#])
        .to_string()
}

fn value(rng: &mut Rng, t: &T, keys: &[u64]) -> String {
    match t {
        T::Int => int(rng),
        T::Text => text(rng),
        T::Bool => rng.pick(&["true", "false"]).to_string(),
        T::Shade => match rng.below(3) {
            0 => "Red".into(),
            1 => "Green".into(),
            _ => format!("Blue {{ n: {} }}", int(rng)),
        },
        T::OptInt => match rng.below(2) {
            0 => "None".into(),
            _ => format!("Some {{ value: {} }}", int(rng)),
        },
        T::Key(j) => format!("r{j}.key({})", 1 + rng.below(keys[*j])),
        T::Row(..) | T::Rec(_) | T::Set(_) => unreachable!("a field type is a scalar or a key"),
    }
}

/// How a restriction tests its predicate under `try`.
#[derive(Clone, Copy, Debug)]
enum Tried {
    True,
    False,
    Faults,
}

/// An expression, rendered once its generators' constructor sets are known.
#[derive(Clone, Debug)]
enum E {
    Lit(String),
    /// A field of a variable.
    Path(String, String),
    KeyOf(String),
    Bin(&'static str, Box<E>, Box<E>),
    Not(Box<E>),
    If(Box<E>, Box<E>, Box<E>),
    /// A match on a row variable: an arm per constructor, each binding a variable. Only the arms
    /// of the constructors the variable ranges over are rendered.
    Row(String, usize, Vec<(usize, String, E)>),
    /// A match on a `Shade`: the arms for `Red`, `Green` and `Blue`, `Blue` binding a variable.
    Shade(Box<E>, Box<E>, Box<E>, String, Box<E>),
    /// A match on an `Option I64`: the arms for `None` and `Some`, `Some` binding a variable.
    Opt(Box<E>, Box<E>, String, Box<E>),
    Call(&'static str, Box<E>),
    /// An aggregate of the select expression of the comprehension over its bindings.
    Agg(&'static str, Box<Comp>),
    Exists(Box<Comp>),
    Set(Box<Comp>),
    Union(Box<E>, Box<E>),
    List(Vec<E>),
    Rec(Vec<(String, E)>),
    Tried(Box<E>, Tried),
    /// `fix x = base union (from y in x, v in r where v.into == y select v.out) in x`, as the
    /// names `x`, `y`, `v`, the relation, the field `into`, the field `out` and the base.
    Fix([String; 3], usize, String, String, Box<Comp>),
}

#[derive(Clone, Debug)]
struct Binding {
    var: String,
    rel: usize,
    /// The constructors the variable ranges over, ascending.
    ctors: Vec<usize>,
}

#[derive(Clone, Debug)]
struct Comp {
    gens: Vec<Binding>,
    filter: Option<E>,
    select: E,
}

struct Render<'w> {
    w: &'w World,
    /// The constructors each variable ranges over, set before rendering to narrow a generator.
    ctors: BTreeMap<String, Vec<usize>>,
}

impl<'w> Render<'w> {
    fn new(w: &'w World) -> Render<'w> {
        Render {
            w,
            ctors: BTreeMap::new(),
        }
    }

    fn comp(&mut self, c: &Comp) -> String {
        let gens = self.gens(&c.gens);
        let filter = self.filter(&c.filter);
        format!("(from {gens}{filter} select {})", self.expr(&c.select))
    }

    fn gens(&mut self, gens: &[Binding]) -> String {
        let mut out = Vec::new();
        for b in gens {
            let ctors = self
                .ctors
                .entry(b.var.clone())
                .or_insert_with(|| b.ctors.clone())
                .clone();
            let r = &self.w.rels[b.rel];
            let range = if ctors.len() == r.ctors.len() {
                r.name.clone()
            } else if let [c] = ctors.as_slice() {
                format!("{}.{}", r.name, r.ctors[*c].name)
            } else {
                let names: Vec<&str> = ctors.iter().map(|c| r.ctors[*c].name.as_str()).collect();
                format!("{}.({})", r.name, names.join(" | "))
            };
            out.push(format!("{} in {range}", b.var));
        }
        out.join(", ")
    }

    fn filter(&mut self, f: &Option<E>) -> String {
        match f {
            Some(f) => format!(" where {}", self.expr(f)),
            None => String::new(),
        }
    }

    fn expr(&mut self, e: &E) -> String {
        match e {
            E::Lit(s) => s.clone(),
            E::Path(v, f) => format!("{v}.{f}"),
            E::KeyOf(v) => format!("(key {v})"),
            E::Bin(op, a, b) => format!("({} {op} {})", self.expr(a), self.expr(b)),
            E::Not(a) => format!("(not {})", self.expr(a)),
            E::If(c, a, b) => format!(
                "(if {} then {} else {})",
                self.expr(c),
                self.expr(a),
                self.expr(b)
            ),
            E::Row(v, rel, arms) => self.row(v, *rel, arms),
            E::Shade(s, red, green, x, blue) => format!(
                "(match {} | Red -> {} | Green -> {} | Blue {x} -> {})",
                self.expr(s),
                self.expr(red),
                self.expr(green),
                self.expr(blue)
            ),
            E::Opt(o, none, x, some) => format!(
                "(match {} | None -> {} | Some {x} -> {})",
                self.expr(o),
                self.expr(none),
                self.expr(some)
            ),
            E::Call(f, a) => format!("{f}({})", self.expr(a)),
            E::Agg(f, c) => {
                let gens = self.gens(&c.gens);
                let filter = self.filter(&c.filter);
                format!("{f}({} for {gens}{filter})", self.expr(&c.select))
            }
            E::Exists(c) => {
                let gens = self.gens(&c.gens);
                format!("(exists {gens}{})", self.filter(&c.filter))
            }
            E::Set(c) => self.comp(c),
            E::Union(a, b) => format!("({} union {})", self.expr(a), self.expr(b)),
            E::List(xs) => {
                let xs: Vec<String> = xs.iter().map(|x| self.expr(x)).collect();
                format!("[{}]", xs.join(", "))
            }
            E::Rec(fs) => {
                let fs: Vec<String> = fs
                    .iter()
                    .map(|(n, x)| format!("{n}: {}", self.expr(x)))
                    .collect();
                format!("{{ {} }}", fs.join(", "))
            }
            E::Fix([x, y, v], rel, into, out, base) => format!(
                "(fix {x} = ({} union (from {y} in {x}, {v} in {} where ({v}.{into} == {y}) \
                 select {v}.{out})) in {x})",
                self.comp(base),
                self.w.rels[*rel].name
            ),
            E::Tried(p, t) => {
                let (ok, err) = match t {
                    Tried::True => ("tried.value", "false"),
                    Tried::False => ("(not tried.value)", "false"),
                    Tried::Faults => ("false", "true"),
                };
                format!(
                    "(match try {} | Ok tried -> {ok} | Err _ -> {err})",
                    self.expr(p)
                )
            }
        }
    }

    fn row(&mut self, v: &str, rel: usize, arms: &[(usize, String, E)]) -> String {
        let ctors = self.ctors.get(v).cloned().unwrap_or_default();
        let mut out = format!("(match {v}");
        for (c, x, body) in arms.iter().filter(|(c, ..)| ctors.contains(c)) {
            self.ctors.insert(x.clone(), vec![*c]);
            let name = &self.w.rels[rel].ctors[*c].name;
            out.push_str(&format!(" | {name} {x} -> {}", self.expr(body)));
        }
        out.push(')');
        out
    }
}

/// Draws expressions over the relations of one world, naming variables freshly.
struct Gen<'w> {
    w: &'w World,
    scope: Vec<(String, T)>,
    next: usize,
}

impl Gen<'_> {
    fn fresh(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    fn within<R>(&mut self, gens: &[Binding], f: impl FnOnce(&mut Self) -> R) -> R {
        let depth = self.scope.len();
        for b in gens {
            self.scope
                .push((b.var.clone(), T::Row(b.rel, b.ctors.clone())));
        }
        let r = f(self);
        self.scope.truncate(depth);
        r
    }

    fn select_ty(&mut self, rng: &mut Rng) -> T {
        let key = T::Key(rng.below(self.w.rels.len() as u64) as usize);
        match rng.below(7) {
            0 => T::Int,
            1 => T::Text,
            2 => T::Bool,
            3 => key,
            4 => T::Rec(vec![("a".into(), T::Int), ("b".into(), key)]),
            5 => T::Rec(vec![("a".into(), T::Text), ("b".into(), T::Shade)]),
            _ => T::Rec(vec![
                ("a".into(), key),
                ("b".into(), T::Set(Box::new(T::Int))),
            ]),
        }
    }

    fn binding(&mut self, rng: &mut Rng) -> Binding {
        let rel = rng.below(self.w.rels.len() as u64) as usize;
        let n = self.w.rels[rel].ctors.len();
        let all: Vec<usize> = (0..n).collect();
        let ctors = if n > 1 && rng.below(3) == 0 {
            let picked: Vec<usize> = all.iter().copied().filter(|_| rng.below(2) == 0).collect();
            if picked.is_empty() {
                vec![rng.below(n as u64) as usize]
            } else {
                picked
            }
        } else {
            all
        };
        Binding {
            var: self.fresh("v"),
            rel,
            ctors,
        }
    }

    /// Returns a comprehension of `sel` over at most `gens` generators.
    fn comp(&mut self, rng: &mut Rng, sel: &T, d: u32, gens: u64) -> Comp {
        let gens: Vec<Binding> = (0..=rng.below(gens)).map(|_| self.binding(rng)).collect();
        self.within(&gens.clone(), |g| {
            let filter = (rng.below(3) != 0).then(|| g.boolean(rng, d));
            let select = g.expr(rng, sel, d);
            Comp {
                gens,
                filter,
                select,
            }
        })
    }

    fn expr(&mut self, rng: &mut Rng, t: &T, d: u32) -> E {
        if d == 0 || rng.below(3) == 0 {
            return self.leaf(rng, t);
        }
        let d = d - 1;
        if matches!(
            t,
            T::Int | T::Text | T::Bool | T::Shade | T::OptInt | T::Key(_)
        ) {
            match rng.below(8) {
                0 => return self.matched(rng, t, d),
                1 => {
                    return E::If(
                        Box::new(self.boolean(rng, d)),
                        Box::new(self.expr(rng, t, d)),
                        Box::new(self.expr(rng, t, d)),
                    );
                }
                _ => {}
            }
        }
        match t {
            T::Int => self.int(rng, d),
            T::Text => self.text(rng, d),
            T::Bool => self.boolean(rng, d),
            T::Shade | T::OptInt | T::Key(_) | T::Row(..) => self.leaf(rng, t),
            T::Rec(fs) => E::Rec(
                fs.iter()
                    .map(|(n, ft)| (n.clone(), self.expr(rng, ft, d)))
                    .collect(),
            ),
            T::Set(elem) => self.set(rng, elem, d),
        }
    }

    /// Returns the expressions of type `t` that read a variable in scope.
    fn reads(&self, t: &T) -> Vec<E> {
        let mut out = Vec::new();
        for (v, vt) in &self.scope {
            let T::Row(r, cs) = vt else { continue };
            let rel = &self.w.rels[*r];
            if *t == T::Key(*r) {
                out.push(E::KeyOf(v.clone()));
            }
            let own: &[Field] = match cs.as_slice() {
                [c] => &rel.ctors[*c].fields,
                _ => &[],
            };
            for f in rel.header.iter().chain(own).filter(|f| f.ty == *t) {
                out.push(E::Path(v.clone(), f.name.clone()));
            }
        }
        out
    }

    fn leaf(&mut self, rng: &mut Rng, t: &T) -> E {
        let reads = self.reads(t);
        if !reads.is_empty() && rng.below(4) != 0 {
            return rng.pick(&reads).clone();
        }
        match t {
            T::Int | T::Text | T::Bool => E::Lit(value(rng, t, &[])),
            T::Shade => E::Lit(format!("shade({})", value(rng, t, &[]))),
            T::OptInt => E::Lit(format!("opt({})", value(rng, t, &[]))),
            T::Key(r) => E::Lit(format!("r{r}.key({})", 1 + rng.below(3))),
            T::Row(..) => unreachable!("no expression has a row type"),
            T::Rec(fs) => E::Rec(
                fs.iter()
                    .map(|(n, ft)| (n.clone(), self.leaf(rng, ft)))
                    .collect(),
            ),
            T::Set(elem) => match elem.as_ref() {
                T::Int | T::Text | T::Bool => E::List(vec![self.leaf(rng, elem)]),
                _ => E::Set(Box::new(self.comp(rng, elem, 0, 1))),
            },
        }
    }

    /// Returns a match of type `t` on a row variable of a sum, a `Shade` or an `Option I64`.
    fn matched(&mut self, rng: &mut Rng, t: &T, d: u32) -> E {
        let rows: Vec<(String, usize, Vec<usize>)> = self
            .scope
            .iter()
            .filter_map(|(v, vt)| match vt {
                T::Row(r, cs) if self.w.rels[*r].sum => Some((v.clone(), *r, cs.clone())),
                _ => None,
            })
            .collect();
        match rng.below(3) {
            0 if !rows.is_empty() => {
                let (v, r, cs) = rng.pick(&rows).clone();
                let mut arms = Vec::new();
                for c in cs {
                    let x = self.fresh("a");
                    let body = self.with(&x, T::Row(r, vec![c]), |g| g.expr(rng, t, d));
                    arms.push((c, x, body));
                }
                E::Row(v, r, arms)
            }
            1 => {
                let s = self.expr(rng, &T::Shade, d);
                let (red, green) = (self.expr(rng, t, d), self.expr(rng, t, d));
                let x = self.fresh("s");
                let n = T::Rec(vec![("n".into(), T::Int)]);
                let blue = self.with(&x, n, |g| g.expr(rng, t, d));
                E::Shade(
                    Box::new(s),
                    Box::new(red),
                    Box::new(green),
                    x,
                    Box::new(blue),
                )
            }
            _ => {
                let o = self.expr(rng, &T::OptInt, d);
                let none = self.expr(rng, t, d);
                let x = self.fresh("o");
                let value = T::Rec(vec![("value".into(), T::Int)]);
                let some = self.with(&x, value, |g| g.expr(rng, t, d));
                E::Opt(Box::new(o), Box::new(none), x, Box::new(some))
            }
        }
    }

    fn with<R>(&mut self, v: &str, t: T, f: impl FnOnce(&mut Self) -> R) -> R {
        self.scope.push((v.to_owned(), t));
        let r = f(self);
        self.scope.pop();
        r
    }

    fn int(&mut self, rng: &mut Rng, d: u32) -> E {
        let bin = |g: &mut Self, rng: &mut Rng, op| {
            E::Bin(
                op,
                Box::new(g.expr(rng, &T::Int, d)),
                Box::new(g.expr(rng, &T::Int, d)),
            )
        };
        match rng.below(16) {
            0..=2 => bin(self, rng, "+"),
            3 | 4 => bin(self, rng, "-"),
            5 => bin(self, rng, "*"),
            6 => {
                let op = *rng.pick(&["/", "%"]);
                bin(self, rng, op)
            }
            7 => E::Call("scalar_len", Box::new(self.expr(rng, &T::Text, d))),
            8..=10 => {
                let sel = self.leaf_ty(rng);
                E::Agg("count", Box::new(self.comp(rng, &sel, d, 1)))
            }
            11 | 12 => E::Agg("sum", Box::new(self.comp(rng, &T::Int, d, 1))),
            _ => self.leaf(rng, &T::Int),
        }
    }

    fn text(&mut self, rng: &mut Rng, d: u32) -> E {
        match rng.below(3) {
            0 => E::Bin(
                "++",
                Box::new(self.expr(rng, &T::Text, d)),
                Box::new(self.expr(rng, &T::Text, d)),
            ),
            _ => self.leaf(rng, &T::Text),
        }
    }

    fn boolean(&mut self, rng: &mut Rng, d: u32) -> E {
        let cmp = |g: &mut Self, rng: &mut Rng, t: &T, op| {
            E::Bin(op, Box::new(g.expr(rng, t, d)), Box::new(g.expr(rng, t, d)))
        };
        match rng.below(14) {
            0..=2 => {
                let op = *rng.pick(&["<", "<=", ">", ">=", "==", "!="]);
                cmp(self, rng, &T::Int, op)
            }
            3 => {
                let op = *rng.pick(&["==", "<"]);
                cmp(self, rng, &T::Text, op)
            }
            4..=6 => {
                let t = self.joinable(rng);
                cmp(self, rng, &t, "==")
            }
            7 => {
                let t = rng.pick(&[T::Shade, T::OptInt, T::Bool]).clone();
                cmp(self, rng, &t, "==")
            }
            8 | 9 => {
                let op = *rng.pick(&["and", "or"]);
                cmp(self, rng, &T::Bool, op)
            }
            10 => E::Not(Box::new(self.expr(rng, &T::Bool, d))),
            11 | 12 => E::Exists(Box::new(self.comp(rng, &T::Bool, d, 1))),
            _ => self.leaf(rng, &T::Bool),
        }
    }

    /// Returns a key type some variable in scope reads, to compare keys across generators.
    fn joinable(&self, rng: &mut Rng) -> T {
        let tys: Vec<T> = (0..self.w.rels.len())
            .map(T::Key)
            .filter(|t| !self.reads(t).is_empty())
            .collect();
        if tys.is_empty() {
            T::Key(0)
        } else {
            rng.pick(&tys).clone()
        }
    }

    fn leaf_ty(&self, rng: &mut Rng) -> T {
        let key = T::Key(rng.below(self.w.rels.len() as u64) as usize);
        rng.pick(&[T::Int, T::Text, T::Bool, key]).clone()
    }

    /// Returns the closure of a set of integers under a step from one field of a relation to
    /// another, when some relation has a header field of type `I64`.
    fn fix(&mut self, rng: &mut Rng, d: u32) -> Option<E> {
        let steps: Vec<(usize, String, String)> = self
            .w
            .rels
            .iter()
            .enumerate()
            .flat_map(|(r, rel)| {
                let ints: Vec<&Field> = rel.header.iter().filter(|f| f.ty == T::Int).collect();
                let pairs: Vec<(usize, String, String)> = ints
                    .iter()
                    .flat_map(|a| {
                        ints.iter()
                            .map(move |b| (r, a.name.clone(), b.name.clone()))
                    })
                    .collect();
                pairs
            })
            .collect();
        if steps.is_empty() {
            return None;
        }
        let (rel, into, out) = rng.pick(&steps).clone();
        let base = self.comp(rng, &T::Int, d, 1);
        let names = [self.fresh("x"), self.fresh("y"), self.fresh("v")];
        Some(E::Fix(names, rel, into, out, Box::new(base)))
    }

    fn set(&mut self, rng: &mut Rng, elem: &T, d: u32) -> E {
        let set = T::Set(Box::new(elem.clone()));
        match rng.below(5) {
            0 => E::Union(
                Box::new(self.expr(rng, &set, d)),
                Box::new(self.expr(rng, &set, d)),
            ),
            1 if matches!(elem, T::Int | T::Text | T::Bool) => E::List(
                (0..=rng.below(2))
                    .map(|_| self.expr(rng, elem, d))
                    .collect(),
            ),
            2 if *elem == T::Int => self.fix(rng, d).unwrap_or_else(|| self.leaf(rng, &set)),
            _ => E::Set(Box::new(self.comp(rng, elem, d, 2))),
        }
    }
}
