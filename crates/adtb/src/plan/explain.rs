//! The plan of a statement as `explain` shows it: a tree of operators, as indented text and as
//! a JSON document.

use std::collections::BTreeMap;
use std::fmt::Write;

use adtb_lang::schema::Schema;
use adtb_lang::types::{RelId, Ty};

use super::{
    Catalog, Chain, End, Eval, HeapId, IndexId, Pipeline, Plan, Query, Sink, Source, Spine, Step,
};

/// The version of the shape of [`Node::document`].
pub const FORMAT: u32 = 1;

/// One operator of a plan.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    pub kind: String,
    /// What the operator reads or computes, for people.
    pub detail: String,
    pub children: Vec<Node>,
    /// The arm of each constructor a routing operator routes to, by constructor index.
    pub arms: Vec<Arm>,
    /// The constructors of a read of a sum it reads none of.
    pub pruned: Vec<Pruned>,
    pub actual: Option<Actual>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Arm {
    pub ctor: u32,
    pub name: String,
    pub child: Node,
}

/// A constructor a read leaves out, and whether types or parameters pruned it.
#[derive(Clone, Debug, PartialEq)]
pub struct Pruned {
    pub ctor: u32,
    pub name: String,
    pub at_start: bool,
}

/// What a run of an operator did.
#[derive(Clone, Debug, PartialEq)]
pub struct Actual {
    pub rows: u64,
    pub loops: u64,
    pub time_us: u64,
    pub storage: String,
}

impl Node {
    pub fn new(kind: &str, detail: impl Into<String>, children: Vec<Node>) -> Node {
        Node {
            kind: kind.to_string(),
            detail: detail.into(),
            children,
            ..Node::default()
        }
    }

    /// Returns the tree as indented text, one operator per line.
    pub fn text(&self) -> String {
        let mut out = String::new();
        self.text_at(0, &mut out);
        out
    }

    fn text_at(&self, depth: usize, out: &mut String) {
        let pad = "  ".repeat(depth);
        let line = match self.detail.as_str() {
            "" => self.kind.clone(),
            d => format!("{} {d}", self.kind),
        };
        writeln!(out, "{pad}{line}").expect("a String takes every write");
        if !self.pruned.is_empty() {
            let names: Vec<String> = self
                .pruned
                .iter()
                .map(|p| format!("{} at {}", p.name, stage(p.at_start)))
                .collect();
            writeln!(out, "{pad}  pruned {}", names.join(", "))
                .expect("a String takes every write");
        }
        if let Some(a) = &self.actual {
            writeln!(
                out,
                "{pad}  actual rows {} loops {} time {}us",
                a.rows, a.loops, a.time_us
            )
            .expect("a String takes every write");
        }
        for c in &self.children {
            c.text_at(depth + 1, out);
        }
        for a in &self.arms {
            writeln!(out, "{pad}  {} ->", a.name).expect("a String takes every write");
            a.child.text_at(depth + 2, out);
        }
    }

    /// Returns the plan document whose root is this node, `extra` holding further members of
    /// the top-level object, each a name and a JSON value.
    pub fn document(&self, extra: &[(&str, String)]) -> String {
        let mut members = vec![
            ("format".to_string(), FORMAT.to_string()),
            ("release".to_string(), json_text(env!("CARGO_PKG_VERSION"))),
            ("root".to_string(), self.json()),
        ];
        members.extend(extra.iter().map(|(n, v)| ((*n).to_string(), v.clone())));
        object(&members)
    }

    fn json(&self) -> String {
        let mut members = vec![
            ("kind".to_string(), json_text(&self.kind)),
            ("detail".to_string(), json_text(&self.detail)),
            ("estimate".to_string(), "null".to_string()),
            (
                "children".to_string(),
                array(self.children.iter().map(Node::json)),
            ),
        ];
        if !self.arms.is_empty() {
            let arms = self.arms.iter().map(|a| {
                object(&[
                    ("ctor".to_string(), a.ctor.to_string()),
                    ("name".to_string(), json_text(&a.name)),
                    ("child".to_string(), a.child.json()),
                ])
            });
            members.push(("arms".to_string(), array(arms)));
        }
        let pruned = self.pruned.iter().map(|p| {
            object(&[
                ("ctor".to_string(), p.ctor.to_string()),
                ("name".to_string(), json_text(&p.name)),
                ("stage".to_string(), json_text(stage(p.at_start))),
            ])
        });
        members.push(("pruned".to_string(), array(pruned)));
        if let Some(a) = &self.actual {
            members.push((
                "actual".to_string(),
                object(&[
                    ("rows".to_string(), a.rows.to_string()),
                    ("loops".to_string(), a.loops.to_string()),
                    ("time_us".to_string(), a.time_us.to_string()),
                    ("storage".to_string(), json_text(&a.storage)),
                ]),
            ));
        }
        object(&members)
    }
}

fn stage(at_start: bool) -> &'static str {
    if at_start { "start" } else { "compile" }
}

/// Returns `t` as a JSON string.
pub fn json_text(t: &str) -> String {
    let mut out = String::from("\"");
    for c in t.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                write!(out, "\\u{:04x}", u32::from(c)).expect("a String takes every write");
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Returns the JSON object of `members`, each a name and a JSON value.
pub fn object(members: &[(String, String)]) -> String {
    let ms: Vec<String> = members
        .iter()
        .map(|(n, v)| format!("{}: {v}", json_text(n)))
        .collect();
    format!("{{{}}}", ms.join(", "))
}

fn array(xs: impl Iterator<Item = String>) -> String {
    format!("[{}]", xs.collect::<Vec<_>>().join(", "))
}

/// Returns the name of constructor `c` of a value of type `ty`, or its number when the type
/// is not a sum.
pub fn ctor_name(schema: &Schema, ty: &Ty, c: u32) -> String {
    let name = match crate::exec::types::repr(schema, ty) {
        Ty::Data { def, .. } => schema
            .ty(def)
            .ctors()
            .get(c as usize)
            .map(|k| k.name.clone()),
        Ty::Sum { ctors, .. } => ctors.get(c as usize).map(|(n, _)| n.clone()),
        _ => None,
    };
    name.unwrap_or_else(|| format!("#{c}"))
}

/// Builds the operators of plans over one schema and catalog.
#[derive(Debug)]
pub struct Describe<'a> {
    schema: &'a Schema,
    heaps: BTreeMap<HeapId, (RelId, u32)>,
    indexes: BTreeMap<IndexId, (RelId, usize)>,
    catalog: &'a Catalog,
}

impl<'a> Describe<'a> {
    pub fn new(schema: &'a Schema, catalog: &'a Catalog) -> Describe<'a> {
        let mut heaps = BTreeMap::new();
        let mut indexes = BTreeMap::new();
        for (r, d) in &catalog.relations {
            for (c, h) in &d.heaps {
                heaps.insert(*h, (*r, *c));
            }
            for (i, x) in d.indexes.iter().enumerate() {
                indexes.insert(x.id, (*r, i));
            }
        }
        Describe {
            schema,
            heaps,
            indexes,
            catalog,
        }
    }

    /// Returns the operators of `plan`.
    pub fn plan(&self, plan: &Plan) -> Node {
        self.eval(plan, &plan.root)
    }

    fn heap(&self, h: HeapId) -> String {
        match self.heaps.get(&h) {
            Some((r, c)) => {
                let rel = self.schema.relation(*r);
                let ctor = &self.schema.ty(rel.elem_def).ctors()[*c as usize].name;
                format!("{}.{ctor}", rel.name)
            }
            None => format!("heap {}", h.0),
        }
    }

    fn index(&self, i: IndexId) -> String {
        let Some((r, k)) = self.indexes.get(&i) else {
            return format!("index {}", i.0);
        };
        let desc = &self.catalog.relation(*r).indexes[*k];
        let elements: Vec<String> = desc
            .elements
            .iter()
            .map(|e| match e {
                super::Element::Field(f) => f.clone(),
                super::Element::Derived { index, element } => format!("#{index}.{element}"),
            })
            .collect();
        format!(
            "{} ({})",
            self.schema.relation(*r).name,
            elements.join(", ")
        )
    }

    /// Returns the constructor name of `c` of relation `r`'s sum.
    fn rel_ctor(&self, r: RelId, c: u32) -> String {
        let rel = self.schema.relation(r);
        self.schema.ty(rel.elem_def).ctors()[c as usize]
            .name
            .clone()
    }

    fn eval(&self, plan: &Plan, e: &Eval) -> Node {
        match e {
            Eval::Expr(_) => Node::new("Compute", "", Vec::new()),
            Eval::Let(_, a, b) => {
                Node::new("Let", "", vec![self.eval(plan, a), self.eval(plan, b)])
            }
            Eval::If(_, a, b) => Node::new("If", "", vec![self.eval(plan, a), self.eval(plan, b)]),
            Eval::Case(t, arms) => {
                let ty = plan.terms.ty(*t);
                let mut n = Node::new("Route by constructor", "", Vec::new());
                n.arms = arms
                    .iter()
                    .map(|(c, _, a)| Arm {
                        ctor: *c,
                        name: ctor_name(self.schema, ty, *c),
                        child: self.eval(plan, a),
                    })
                    .collect();
                n
            }
            Eval::Try(e) => Node::new("Try", "", vec![self.eval(plan, e)]),
            Eval::Lookup(l) => {
                let mut n = Node::new("Lookup", self.index(l.index), Vec::new());
                n.arms = l
                    .routes
                    .iter()
                    .map(|(c, h)| Arm {
                        ctor: *c,
                        name: self.heap_ctor(*h, *c),
                        child: Node::new("Fetch", self.heap(*h), Vec::new()),
                    })
                    .collect();
                n.pruned = self.pruned(l.index, l.routes.iter().map(|(c, _)| *c));
                n
            }
            Eval::Query(q) => self.query(plan, q),
            Eval::GroupRows { .. } => Node::new("Rows of a group", "", Vec::new()),
            Eval::At(a) => {
                let point = if a.instant { "at instant" } else { "at commit" };
                let detail = match a.pins {
                    Some(v) => format!("{point}, typed at version {v}"),
                    None => point.to_string(),
                };
                Node::new("Read the past", detail, vec![self.eval(plan, &a.body)])
            }
            Eval::Fix(f) => {
                let mut children: Vec<Node> = f
                    .binds
                    .iter()
                    .map(|b| Node::new("Step", "", vec![self.eval(plan, &b.body)]))
                    .collect();
                children.push(self.eval(plan, &f.body));
                Node::new("Fixpoint", "", children)
            }
        }
    }

    fn heap_ctor(&self, h: HeapId, c: u32) -> String {
        match self.heaps.get(&h) {
            Some((r, _)) => self.rel_ctor(*r, c),
            None => format!("#{c}"),
        }
    }

    /// Returns the constructors of the relation of index `i` that no route of `routed` reads,
    /// pruned at compile time.
    fn pruned(&self, i: IndexId, routed: impl Iterator<Item = u32>) -> Vec<Pruned> {
        let Some((r, _)) = self.indexes.get(&i) else {
            return Vec::new();
        };
        let routed: Vec<u32> = routed.collect();
        let rel = self.schema.relation(*r);
        (0..self.schema.ty(rel.elem_def).ctors().len() as u32)
            .filter(|c| !routed.contains(c))
            .map(|c| Pruned {
                ctor: c,
                name: self.rel_ctor(*r, c),
                at_start: false,
            })
            .collect()
    }

    fn query(&self, plan: &Plan, q: &Query) -> Node {
        let mut children: Vec<Node> = q
            .builds
            .iter()
            .enumerate()
            .map(|(i, b)| {
                Node::new(
                    "Build",
                    format!("#{i}"),
                    b.pipelines.iter().map(|p| self.pipeline(plan, p)).collect(),
                )
            })
            .collect();
        children.push(self.spine(plan, &q.spine));
        let (kind, detail) = match &q.sink {
            Sink::Set => ("Set", String::new()),
            Sink::Exists => ("Exists", String::new()),
            Sink::IsEmpty => ("Is empty", String::new()),
            Sink::Agg { op, .. } => ("Aggregate", format!("{op:?}").to_lowercase()),
            Sink::Fold { .. } => ("Aggregate", "user-defined".to_string()),
            Sink::Group { aggs, .. } => ("Group", format!("{} aggregates", aggs.len())),
            Sink::Top { .. } => ("Top", String::new()),
            Sink::Present { .. } => ("Present", String::new()),
        };
        Node::new(kind, detail, children)
    }

    fn spine(&self, plan: &Plan, s: &Spine) -> Node {
        match s {
            Spine::Pipeline(p) => self.pipeline(plan, p),
            Spine::Union(ss) => Node::new(
                "Union",
                "",
                ss.iter().map(|s| self.spine(plan, s)).collect(),
            ),
            Spine::If(_, a, b) => Node::new(
                "If at start",
                "",
                vec![self.spine(plan, a), self.spine(plan, b)],
            ),
            Spine::Case(t, arms) => {
                let ty = plan.terms.ty(*t);
                let mut n = Node::new("Route at start", "", Vec::new());
                n.arms = arms
                    .iter()
                    .map(|(c, _, a)| Arm {
                        ctor: *c,
                        name: ctor_name(self.schema, ty, *c),
                        child: self.spine(plan, a),
                    })
                    .collect();
                n
            }
            Spine::Let(_, e, rest) => {
                Node::new("Let", "", vec![self.eval(plan, e), self.spine(plan, rest)])
            }
        }
    }

    fn pipeline(&self, plan: &Plan, p: &Pipeline) -> Node {
        let source = match &p.source {
            Source::Unit => Node::new("One row", "", Vec::new()),
            Source::Scan { heap, .. } => Node::new("Scan", self.heap(*heap), Vec::new()),
        };
        let mut chain = self.chain(plan, &p.chain);
        chain.insert(0, source);
        Node::new("Pipeline", "", chain)
    }

    /// Returns the operators of `c`, its steps in order and then its end.
    fn chain(&self, plan: &Plan, c: &Chain) -> Vec<Node> {
        let mut out: Vec<Node> = c
            .steps
            .iter()
            .map(|s| match s {
                Step::Filter(_) => Node::new("Filter", "", Vec::new()),
                Step::Bind(_, e) => Node::new("Bind", "", vec![self.eval(plan, e)]),
                Step::Unnest(..) => Node::new("Unnest", "", Vec::new()),
                Step::Probe { build, .. } => {
                    Node::new("Probe build", format!("#{}", build.0), Vec::new())
                }
            })
            .collect();
        out.push(self.end(plan, &c.end));
        out
    }

    fn end(&self, plan: &Plan, e: &End) -> Node {
        let chained = |c: &Chain| Node::new("Chain", "", self.chain(plan, c));
        match e {
            End::Nothing => Node::new("Drop", "", Vec::new()),
            End::Emit(_) => Node::new("Emit", "", Vec::new()),
            End::Fork(cs) => Node::new("Fork", "", cs.iter().map(chained).collect()),
            End::If(_, a, b) => Node::new("If", "", vec![chained(a), chained(b)]),
            End::Case(t, arms) => {
                let ty = plan.terms.ty(*t);
                let mut n = Node::new("Route by constructor", "", Vec::new());
                n.arms = arms
                    .iter()
                    .map(|(c, _, a)| Arm {
                        ctor: *c,
                        name: ctor_name(self.schema, ty, *c),
                        child: chained(a),
                    })
                    .collect();
                n
            }
            End::Index { index, routes, .. } | End::Range { index, routes, .. } => {
                let kind = match e {
                    End::Range { .. } => "Range probe",
                    _ => "Probe",
                };
                let mut n = Node::new(kind, self.index(*index), Vec::new());
                n.arms = routes
                    .iter()
                    .map(|r| Arm {
                        ctor: r.ctor,
                        name: self.heap_ctor(r.heap, r.ctor),
                        child: chained(&r.chain),
                    })
                    .collect();
                n.pruned = self.pruned(*index, routes.iter().map(|r| r.ctor));
                n
            }
        }
    }
}
