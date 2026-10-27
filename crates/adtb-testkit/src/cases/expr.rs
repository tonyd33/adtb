use std::collections::{BTreeMap, BTreeSet};

use super::world::World;

/// How a restriction tests its predicate under `try`.
#[derive(Clone, Copy, Debug)]
pub(super) enum Tried {
    True,
    False,
    Faults,
}

/// The operators whose operands have the type of their result.
const CLOSED: [&str; 8] = ["+", "-", "*", "/", "%", "++", "and", "or"];

/// A step of a fixpoint: the rows `var` of a relation, or of one of its constructors, from whose
/// field `into` equal to the element the step reaches field `out`.
#[derive(Clone, Debug)]
pub(super) struct Step {
    pub(super) rel: usize,
    pub(super) ctor: Option<usize>,
    pub(super) into: String,
    pub(super) out: String,
}

/// An expression, rendered once its generators' constructor sets are known.
#[derive(Clone, Debug)]
pub(super) enum E {
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
    /// names `x`, `y`, `v`, the step and the base.
    Fix([String; 3], Step, Box<Comp>),
}

#[derive(Clone, Debug)]
pub(super) struct Binding {
    pub(super) var: String,
    pub(super) rel: usize,
    /// The constructors the variable ranges over, ascending.
    pub(super) ctors: Vec<usize>,
}

#[derive(Clone, Debug)]
pub(super) struct Comp {
    pub(super) gens: Vec<Binding>,
    pub(super) filter: Option<E>,
    pub(super) select: E,
}

impl E {
    /// Returns the expressions directly inside this one, outside a comprehension.
    fn children(&self) -> Vec<&E> {
        match self {
            E::Lit(_)
            | E::Path(..)
            | E::KeyOf(_)
            | E::Agg(..)
            | E::Exists(_)
            | E::Set(_)
            | E::Fix(..) => Vec::new(),
            E::Bin(_, a, b) | E::Union(a, b) => vec![a, b],
            E::Not(a) | E::Call(_, a) | E::Tried(a, _) => vec![a],
            E::If(c, a, b) => vec![c, a, b],
            E::Row(_, _, arms) => arms.iter().map(|(_, _, e)| e).collect(),
            E::Shade(s, r, g, _, b) => vec![s, r, g, b],
            E::Opt(o, n, _, s) => vec![o, n, s],
            E::List(xs) => xs.iter().collect(),
            E::Rec(fs) => fs.iter().map(|(_, e)| e).collect(),
        }
    }

    fn children_mut(&mut self) -> Vec<&mut E> {
        match self {
            E::Lit(_)
            | E::Path(..)
            | E::KeyOf(_)
            | E::Agg(..)
            | E::Exists(_)
            | E::Set(_)
            | E::Fix(..) => Vec::new(),
            E::Bin(_, a, b) | E::Union(a, b) => vec![a, b],
            E::Not(a) | E::Call(_, a) | E::Tried(a, _) => vec![a],
            E::If(c, a, b) => vec![c, a, b],
            E::Row(_, _, arms) => arms.iter_mut().map(|(_, _, e)| e).collect(),
            E::Shade(s, r, g, _, b) => vec![s, r, g, b],
            E::Opt(o, n, _, s) => vec![o, n, s],
            E::List(xs) => xs.iter_mut().collect(),
            E::Rec(fs) => fs.iter_mut().map(|(_, e)| e).collect(),
        }
    }

    /// Returns the comprehension directly inside this expression, if any.
    fn comp(&self) -> Option<&Comp> {
        match self {
            E::Agg(_, c) | E::Exists(c) | E::Set(c) | E::Fix(_, _, c) => Some(c),
            _ => None,
        }
    }

    /// Calls `f` on this expression and every expression inside it, comprehensions included.
    pub(super) fn visit(&self, f: &mut impl FnMut(&E)) {
        f(self);
        for c in self.children() {
            c.visit(f);
        }
        if let Some(c) = self.comp() {
            c.visit(f);
        }
    }

    pub(super) fn reads(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        self.visit(&mut |e| match e {
            E::Path(v, _) | E::KeyOf(v) | E::Row(v, ..) => {
                out.insert(v.clone());
            }
            _ => {}
        });
        out
    }

    pub(super) fn binds(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        self.visit(&mut |e| match e {
            E::Row(_, _, arms) => out.extend(arms.iter().map(|(_, x, _)| x.clone())),
            E::Shade(_, _, _, x, _) | E::Opt(_, _, x, _) => {
                out.insert(x.clone());
            }
            E::Agg(_, c) | E::Exists(c) | E::Set(c) => {
                out.extend(c.gens.iter().map(|b| b.var.clone()));
            }
            E::Fix(names, _, c) => {
                out.extend(names.iter().cloned());
                out.extend(c.gens.iter().map(|b| b.var.clone()));
            }
            _ => {}
        });
        out
    }

    /// Returns whether a comprehension inside this expression reads a variable bound outside it.
    pub(super) fn correlated(&self) -> bool {
        let mut found = false;
        self.visit(&mut |e| {
            if let E::Agg(_, c) | E::Exists(c) | E::Set(c) | E::Fix(_, _, c) = e {
                let own: BTreeSet<String> = c.gens.iter().map(|b| b.var.clone()).collect();
                let inner = c.binds();
                found |= c
                    .reads()
                    .iter()
                    .any(|v| !own.contains(v) && !inner.contains(v));
            }
        });
        found
    }

    /// Returns the expressions one step smaller than this one, each of its type and reading only
    /// variables it reads: a part of it in its place, then it with one child or comprehension
    /// smaller.
    pub(super) fn shrinks(&self) -> Vec<E> {
        let mut out = self.hoists();
        for (k, child) in self.children().into_iter().enumerate() {
            for small in child.shrinks() {
                let mut e = self.clone();
                *e.children_mut().swap_remove(k) = small;
                out.push(e);
            }
        }
        if let Some(c) = self.comp() {
            for small in c.shrinks(&BTreeSet::new()) {
                let mut e = self.clone();
                if let E::Agg(_, c) | E::Exists(c) | E::Set(c) | E::Fix(_, _, c) = &mut e {
                    *c.as_mut() = small;
                }
                out.push(e);
            }
        }
        out
    }

    /// Returns the expressions of this one's type that a part of it gives in its place.
    fn hoists(&self) -> Vec<E> {
        let lits = || vec![E::Lit("true".into()), E::Lit("false".into())];
        let unbound = |x: &String, body: &E| (!body.reads().contains(x)).then(|| body.clone());
        match self {
            E::Bin(op, a, b) if CLOSED.contains(op) => vec![a.as_ref().clone(), b.as_ref().clone()],
            E::Bin(..) | E::Exists(_) => lits(),
            E::If(_, a, b) | E::Union(a, b) => vec![a.as_ref().clone(), b.as_ref().clone()],
            E::Not(a) => vec![a.as_ref().clone()],
            E::Row(_, _, arms) => arms.iter().filter_map(|(_, x, e)| unbound(x, e)).collect(),
            E::Shade(_, red, green, x, blue) => {
                [Some(red.as_ref().clone()), Some(green.as_ref().clone())]
                    .into_iter()
                    .chain([unbound(x, blue)])
                    .flatten()
                    .collect()
            }
            E::Opt(_, none, x, some) => [Some(none.as_ref().clone()), unbound(x, some)]
                .into_iter()
                .flatten()
                .collect(),
            E::List(xs) if xs.len() > 1 => (0..xs.len())
                .map(|k| {
                    let mut ys = xs.clone();
                    ys.remove(k);
                    E::List(ys)
                })
                .collect(),
            E::Fix(_, _, base) => vec![E::Set(base.clone())],
            _ => Vec::new(),
        }
    }
}

impl Comp {
    fn visit(&self, f: &mut impl FnMut(&E)) {
        if let Some(c) = &self.filter {
            c.visit(f);
        }
        self.select.visit(f);
    }

    pub(super) fn reads(&self) -> BTreeSet<String> {
        let mut out = self.select.reads();
        out.extend(self.filter.iter().flat_map(E::reads));
        out
    }

    fn binds(&self) -> BTreeSet<String> {
        let mut out = self.select.binds();
        out.extend(self.filter.iter().flat_map(E::binds));
        out
    }

    /// Returns the comprehensions one step smaller than this one, keeping every generator whose
    /// variable `outside` reads.
    pub(super) fn shrinks(&self, outside: &BTreeSet<String>) -> Vec<Comp> {
        let mut out = Vec::new();
        if let Some(f) = &self.filter {
            out.push(Comp {
                filter: None,
                ..self.clone()
            });
            out.extend(f.shrinks().into_iter().map(|f| Comp {
                filter: Some(f),
                ..self.clone()
            }));
        }
        if self.gens.len() > 1 {
            let read = self.reads();
            for k in 0..self.gens.len() {
                let v = &self.gens[k].var;
                if !read.contains(v) && !outside.contains(v) {
                    let mut c = self.clone();
                    c.gens.remove(k);
                    out.push(c);
                }
            }
        }
        out.extend(self.select.shrinks().into_iter().map(|s| Comp {
            select: s,
            ..self.clone()
        }));
        out
    }

    pub(super) fn correlated(&self) -> bool {
        self.filter.as_ref().is_some_and(E::correlated) || self.select.correlated()
    }
}

pub(super) struct Render<'w> {
    w: &'w World,
    /// The constructors each variable ranges over, set before rendering to narrow a generator.
    pub(super) ctors: BTreeMap<String, Vec<usize>>,
}

impl<'w> Render<'w> {
    pub(super) fn new(w: &'w World) -> Render<'w> {
        Render {
            w,
            ctors: BTreeMap::new(),
        }
    }

    pub(super) fn comp(&mut self, c: &Comp) -> String {
        let gens = self.gens(&c.gens);
        let filter = self.filter(&c.filter);
        format!("(from {gens}{filter} select {})", self.expr(&c.select))
    }

    pub(super) fn gens(&mut self, gens: &[Binding]) -> String {
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

    pub(super) fn filter(&mut self, f: &Option<E>) -> String {
        match f {
            Some(f) => format!(" where {}", self.expr(f)),
            None => String::new(),
        }
    }

    pub(super) fn expr(&mut self, e: &E) -> String {
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
            E::Fix([x, y, v], step, base) => {
                let r = &self.w.rels[step.rel];
                let range = match step.ctor {
                    Some(c) => format!("{}.{}", r.name, r.ctors[c].name),
                    None => r.name.clone(),
                };
                format!(
                    "(fix {x} = ({} union (from {y} in {x}, {v} in {range} where ({v}.{} == {y}) \
                     select {v}.{})) in {x})",
                    self.comp(base),
                    step.into,
                    step.out
                )
            }
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
