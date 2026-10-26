//! Programs for fuzzing the front end: drawn from a grammar of well-typed statements and schemas,
//! and mutated at the token level, each a function of its seed alone.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use adtb_lang::check::{Program, check_adhoc, check_schema};
use adtb_lang::norm;
use adtb_lang::parse::{
    Options, parse_adhoc, parse_bundle, parse_example, parse_expr, parse_migration, parse_schema,
    parse_type,
};
use adtb_lang::schema::{Schema, build};
use adtb_lang::span::FileId;

use crate::Rng;

/// The schema every drawn ad hoc statement ranges over.
pub const SCHEMA: &str = r#"module app
type Status = Draft | Sent { at: I64 } | Paid { at: I64, by: Text }
type Item = { id: Key items, name: Text, n: I64, status: Status }
type Tagged { id: Key tags, item: Key items } = Small { size: I64 } | Large { label: Text }
relation items : Item key id generate sequence
relation tags : Tagged key id generate sequence
fn twice (x: I64) -> I64 = x * 2
fn label (s: Status) -> Text = match s | Draft -> "draft" | Sent _ -> "sent" | Paid p -> p.by
fn as_status (s: Status) -> Status = s
fn as_opt (o: Option I64) -> Option I64 = o
"#;

/// Which front-end entry points a case drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// Ad hoc text over [`SCHEMA`].
    Adhoc,
    /// A schema file.
    Schema,
    /// Text of no known kind, given to every parser.
    Any,
}

/// One input to the front end.
#[derive(Clone, Debug)]
pub struct Case {
    pub kind: Kind,
    /// Whether the text was mutated after it was drawn.
    pub mutated: bool,
    pub text: String,
}

/// How far a case got through the front end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reached {
    Refused,
    Parsed,
    Checked,
    Normalised,
}

/// The front end over [`SCHEMA`], built once and shared by every case.
#[derive(Debug)]
pub struct Front {
    schema: Schema,
    program: Program,
}

impl Default for Front {
    fn default() -> Front {
        Front::new()
    }
}

impl Front {
    pub fn new() -> Front {
        let file = parse_schema(SCHEMA, Options::default())
            .ok()
            .unwrap_or_else(|ds| panic!("the fuzz schema does not parse: {ds:?}"));
        let schema = build(&[(FileId(0), file)])
            .unwrap_or_else(|ds| panic!("the fuzz schema does not build: {ds:?}"));
        let program = check_schema(&schema)
            .unwrap_or_else(|ds| panic!("the fuzz schema does not check: {ds:?}"));
        Front { schema, program }
    }

    /// Runs `case` through parse, check and normalise as far as it is accepted.
    pub fn run(&self, case: &Case) -> Reached {
        match case.kind {
            Kind::Adhoc => self.adhoc(&case.text),
            Kind::Schema => schema(&case.text),
            Kind::Any => {
                let opts = Options::default();
                let other = [
                    parse_migration(&case.text, opts).ok().is_ok(),
                    parse_bundle(&case.text, opts).ok().is_ok(),
                    parse_example(&case.text, Options { elisions: true })
                        .ok()
                        .is_ok(),
                    parse_expr(&case.text, opts).ok().is_ok(),
                    parse_type(&case.text, opts).ok().is_ok(),
                ]
                .contains(&true);
                let reached = self.adhoc(&case.text).max(schema(&case.text));
                if other {
                    reached.max(Reached::Parsed)
                } else {
                    reached
                }
            }
        }
    }

    fn adhoc(&self, text: &str) -> Reached {
        let Ok(items) = parse_adhoc(text, Options::default()).ok() else {
            return Reached::Refused;
        };
        let Ok(adhoc) = check_adhoc(&self.schema, &self.program, &items, &BTreeMap::new()) else {
            return Reached::Parsed;
        };
        match norm::adhoc(&self.schema, &self.program, &adhoc) {
            Ok(_) => Reached::Normalised,
            Err(_) => Reached::Checked,
        }
    }
}

fn schema(text: &str) -> Reached {
    let Ok(file) = parse_schema(text, Options::default()).ok() else {
        return Reached::Refused;
    };
    let Ok(schema) = build(&[(FileId(0), file)]) else {
        return Reached::Parsed;
    };
    let Ok(program) = check_schema(&schema) else {
        return Reached::Parsed;
    };
    match norm::program(&schema, &program) {
        Ok(_) => Reached::Normalised,
        Err(_) => Reached::Checked,
    }
}

/// Returns the text of every `adt` block in the markdown files of `dir`, in file name order.
pub fn spec_corpus(dir: &Path) -> Vec<String> {
    let mut paths: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.expect("a directory entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let src = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut block: Option<String> = None;
        for line in src.lines() {
            match (line.trim_start().strip_prefix("```"), &mut block) {
                (Some(_), Some(text)) => {
                    out.push(std::mem::take(text));
                    block = None;
                }
                (Some(info), None) if info.trim().starts_with("adt") => block = Some(String::new()),
                (None, Some(text)) => {
                    text.push_str(line);
                    text.push('\n');
                }
                (Some(_) | None, None) => {}
            }
        }
    }
    out
}

/// Returns the case of `seed`, drawing mutation material from `corpus`.
pub fn case(seed: u64, corpus: &[String]) -> Case {
    let mut rng = Rng::new(seed);
    let (kind, text) = match rng.below(4) {
        0 | 1 => (Kind::Adhoc, adhoc(&mut rng)),
        2 => (Kind::Schema, schema_text(&mut rng)),
        _ if corpus.is_empty() => (Kind::Adhoc, adhoc(&mut rng)),
        _ => (Kind::Any, rng.pick(corpus).clone()),
    };
    if kind != Kind::Any && rng.below(2) == 0 {
        return Case {
            kind,
            mutated: false,
            text,
        };
    }
    let donor = if corpus.is_empty() {
        adhoc(&mut rng)
    } else {
        rng.pick(corpus).clone()
    };
    let text = mutate(&mut rng, &text, &donor);
    Case {
        kind,
        mutated: true,
        text,
    }
}

/// The types a drawn expression may have.
#[derive(Clone, Debug, PartialEq, Eq)]
enum G {
    Int,
    Text,
    Bool,
    Status,
    OptInt,
    /// A row of the relation with this index in the world.
    Row(usize),
    Set(Box<G>),
}

/// The relations an expression may range over and the scalar fields of their rows.
struct World {
    rels: Vec<(String, Vec<(String, G)>)>,
    fns: Vec<(String, Vec<G>, G)>,
}

impl World {
    fn fixed() -> World {
        World {
            rels: vec![(
                "items".into(),
                vec![
                    ("name".into(), G::Text),
                    ("n".into(), G::Int),
                    ("status".into(), G::Status),
                ],
            )],
            fns: vec![
                ("twice".into(), vec![G::Int], G::Int),
                ("label".into(), vec![G::Status], G::Text),
            ],
        }
    }
}

/// Draws expressions of one world, naming binders freshly.
struct Gen<'w> {
    world: &'w World,
    env: Vec<(String, G)>,
    next: usize,
}

impl Gen<'_> {
    fn fresh(&mut self) -> String {
        self.next += 1;
        format!("v{}", self.next)
    }

    fn var_of(&self, rng: &mut Rng, g: &G) -> Option<String> {
        let vs: Vec<&String> = self
            .env
            .iter()
            .filter(|(_, t)| t == g)
            .map(|(n, _)| n)
            .collect();
        (!vs.is_empty()).then(|| rng.pick(&vs).to_string())
    }

    /// Returns a field access of type `g` on a row in scope.
    fn field_of(&self, rng: &mut Rng, g: &G) -> Option<String> {
        let mut out = Vec::new();
        for (n, t) in &self.env {
            if let G::Row(r) = t {
                for (f, ft) in &self.world.rels[*r].1 {
                    if ft == g {
                        out.push(format!("{n}.{f}"));
                    }
                }
            }
        }
        (!out.is_empty()).then(|| rng.pick(&out).clone())
    }

    fn with<T>(&mut self, name: &str, g: G, f: impl FnOnce(&mut Self) -> T) -> T {
        self.env.push((name.to_string(), g));
        let r = f(self);
        self.env.pop();
        r
    }

    fn expr(&mut self, rng: &mut Rng, g: &G, depth: u32) -> String {
        if depth == 0 || rng.below(4) == 0 {
            if let Some(v) = self.var_of(rng, g)
                && rng.below(2) == 0
            {
                return v;
            }
            if let Some(f) = self.field_of(rng, g)
                && rng.below(2) == 0
            {
                return f;
            }
            return self.leaf(rng, g);
        }
        let d = depth - 1;
        match g {
            G::Int => self.int(rng, d),
            G::Text => self.text(rng, d),
            G::Bool => self.boolean(rng, d),
            G::Status => match rng.below(4) {
                0 => "as_status(Draft)".into(),
                1 => format!("as_status(Sent {{ at: {} }})", self.expr(rng, &G::Int, d)),
                2 => format!(
                    "as_status(Paid {{ at: {}, by: {} }})",
                    self.expr(rng, &G::Int, d),
                    self.expr(rng, &G::Text, d)
                ),
                _ => self.conditional(rng, g, d),
            },
            G::OptInt => match rng.below(3) {
                0 => "as_opt(None)".into(),
                1 => format!("as_opt(Some {{ value: {} }})", self.expr(rng, &G::Int, d)),
                _ => self.conditional(rng, g, d),
            },
            G::Row(_) => self.leaf(rng, g),
            G::Set(elem) => self.set(rng, elem, d),
        }
    }

    fn leaf(&mut self, rng: &mut Rng, g: &G) -> String {
        match g {
            G::Int => rng
                .pick(&["0", "1", "-1", "7", "-42", "9223372036854775807", "1000000"])
                .to_string(),
            G::Text => rng
                .pick(&[
                    r#""""#,
                    r#""a""#,
                    r#""tab\there""#,
                    r#""quote \" and \\""#,
                    r#""é😀""#,
                ])
                .to_string(),
            G::Bool => rng.pick(&["true", "false"]).to_string(),
            G::Status => "as_status(Draft)".into(),
            G::OptInt => "as_opt(None)".into(),
            G::Row(r) => match self.var_of(rng, g) {
                Some(v) => v,
                None => format!(
                    "(top 1 by (r.n) (from r in {} select r))",
                    self.world.rels[*r].0
                ),
            },
            G::Set(elem) => match elem.as_ref() {
                G::Row(r) => self.world.rels[*r].0.clone(),
                e => format!("[{}]", self.leaf(rng, e)),
            },
        }
    }

    fn conditional(&mut self, rng: &mut Rng, g: &G, d: u32) -> String {
        format!(
            "(if {} then {} else {})",
            self.expr(rng, &G::Bool, d),
            self.expr(rng, g, d),
            self.expr(rng, g, d)
        )
    }

    fn int(&mut self, rng: &mut Rng, d: u32) -> String {
        match rng.below(12) {
            0..=2 => format!(
                "({} {} {})",
                self.expr(rng, &G::Int, d),
                rng.pick(&["+", "-", "*", "/", "%"]),
                self.expr(rng, &G::Int, d)
            ),
            3 => format!("-({})", self.expr(rng, &G::Int, d)),
            4 => self.call(rng, &G::Int, d),
            5 => {
                let (rel, y) = (self.rel(rng), self.fresh());
                let r = self.rel_index(&rel);
                let filter = self.with(&y, G::Row(r), |s| s.expr(rng, &G::Bool, d));
                format!("count({y} for {y} in {rel} where {filter})")
            }
            6 => {
                let (rel, y) = (self.rel(rng), self.fresh());
                let r = self.rel_index(&rel);
                let body = self.with(&y, G::Row(r), |s| s.expr(rng, &G::Int, d));
                format!("{}({body} for {y} in {rel})", rng.pick(&["sum", "count"]))
            }
            7 => {
                let s = self.expr(rng, &G::Status, d);
                let (a, b, c) = (self.fresh(), self.fresh(), self.expr(rng, &G::Int, d));
                format!("(match {s} | Draft -> {c} | Sent {a} -> {a}.at | Paid {b} -> {b}.at)")
            }
            8 => {
                let o = self.expr(rng, &G::OptInt, d);
                let (v, a) = (self.fresh(), self.expr(rng, &G::Int, d));
                format!("(match {o} | None -> {a} | Some {v} -> {v}.value)")
            }
            9 => format!("scalar_len({})", self.expr(rng, &G::Text, d)),
            _ => self.conditional(rng, &G::Int, d),
        }
    }

    fn text(&mut self, rng: &mut Rng, d: u32) -> String {
        match rng.below(4) {
            0 => format!(
                "({} ++ {})",
                self.expr(rng, &G::Text, d),
                self.expr(rng, &G::Text, d)
            ),
            1 => self.call(rng, &G::Text, d),
            2 => self.conditional(rng, &G::Text, d),
            _ => self.leaf(rng, &G::Text),
        }
    }

    fn boolean(&mut self, rng: &mut Rng, d: u32) -> String {
        match rng.below(8) {
            0 | 1 => format!(
                "({} {} {})",
                self.expr(rng, &G::Int, d),
                rng.pick(&["<", "<=", ">", ">=", "==", "!="]),
                self.expr(rng, &G::Int, d)
            ),
            2 => format!(
                "({} == {})",
                self.expr(rng, &G::Text, d),
                self.expr(rng, &G::Text, d)
            ),
            3 => format!(
                "({} {} {})",
                self.expr(rng, &G::Bool, d),
                rng.pick(&["and", "or"]),
                self.expr(rng, &G::Bool, d)
            ),
            4 => format!("(not {})", self.expr(rng, &G::Bool, d)),
            5 => {
                let (rel, y) = (self.rel(rng), self.fresh());
                let r = self.rel_index(&rel);
                let filter = self.with(&y, G::Row(r), |s| s.expr(rng, &G::Bool, d));
                format!("(exists {y} in {rel} where {filter})")
            }
            6 => {
                let s = self.expr(rng, &G::Status, d);
                format!("(match {s} | Draft -> true | Sent _ -> false | Paid _ -> false)")
            }
            _ => self.conditional(rng, &G::Bool, d),
        }
    }

    fn call(&mut self, rng: &mut Rng, g: &G, d: u32) -> String {
        let fns: Vec<(String, Vec<G>)> = self
            .world
            .fns
            .iter()
            .filter(|(_, _, r)| r == g)
            .map(|(n, ps, _)| (n.clone(), ps.clone()))
            .collect();
        if fns.is_empty() {
            return self.leaf(rng, g);
        }
        let (name, params) = rng.pick(&fns).clone();
        let args: Vec<String> = params.iter().map(|p| self.expr(rng, p, d)).collect();
        format!("{name}({})", args.join(", "))
    }

    fn rel(&self, rng: &mut Rng) -> String {
        rng.pick(&self.world.rels).0.clone()
    }

    fn rel_index(&self, name: &str) -> usize {
        self.world
            .rels
            .iter()
            .position(|(n, _)| n == name)
            .expect("a relation of the world")
    }

    fn set(&mut self, rng: &mut Rng, elem: &G, d: u32) -> String {
        match rng.below(4) {
            0 => format!(
                "[{}]",
                (0..=rng.below(3))
                    .map(|_| self.expr(rng, elem, d))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            1 => format!(
                "({} union {})",
                self.expr(rng, &G::Set(Box::new(elem.clone())), d),
                self.expr(rng, &G::Set(Box::new(elem.clone())), d)
            ),
            _ => self.comprehension(rng, elem, d),
        }
    }

    fn comprehension(&mut self, rng: &mut Rng, elem: &G, d: u32) -> String {
        let (rel, y) = (self.rel(rng), self.fresh());
        let r = self.rel_index(&rel);
        self.with(&y, G::Row(r), |s| {
            let filter = if rng.below(2) == 0 {
                format!(" where {}", s.expr(rng, &G::Bool, d))
            } else {
                String::new()
            };
            let select = s.expr(rng, elem, d);
            format!("(from {y} in {rel}{filter} select {select})")
        })
    }
}

impl Gen<'_> {
    /// Returns a statement around one long flat list: a chain of `and`, `or` or `union`, the
    /// items of a `transact`, the arms of a `match` or the elements of a set literal.
    fn long(&mut self, rng: &mut Rng) -> String {
        let n = 2 + rng.below(1500) as usize;
        let x = self.fresh();
        match rng.below(6) {
            0 => {
                let op = rng.pick(&["and", "or"]).to_string();
                let terms: Vec<String> = self.with(&x, G::Row(0), |s| {
                    (0..n).map(|_| s.expr(rng, &G::Bool, 1)).collect()
                });
                format!(
                    "from {x} in items where {} select {x}",
                    terms.join(&format!(" {op} "))
                )
            }
            1 => {
                let int_set = G::Set(Box::new(G::Int));
                let sets: Vec<String> = (0..n).map(|_| self.expr(rng, &int_set, 1)).collect();
                sets.join(" union ")
            }
            2 => {
                let items: String = (0..n)
                    .map(|i| {
                        let name = self.expr(rng, &G::Text, 1);
                        let insert =
                            format!("insert items {{ name: {name}, n: {i}, status: Draft }}");
                        if rng.below(2) == 0 {
                            format!("  let {x}_{i} = {insert}\n")
                        } else {
                            format!("  {insert}\n")
                        }
                    })
                    .collect();
                format!("transact\n{items}")
            }
            3 => {
                let arms: String = self.with(&x, G::Row(0), |s| {
                    (0..n)
                        .map(|i| format!(" | {i} -> {}", s.expr(rng, &G::Int, 1)))
                        .collect()
                });
                format!("from {x} in items select match {x}.n{arms} | _ -> 0")
            }
            4 => {
                let arms: String = (0..n)
                    .map(|i| format!(" | {i} -> delete {x} in items where {x}.n == {i}"))
                    .collect();
                format!("transact\n  let n = count(y for y in items)\n  match n{arms} | _ -> skip")
            }
            _ => {
                let elems: Vec<String> = (0..n).map(|_| self.expr(rng, &G::Int, 1)).collect();
                format!("from {x} in [{}] select {x} + 1", elems.join(", "))
            }
        }
    }
}

fn scalar(rng: &mut Rng) -> G {
    rng.pick(&[G::Int, G::Text, G::Bool, G::Status, G::OptInt])
        .clone()
}

/// Returns the well-typed ad hoc statement of `seed` around one long flat list.
pub fn long_case(seed: u64) -> Case {
    let world = World::fixed();
    let mut g = Gen {
        world: &world,
        env: Vec::new(),
        next: 0,
    };
    Case {
        kind: Kind::Adhoc,
        mutated: false,
        text: g.long(&mut Rng::new(seed)),
    }
}

/// Returns a well-typed ad hoc statement over [`SCHEMA`].
fn adhoc(rng: &mut Rng) -> String {
    let world = World::fixed();
    let mut g = Gen {
        world: &world,
        env: Vec::new(),
        next: 0,
    };
    let depth = 1 + rng.below(4) as u32;
    match rng.below(12) {
        10 | 11 => g.long(rng),
        0 => {
            let t = scalar(rng);
            g.expr(rng, &t, depth)
        }
        1 | 2 => {
            let t = scalar(rng);
            let q = g.comprehension(rng, &t, depth);
            q[1..q.len() - 1].to_string()
        }
        3 => {
            let x = g.fresh();
            let (key, n) = g.with(&x, G::Row(0), |s| {
                (s.expr(rng, &G::Int, depth), s.expr(rng, &G::Int, depth))
            });
            format!(
                "from {x} in items where ({key} < {n}) select {x} order by ({x}.n desc, key {x}) take {}",
                rng.below(5)
            )
        }
        4 => {
            let (x, k) = (g.fresh(), g.fresh());
            let by = g.with(&x, G::Row(0), |s| s.expr(rng, &G::Int, depth));
            format!(
                "from {x} in items group {x} by {by} into {k} select {{ key: {k}.key, n: count(y for y in {k}) }}"
            )
        }
        5 => format!(
            "insert items {{ name: {}, n: {}, status: {} }}",
            g.expr(rng, &G::Text, depth),
            g.expr(rng, &G::Int, depth),
            g.expr(rng, &G::Status, depth)
        ),
        6 => {
            let x = g.fresh();
            let filter = g.with(&x, G::Row(0), |s| s.expr(rng, &G::Bool, depth));
            format!("delete {x} in items where {filter}")
        }
        7 => {
            let x = g.fresh();
            let (filter, n) = g.with(&x, G::Row(0), |s| {
                (s.expr(rng, &G::Bool, depth), s.expr(rng, &G::Int, depth))
            });
            format!("update {x} in items where {filter} set {{ {x} with n: {n} }}")
        }
        8 => {
            let a = g.fresh();
            let size = g.expr(rng, &G::Int, depth);
            let label = g.expr(rng, &G::Text, depth);
            let name = g.expr(rng, &G::Text, depth);
            format!(
                "transact\n  let {a} = insert items {{ name: {name}, n: 1, status: Draft }}\n  insert tags.Small {{ item: key {a}, size: {size} }}\n  insert tags.Large {{ item: key {a}, label: {label} }}\n  return key {a}"
            )
        }
        _ => {
            let (r, y, z) = (g.fresh(), g.fresh(), g.fresh());
            let bound = g.expr(rng, &G::Int, depth);
            format!(
                "fix {r} = (from x in items select x.n) union (from {y} in {r} where {y} < {bound} select {y} + 1)\nin from {z} in {r} select {z}"
            )
        }
    }
}

/// Returns a schema file of a few types and relations, with functions, constraints and checks
/// over them.
fn schema_text(rng: &mut Rng) -> String {
    let n = 1 + rng.below(3) as usize;
    let mut out = String::from(
        "module m\ntype Status = Draft | Sent { at: I64 } | Paid { at: I64, by: Text }\n\
         fn as_status (s: Status) -> Status = s\n\
         fn as_opt (o: Option I64) -> Option I64 = o\n",
    );
    let mut rels = Vec::new();
    for i in 0..n {
        let mut fields = vec![("a".to_string(), G::Int)];
        for (f, g) in [("b", G::Text), ("c", G::Bool), ("s", G::Status)] {
            if rng.below(2) == 0 {
                fields.push((f.to_string(), g));
            }
        }
        let mut decl: Vec<String> = vec![format!("id: Key r{i}")];
        decl.extend(fields.iter().map(|(f, g)| format!("{f}: {}", type_name(g))));
        if i > 0 && rng.below(2) == 0 {
            decl.push(format!("up: Key r{}", rng.below(i as u64)));
        }
        if rng.below(2) == 0 {
            out.push_str(&format!("type T{i} = {{ {} }}\n", decl.join(", ")));
        } else {
            out.push_str(&format!(
                "type T{i} {{ {} }} = A{i} {{ x: I64 }} | B{i} | C{i} {{ y: Text }}\n",
                decl.join(", ")
            ));
        }
        out.push_str(&format!("relation r{i} : T{i} key id generate sequence"));
        if rng.below(3) == 0 {
            out.push_str(" unique (a)");
        }
        if rng.below(3) == 0 {
            out.push_str(" index (a)");
        }
        out.push('\n');
        rels.push((format!("r{i}"), fields));
    }
    let world = World {
        rels,
        fns: Vec::new(),
    };
    let mut g = Gen {
        world: &world,
        env: Vec::new(),
        next: 0,
    };
    for k in 0..rng.below(3) {
        let ret = scalar(rng);
        let body = g.with("p", G::Int, |s| {
            s.with("q", G::Text, |s| s.expr(rng, &ret, 3))
        });
        out.push_str(&format!(
            "fn f{k} (p: I64, q: Text) -> {} = {body}\n",
            type_name(&ret)
        ));
    }
    for k in 0..rng.below(2) {
        let r = rng.below(n as u64) as usize;
        let x = g.fresh();
        let filter = g.with(&x, G::Row(r), |s| s.expr(rng, &G::Bool, 2));
        out.push_str(&format!(
            "constraint c{k} = from {x} in r{r} where {filter} select {x}\n"
        ));
    }
    for k in 0..rng.below(2) {
        let r = rng.below(n as u64) as usize;
        let fields = world.rels[r].1.clone();
        let cond = g.with_all(&fields, |s| s.expr(rng, &G::Bool, 2));
        out.push_str(&format!("check k{k} on r{r} ({cond})\n"));
    }
    out
}

impl Gen<'_> {
    fn with_all<T>(&mut self, binds: &[(String, G)], f: impl FnOnce(&mut Self) -> T) -> T {
        let n = self.env.len();
        self.env.extend(binds.iter().cloned());
        let r = f(self);
        self.env.truncate(n);
        r
    }
}

fn type_name(g: &G) -> String {
    match g {
        G::Int => "I64".into(),
        G::Text => "Text".into(),
        G::Bool => "Bool".into(),
        G::Status => "Status".into(),
        G::OptInt => "Option I64".into(),
        G::Row(_) => "Item".into(),
        G::Set(e) => format!("Set ({})", type_name(e)),
    }
}

const VOCABULARY: &[&str] = &[
    "from",
    "select",
    "where",
    "in",
    "match",
    "fix",
    "fn",
    "exists",
    "top",
    "if",
    "then",
    "else",
    "not",
    "and",
    "or",
    "union",
    "key",
    "try",
    "insert",
    "update",
    "delete",
    "set",
    "transact",
    "let",
    "return",
    "group",
    "by",
    "into",
    "order",
    "take",
    "after",
    "true",
    "false",
    "type",
    "relation",
    "module",
    "constraint",
    "check",
    "unique",
    "index",
    "default",
    "generate",
    "sequence",
    "ordered",
    "new",
    "with",
    "on",
    "skip",
    "fail",
    "(",
    ")",
    "[",
    "]",
    "{",
    "}",
    "|",
    "->",
    ",",
    ".",
    ":",
    "=",
    "==",
    "!=",
    "<",
    "<=",
    ">",
    ">=",
    "+",
    "-",
    "*",
    "/",
    "%",
    "++",
    ".<",
    "_",
    "...",
    "items",
    "tags",
    "x",
    "Draft",
    "Sent",
    "Paid",
    "Set",
    "Option",
    "Key",
    "Text",
    "I64",
    "Bool",
    "None",
    "Some",
    "count",
    "sum",
    "twice",
    "0",
    "-1",
    "9223372036854775808",
    "1.5",
    "2d",
    "\"s\"",
    "\"",
    "\\",
    "--",
    "\n",
    "é",
    "\u{0}",
];

/// Splits `text` into tokens and the whitespace between them, keeping every character.
fn chunks(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut class = 0u8;
    for ch in text.chars() {
        let c = if ch.is_alphanumeric() || ch == '_' {
            1
        } else if ch.is_whitespace() {
            2
        } else {
            3
        };
        if c != class || c == 3 {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            class = c;
        }
        cur.push(ch);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Returns `text` after one to four token-level mutations, splicing from `donor`.
fn mutate(rng: &mut Rng, text: &str, donor: &str) -> String {
    let mut cs = chunks(text);
    let donor = chunks(donor);
    for _ in 0..=rng.below(4) {
        if cs.is_empty() {
            cs.push(rng.pick(VOCABULARY).to_string());
        }
        let i = rng.below(cs.len() as u64) as usize;
        let j = (i + 1 + rng.below(4) as usize).min(cs.len());
        match rng.below(10) {
            0 => {
                cs.drain(i..j);
            }
            1 => {
                let dup: Vec<String> = cs[i..j].to_vec();
                cs.splice(j..j, dup);
            }
            2 => {
                let k = rng.below(cs.len() as u64) as usize;
                cs.swap(i, k);
            }
            3 => cs[i] = rng.pick(VOCABULARY).to_string(),
            4 => cs.insert(i, format!(" {} ", rng.pick(VOCABULARY))),
            5 if !donor.is_empty() => {
                let a = rng.below(donor.len() as u64) as usize;
                let b = (a + 1 + rng.below(12) as usize).min(donor.len());
                cs.splice(i..i, donor[a..b].iter().cloned());
            }
            6 => cs.truncate(i),
            7 => {
                let (open, close) =
                    *rng.pick(&[("(", ")"), ("[", "]"), ("not (", ")"), ("-(", ")")]);
                let k = 1 + rng.below(80) as usize;
                cs.insert(j, close.repeat(k));
                cs.insert(i, open.repeat(k));
            }
            8 => {
                let k = 1 + rng.below(80) as usize;
                let link = rng.pick(&[" + 1", " or true", ".n", " ++ \"a\""]);
                cs.insert(j, link.repeat(k));
            }
            _ => {
                let s = &mut cs[i];
                let at = s.chars().count();
                let mut chars: Vec<char> = s.chars().collect();
                let c = *rng.pick(&[
                    '\u{0}',
                    '\u{7f}',
                    'é',
                    '\u{1f600}',
                    '"',
                    '\\',
                    '\n',
                    '`',
                    '\u{ffff}',
                ]);
                chars.insert(rng.below(at as u64 + 1) as usize, c);
                *s = chars.into_iter().collect();
            }
        }
    }
    cs.concat()
}
