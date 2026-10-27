use crate::Rng;

/// The declarations of every case before its relations, each with the verb that adds it in a
/// migration. A literal of a sum type passes through a function of that type, so its type is the
/// whole sum and not its one constructor.
const PRELUDE: [(&str, &str); 3] = [
    ("add", "type Shade = Red | Green | Blue { n: I64 }"),
    ("define", "fn shade (s: Shade) -> Shade = s"),
    ("define", "fn opt (o: Option I64) -> Option I64 = o"),
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
    /// A row of the relation of this index, of one of these constructors.
    Row(usize, Vec<usize>),
    Rec(Vec<(String, T)>),
    Set(Box<T>),
}

impl T {
    pub(super) fn name(&self) -> String {
        match self {
            T::Int => "I64".into(),
            T::Text => "Text".into(),
            T::Bool => "Bool".into(),
            T::Shade => "Shade".into(),
            T::OptInt => "Option I64".into(),
            T::Float => "F64".into(),
            T::Key(r) => format!("Key r{r}"),
            T::FloatSum | T::Row(..) | T::Rec(_) | T::Set(_) => {
                unreachable!("a field type is a scalar or a key")
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
}

#[derive(Clone, Debug)]
pub(super) struct Ctor {
    pub(super) name: String,
    pub(super) fields: Vec<Field>,
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
}

/// An index of a relation over one field, of a constructor or of the header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Index {
    pub(super) field: String,
    pub(super) ctor: Option<usize>,
}

/// A row the load writes: its relation and constructor, its key and each field's literal.
#[derive(Clone, Debug)]
pub(super) struct Row {
    pub(super) rel: usize,
    pub(super) ctor: usize,
    pub(super) key: u64,
    pub(super) fields: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
pub(super) struct World {
    pub(super) rels: Vec<Rel>,
    /// The rows of the database, in the order they are written.
    pub(super) rows: Vec<Row>,
    /// The number of names drawn after the world, for fields and constructors a schema
    /// operator adds or renames.
    pub(super) names: usize,
}

impl World {
    pub(super) fn draw(rng: &mut Rng) -> World {
        let n = 2 + rng.below(2) as usize;
        World {
            rels: (0..n).map(|i| Rel::draw(rng, i)).collect(),
            rows: Vec::new(),
            names: 0,
        }
    }

    pub(super) fn decls(&self) -> Vec<(&'static str, String)> {
        let mut out: Vec<(&str, String)> =
            PRELUDE.iter().map(|(v, d)| (*v, (*d).to_owned())).collect();
        for r in &self.rels {
            out.push(("add", r.type_decl()));
            out.push(("add", r.relation_decl()));
        }
        out
    }

    /// Returns a name no field or constructor of the world holds, starting with `prefix`.
    pub(super) fn fresh(&mut self, prefix: &str) -> String {
        self.names += 1;
        format!("{prefix}x{}", self.names)
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
            .map(|(_, v)| v.clone())
            .collect()
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
                    let row = self.row(rng, i, c);
                    self.rows.push(row);
                }
            }
        }
        self.rows[start..].iter().map(|r| self.insert(r)).collect()
    }

    /// Returns up to `n` writes of new rows of constructors drawn at random, each a key of the
    /// relation its key fields name.
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
            let c = rng.below(r.ctors.len() as u64) as usize;
            let row = self.row(rng, i, c);
            self.rows.push(row);
        }
        self.rows[start..].iter().map(|r| self.insert(r)).collect()
    }

    pub(super) fn inserts(&self) -> Vec<String> {
        self.rows.iter().map(|r| self.insert(r)).collect()
    }

    /// Returns a new row of constructor `c` of relation `i`, its key the next of the relation.
    fn row(&self, rng: &mut Rng, i: usize, c: usize) -> Row {
        let keys = self.keys();
        let r = &self.rels[i];
        let fields = r
            .header
            .iter()
            .chain(&r.ctors[c].fields)
            .map(|f| (f.name.clone(), value(rng, &f.ty, &keys)))
            .collect();
        Row {
            rel: i,
            ctor: c,
            key: keys[i] + 1,
            fields,
        }
    }

    fn insert(&self, row: &Row) -> String {
        let r = &self.rels[row.rel];
        let mut fields = vec![format!("id: {}.key({})", r.name, row.key)];
        fields.extend(row.fields.iter().map(|(f, v)| format!("{f}: {v}")));
        format!(
            "insert {} {{ {} }}",
            r.target(&r.ctors[row.ctor]),
            fields.join(", ")
        )
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
        }
    }

    pub(super) fn type_decl(&self) -> String {
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

    pub(super) fn relation_decl(&self) -> String {
        let mut out = format!(
            "relation {} : {} key id generate sequence",
            self.name, self.ty
        );
        for i in &self.indexes {
            out.push_str(&format!("\n  index ({})", i.field));
            if let Some(c) = i.ctor {
                out.push_str(&format!(" in {}", self.ctors[c].name));
            }
        }
        out
    }

    /// Returns the constructor set an operator names index `i` over.
    pub(super) fn index_target(&self, i: &Index) -> String {
        match i.ctor {
            Some(c) => format!("{}.{}", self.name, self.ctors[c].name),
            None => self.name.clone(),
        }
    }

    /// Returns the relation expression of the rows of constructor `c`.
    fn target(&self, c: &Ctor) -> String {
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

/// Returns a literal of field type `t`, a key drawing from the first `keys[j]` keys of
/// relation `j`.
pub(super) fn value(rng: &mut Rng, t: &T, keys: &[u64]) -> String {
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
        T::Key(j) => format!("r{j}.key({})", 1 + rng.below(keys[*j])),
        T::FloatSum | T::Row(..) | T::Rec(_) | T::Set(_) => {
            unreachable!("a field type is a scalar or a key")
        }
    }
}
