use std::fmt;

use crate::Rng;

/// The bodies of the function `shade`, each the identity on `Shade`. A literal of a sum type
/// passes through a function of that type, so its type is the whole sum and not its one
/// constructor.
pub(super) const SHADE: [&str; 2] = [
    "s",
    "match s | Red -> Red | Green -> Green | Blue b -> Blue { n: b.n }",
];

/// The float literals of a case, the last of greatest magnitude.
const FLOATS: [&str; 7] = [
    "0.0e0",
    "0.1e0",
    "0.3e0",
    "(-0.5e0)",
    "2.5e0",
    "(-3.75e0)",
    "1.0e6",
];

const FLOAT_MAX: f64 = 1_000_000.0;

/// The least integer literal a case draws.
pub(super) const INT_MIN: &str = "(-2)";

/// The type of a field or an expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum T {
    Int,
    Text,
    Bool,
    Shade,
    OptInt,
    Float,
    /// A sum of floats, which reaches only the result: no comparison or arithmetic reads it.
    FloatSum,
    /// A key of the relation of this index.
    Key(usize),
    /// A value of the sum of this index that a schema walk declares. No query reads one.
    Named(usize),
    /// A row of the relation of this index, of one of these constructors.
    Row(usize, Vec<usize>),
    Rec(Vec<(String, T)>),
    Set(Box<T>),
}

impl T {
    pub(super) fn name(&self, w: &World) -> String {
        match self {
            T::Int => "I64".into(),
            T::Text => "Text".into(),
            T::Bool => "Bool".into(),
            T::Shade => "Shade".into(),
            T::OptInt => "Option I64".into(),
            T::Float => "F64".into(),
            T::Key(r) => format!("Key {}", w.rels[*r].name),
            T::Named(s) => w.sums[*s].name.clone(),
            T::FloatSum | T::Row(..) | T::Rec(_) | T::Set(_) => {
                unreachable!("a field type is a scalar, a key or a declared sum")
            }
        }
    }

    pub(super) fn scalar(&self) -> bool {
        matches!(
            self,
            T::Int | T::Text | T::Bool | T::Shade | T::OptInt | T::Float | T::Key(_)
        )
    }
}

#[derive(Clone, Debug)]
pub(super) struct Field {
    pub(super) name: String,
    pub(super) ty: T,
    /// The delete action a key field states, when it states one.
    pub(super) on_delete: Option<&'static str>,
}

impl Field {
    pub(super) fn new(name: String, ty: T) -> Field {
        Field {
            name,
            ty,
            on_delete: None,
        }
    }

    fn decl(&self, w: &World) -> String {
        match self.on_delete {
            Some(a) => format!("{}: {} on delete {a}", self.name, self.ty.name(w)),
            None => format!("{}: {}", self.name, self.ty.name(w)),
        }
    }
}

/// The write state of a constructor relation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum State {
    Declared,
    Enabled,
    Deprecated,
}

#[derive(Clone, Debug)]
pub(super) struct Ctor {
    pub(super) name: String,
    pub(super) fields: Vec<Field>,
    pub(super) state: State,
}

impl Ctor {
    /// Returns the constructor's name and fields as a type declaration writes them.
    pub(super) fn decl(&self, w: &World) -> String {
        if self.fields.is_empty() {
            return self.name.clone();
        }
        let fs: Vec<String> = self.fields.iter().map(|f| f.decl(w)).collect();
        format!("{} {{ {} }}", self.name, fs.join(", "))
    }
}

/// A sum a schema walk declares as a field type.
#[derive(Clone, Debug)]
pub(super) struct Sum {
    pub(super) name: String,
    pub(super) ctors: Vec<Ctor>,
}

impl Sum {
    pub(super) fn decl(&self, w: &World) -> String {
        let cs: Vec<String> = self.ctors.iter().map(|c| c.decl(w)).collect();
        format!("type {} = {}", self.name, cs.join(" | "))
    }
}

/// A relation of a type of its own: a record, its one constructor named as the type and its
/// fields all header fields, or a sum of at least two constructors.
#[derive(Clone, Debug)]
pub(super) struct Rel {
    pub(super) name: String,
    pub(super) ty: String,
    pub(super) sum: bool,
    /// The header fields after the key `id`.
    pub(super) header: Vec<Field>,
    pub(super) ctors: Vec<Ctor>,
    pub(super) indexes: Vec<Index>,
    pub(super) uniques: Vec<Index>,
    /// Each default with the literal it gives.
    pub(super) defaults: Vec<(Index, String)>,
    /// Each check by name with the integer header field it bounds below by [`INT_MIN`].
    pub(super) checks: Vec<(String, String)>,
}

/// An index of a relation over one field, of a constructor or of the header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Index {
    pub(super) field: String,
    pub(super) ctor: Option<usize>,
}

/// A value a row holds in a field: a literal, or a value of a sum a schema walk declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Val {
    Lit(String),
    Ctor(String, Vec<(String, Val)>),
}

impl fmt::Display for Val {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Val::Lit(s) => f.write_str(s),
            Val::Ctor(c, fs) if fs.is_empty() => f.write_str(c),
            Val::Ctor(c, fs) => {
                let fs: Vec<String> = fs.iter().map(|(n, v)| format!("{n}: {v}")).collect();
                write!(f, "{c} {{ {} }}", fs.join(", "))
            }
        }
    }
}

/// A row the load writes: its relation and constructor, its key and each field's value.
#[derive(Clone, Debug)]
pub(super) struct Row {
    pub(super) rel: usize,
    pub(super) ctor: usize,
    pub(super) key: u64,
    pub(super) fields: Vec<(String, Val)>,
}

#[derive(Clone, Debug)]
pub(super) struct World {
    pub(super) rels: Vec<Rel>,
    pub(super) sums: Vec<Sum>,
    /// The rows of the database, in the order they are written.
    pub(super) rows: Vec<Row>,
    /// The number of names drawn after the world, for fields and constructors a schema
    /// operator adds or renames.
    pub(super) names: usize,
    /// The index in [`SHADE`] of the body of `shade`.
    pub(super) shade: usize,
}

impl World {
    pub(super) fn draw(rng: &mut Rng) -> World {
        let mut w = World {
            rels: Vec::new(),
            sums: Vec::new(),
            rows: Vec::new(),
            names: 0,
            shade: 0,
        };
        for i in 0..2 + rng.below(2) as usize {
            let rel = Rel::draw(rng, i);
            w.rels.push(rel);
        }
        w
    }

    /// Returns the declarations of the world, each with the verb that adds it in a migration.
    pub(super) fn decls(&self) -> Vec<(&'static str, String)> {
        let mut out: Vec<(&str, String)> = vec![
            ("add", "type Shade = Red | Green | Blue { n: I64 }".into()),
            ("define", self.shade_decl()),
            ("define", "fn opt (o: Option I64) -> Option I64 = o".into()),
        ];
        out.extend(self.sums.iter().map(|s| ("add", s.decl(self))));
        for r in &self.rels {
            out.push(("add", r.type_decl(self)));
            out.push(("add", r.relation_decl()));
        }
        for r in &self.rels {
            out.extend(r.checks.iter().map(|(k, f)| ("add", r.check_decl(k, f))));
        }
        out
    }

    pub(super) fn shade_decl(&self) -> String {
        format!("fn shade (s: Shade) -> Shade = {}", SHADE[self.shade])
    }

    /// Returns a name no field or constructor of the world holds, starting with `prefix`.
    pub(super) fn fresh(&mut self, prefix: &str) -> String {
        self.names += 1;
        format!("{prefix}x{}", self.names)
    }

    /// Returns the expression of key `n` of relation `r`.
    pub(super) fn key(&self, r: usize, n: u64) -> String {
        format!("{}.key({n})", self.rels[r].name)
    }

    /// Returns the greatest key of each relation's rows.
    fn keys(&self) -> Vec<u64> {
        let mut keys = vec![0; self.rels.len()];
        for r in &self.rows {
            keys[r.rel] = keys[r.rel].max(r.key);
        }
        keys
    }

    /// Returns the literals the rows hold in field `field` of relation `rel`.
    pub(super) fn seen(&self, rel: usize, field: &str) -> Vec<String> {
        self.rows
            .iter()
            .filter(|r| r.rel == rel)
            .flat_map(|r| r.fields.iter().filter(|(f, _)| f == field))
            .map(|(_, v)| v.to_string())
            .collect()
    }

    /// Returns the literals the rows of relation `rel` hold in field `field` within constructor
    /// `ctor`, or within every constructor when `ctor` is `None`.
    pub(super) fn seen_in(&self, rel: usize, ctor: Option<usize>, field: &str) -> Vec<String> {
        self.rows
            .iter()
            .filter(|r| r.rel == rel && ctor.is_none_or(|c| r.ctor == c))
            .flat_map(|r| r.fields.iter().filter(|(f, _)| f == field))
            .map(|(_, v)| v.to_string())
            .collect()
    }

    /// Returns whether constructor `c` of relation `rel` holds a row.
    pub(super) fn holds_rows(&self, rel: usize, c: usize) -> bool {
        self.rows.iter().any(|r| r.rel == rel && r.ctor == c)
    }

    /// Returns the writes inserting rows of every constructor, each a key of the relation its
    /// key fields name.
    pub(super) fn load(&mut self, rng: &mut Rng) -> Vec<String> {
        let start = self.rows.len();
        for i in 0..self.rels.len() {
            let keys = self.keys();
            let r = &self.rels[i];
            let refs_empty = r
                .header
                .iter()
                .any(|f| matches!(f.ty, T::Key(j) if keys[j] == 0));
            for c in 0..r.ctors.len() {
                let rows = if refs_empty { 0 } else { rng.below(5) };
                for _ in 0..rows {
                    if let Some(row) = self.row(rng, i, c) {
                        self.rows.push(row);
                    }
                }
            }
        }
        self.rows[start..].iter().map(|r| self.insert(r)).collect()
    }

    /// Returns up to `n` writes of new rows of enabled constructors drawn at random, each a key
    /// of the relation its key fields name and no value a unique holds already.
    pub(super) fn write(&mut self, rng: &mut Rng, n: u64) -> Vec<String> {
        let start = self.rows.len();
        for _ in 0..n {
            let keys = self.keys();
            let i = rng.below(self.rels.len() as u64) as usize;
            let r = &self.rels[i];
            let fields = r
                .header
                .iter()
                .chain(r.ctors.iter().flat_map(|c| &c.fields));
            if fields
                .into_iter()
                .any(|f| matches!(f.ty, T::Key(j) if keys[j] == 0))
            {
                continue;
            }
            let open: Vec<usize> = (0..r.ctors.len())
                .filter(|c| r.ctors[*c].state == State::Enabled)
                .collect();
            let Some(&c) = rng.pick_some(&open) else {
                continue;
            };
            if let Some(row) = self.row(rng, i, c) {
                self.rows.push(row);
            }
        }
        self.rows[start..].iter().map(|r| self.insert(r)).collect()
    }

    pub(super) fn inserts(&self) -> Vec<String> {
        self.rows.iter().map(|r| self.insert(r)).collect()
    }

    /// Returns a new row of constructor `c` of relation `i`, its key the next of the relation,
    /// or `None` when no draw of a uniquely constrained field is free.
    fn row(&self, rng: &mut Rng, i: usize, c: usize) -> Option<Row> {
        let keys = self.keys();
        let r = &self.rels[i];
        let mut fields = Vec::new();
        for f in r.header.iter().chain(&r.ctors[c].fields) {
            let taken = self.taken(i, c, &f.name);
            let mut v = self.val(rng, &f.ty, &keys);
            for _ in 0..8 {
                if !taken.contains(&v.to_string()) {
                    break;
                }
                v = self.val(rng, &f.ty, &keys);
            }
            if taken.contains(&v.to_string()) {
                return None;
            }
            fields.push((f.name.clone(), v));
        }
        Some(Row {
            rel: i,
            ctor: c,
            key: keys[i] + 1,
            fields,
        })
    }

    /// Returns the literals a new row of constructor `c` of relation `i` may not hold in field
    /// `f`, since a unique over it holds them.
    fn taken(&self, i: usize, c: usize, f: &str) -> Vec<String> {
        let scopes: Vec<Option<usize>> = self.rels[i]
            .uniques
            .iter()
            .filter(|u| u.field == f && u.ctor.is_none_or(|u| u == c))
            .map(|u| u.ctor)
            .collect();
        scopes
            .into_iter()
            .flat_map(|s| self.seen_in(i, s, f))
            .collect()
    }

    /// Returns a value of field type `t`, a key drawing from the first `keys[j]` keys of
    /// relation `j`.
    pub(super) fn val(&self, rng: &mut Rng, t: &T, keys: &[u64]) -> Val {
        let T::Named(s) = t else {
            return Val::Lit(value(rng, t, self, keys));
        };
        let c = rng.pick(&self.sums[*s].ctors);
        let fs = c
            .fields
            .iter()
            .map(|f| (f.name.clone(), self.val(rng, &f.ty, keys)))
            .collect();
        Val::Ctor(c.name.clone(), fs)
    }

    fn insert(&self, row: &Row) -> String {
        let r = &self.rels[row.rel];
        let mut fields = vec![format!("id: {}", self.key(row.rel, row.key))];
        fields.extend(row.fields.iter().map(|(f, v)| format!("{f}: {v}")));
        format!(
            "insert {} {{ {} }}",
            r.target(&r.ctors[row.ctor]),
            fields.join(", ")
        )
    }

    /// Replaces constructors `sources` of relation `r` by `products`, which take the position
    /// of the first source in the declaration order, and routes each row of a source through
    /// `route` to the index of its product among `products` and its fields.
    ///
    /// Preconditions:
    /// - no index, unique or default names a source.
    pub(super) fn replace(
        &mut self,
        r: usize,
        sources: &[usize],
        products: Vec<Ctor>,
        mut route: impl FnMut(&Row) -> (usize, Vec<(String, Val)>),
    ) {
        let at = *sources.iter().min().expect("a replacement has a source");
        let rel = &mut self.rels[r];
        let old = std::mem::take(&mut rel.ctors);
        let mut map = vec![None; old.len()];
        let mut first = 0;
        let mut products = Some(products);
        for (k, c) in old.into_iter().enumerate() {
            if k == at {
                first = rel.ctors.len();
                rel.ctors.extend(products.take().unwrap_or_default());
            }
            if !sources.contains(&k) {
                map[k] = Some(rel.ctors.len());
                rel.ctors.push(c);
            }
        }
        let moved = |k: usize| map[k].expect("no index, unique or default names a source");
        for i in rel.indexes.iter_mut().chain(rel.uniques.iter_mut()) {
            i.ctor = i.ctor.map(moved);
        }
        for (i, _) in &mut rel.defaults {
            i.ctor = i.ctor.map(moved);
        }
        for row in self.rows.iter_mut().filter(|row| row.rel == r) {
            if sources.contains(&row.ctor) {
                let (p, fields) = route(row);
                row.ctor = first + p;
                row.fields = fields;
            } else {
                row.ctor = map[row.ctor].expect("a row of a kept constructor keeps it");
            }
        }
    }

    /// Returns the bound two sums of floats of this case agree within: the bound of an
    /// aggregate over `n` values `x_i`, `n * eps * sum(|x_i|)`, at the most rows a relation holds
    /// and the greatest float literal.
    pub(super) fn float_bound(&self) -> f64 {
        let mut rows = vec![0; self.rels.len()];
        for r in &self.rows {
            rows[r.rel] += 1;
        }
        let n = f64::from(rows.into_iter().max().unwrap_or(0));
        n * f64::EPSILON * n * FLOAT_MAX
    }
}

impl Rel {
    pub(super) fn draw(rng: &mut Rng, i: usize) -> Rel {
        let mut next = 0;
        let mut field = |rng: &mut Rng, refs: bool| {
            next += 1;
            let mut tys = vec![
                T::Int,
                T::Int,
                T::Text,
                T::Bool,
                T::Shade,
                T::OptInt,
                T::Float,
            ];
            if refs {
                tys.extend((0..i).map(T::Key));
            }
            Field::new(format!("f{next}"), rng.pick(&tys).clone())
        };
        let header: Vec<Field> = (0..=rng.below(2)).map(|_| field(rng, true)).collect();
        let sum = rng.below(3) != 0;
        let ty = format!("T{i}");
        let ctors = if sum {
            (0..2 + rng.below(2) as u8)
                .map(|k| Ctor {
                    name: format!("C{i}{}", char::from(b'a' + k)),
                    fields: (0..rng.below(3)).map(|_| field(rng, false)).collect(),
                    state: State::Enabled,
                })
                .collect()
        } else {
            vec![Ctor {
                name: ty.clone(),
                fields: Vec::new(),
                state: State::Enabled,
            }]
        };
        let indexable = |f: &Field| matches!(f.ty, T::Int | T::Text | T::Key(_));
        let mut indexes = Vec::new();
        for f in header.iter().filter(|f| indexable(f)) {
            if rng.below(2) == 0 {
                indexes.push(Index {
                    field: f.name.clone(),
                    ctor: None,
                });
            }
        }
        for (k, c) in ctors.iter().enumerate().filter(|_| sum) {
            for f in c.fields.iter().filter(|f| indexable(f)) {
                if rng.below(3) == 0 {
                    indexes.push(Index {
                        field: f.name.clone(),
                        ctor: Some(k),
                    });
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
            uniques: Vec::new(),
            defaults: Vec::new(),
            checks: Vec::new(),
        }
    }

    pub(super) fn type_decl(&self, w: &World) -> String {
        let fields = |id: bool, fs: &[Field]| {
            let mut out: Vec<String> = Vec::new();
            if id {
                out.push(format!("id: Key {}", self.name));
            }
            out.extend(fs.iter().map(|f| f.decl(w)));
            format!("{{ {} }}", out.join(", "))
        };
        if !self.sum {
            return format!("type {} = {}", self.ty, fields(true, &self.header));
        }
        let mut out = format!("type {} {}", self.ty, fields(true, &self.header));
        for (k, c) in self.ctors.iter().enumerate() {
            out.push_str(if k == 0 { "\n  = " } else { "\n  | " });
            out.push_str(&c.decl(w));
        }
        out
    }

    pub(super) fn relation_decl(&self) -> String {
        let mut out = format!(
            "relation {} : {} key id generate sequence",
            self.name, self.ty
        );
        let scoped = |i: &Index| match i.ctor {
            Some(c) => format!("({}) in {}", i.field, self.ctors[c].name),
            None => format!("({})", i.field),
        };
        for i in &self.indexes {
            out.push_str(&format!("\n  index {}", scoped(i)));
        }
        for u in &self.uniques {
            out.push_str(&format!("\n  unique {}", scoped(u)));
        }
        for (d, v) in &self.defaults {
            out.push_str(&format!("\n  default {} = {v}", self.default_target(d)));
        }
        out
    }

    /// Returns the field a default names within the relation: `C.f`, or `f` in the header.
    pub(super) fn default_target(&self, d: &Index) -> String {
        match d.ctor {
            Some(c) => format!("{}.{}", self.ctors[c].name, d.field),
            None => d.field.clone(),
        }
    }

    /// Returns the declaration of check `k` over integer header field `f`.
    pub(super) fn check_decl(&self, k: &str, f: &str) -> String {
        format!("check {k} on {} ({f} >= {INT_MIN})", self.name)
    }

    /// Returns the constructor set an operator names index `i` over.
    pub(super) fn index_target(&self, i: &Index) -> String {
        match i.ctor {
            Some(c) => format!("{}.{}", self.name, self.ctors[c].name),
            None => self.name.clone(),
        }
    }

    /// Returns whether an index, unique, default or check names field `f` of constructor `c`,
    /// or of the header when `c` is `None`.
    pub(super) fn attached(&self, c: Option<usize>, f: &str) -> bool {
        let names = |i: &Index| i.field == f && (i.ctor == c || i.ctor.is_none());
        self.indexes.iter().chain(&self.uniques).any(names)
            || self.defaults.iter().any(|(d, _)| names(d))
            || self.checks.iter().any(|(_, g)| g == f)
    }

    /// Returns whether an index, unique or default ranges over constructor `c` alone.
    pub(super) fn scopes(&self, c: usize) -> bool {
        let names = |i: &Index| i.ctor == Some(c);
        self.indexes.iter().chain(&self.uniques).any(names)
            || self.defaults.iter().any(|(d, _)| names(d))
    }

    /// Returns the relation expression of the rows of constructor `c`.
    pub(super) fn target(&self, c: &Ctor) -> String {
        if self.sum {
            format!("{}.{}", self.name, c.name)
        } else {
            self.name.clone()
        }
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

/// Returns a literal of scalar field type `t`, a key drawing from the first `keys[j]` keys of
/// relation `j`.
pub(super) fn value(rng: &mut Rng, t: &T, w: &World, keys: &[u64]) -> String {
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
        T::Float => rng.pick(&FLOATS).to_string(),
        T::Key(j) => w.key(*j, 1 + rng.below(keys[*j])),
        T::Named(_) | T::FloatSum | T::Row(..) | T::Rec(_) | T::Set(_) => {
            unreachable!("a literal is of a scalar or a key")
        }
    }
}
