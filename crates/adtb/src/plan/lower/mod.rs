use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use adtb_lang::analysis::Totality;
use adtb_lang::ast::FnBody;
use adtb_lang::kernel::{Agg, Kind, PathStep, Prim, Terms, Tm, Var};
use adtb_lang::norm::NormalProgram;
use adtb_lang::schema::Schema;
use adtb_lang::types::{CtorSet, RelId, Ty};

use super::catalog::{Catalog, IndexDesc};
use super::terms::{Facts, with_children, with_vars};
use super::*;
use reads::collect_eval;

mod group;
mod probe;
mod reads;
mod split;

/// The rows the fixed cost model assumes in every heap.
const HEAP_ROWS: f64 = 1000.0;
/// The rows the fixed cost model assumes a probe of an index that is not unique returns.
const PROBE_ROWS: f64 = 100.0;
/// The cost the fixed cost model assigns to one index probe, relative to reading one row.
const PROBE_COST: f64 = 4.0;
/// The rows the fixed cost model assumes an unnested set holds.
const SET_ROWS: f64 = 10.0;

/// Returns the plan of `root`, a normal form in `terms` whose free variables are `params`.
///
/// Preconditions:
/// - `root` is a normal form `adtb_lang::norm` returned for `schema`, in the arena `terms`.
/// - `normal` is the normal program of `schema`.
/// - `catalog` describes every relation variable `root` reads.
pub fn plan(
    schema: &Schema,
    normal: &NormalProgram,
    catalog: &Catalog,
    terms: &Terms,
    root: Tm,
    params: &[Var],
    options: Options,
) -> Result<Plan, Error> {
    let input = Input {
        schema,
        normal,
        catalog,
        terms,
        root,
        params,
    };
    plan_in(&input, options, false)
}

/// Returns the plan of `root`, as [`plan`] does, reading every constructor relation only through
/// an index probe keyed by values bound before it.
///
/// Fails with [`Error::Unbounded`] when some read has no such probe, and with
/// [`Error::Outside`] when `root` holds a `fix`.
pub fn plan_bounded(
    schema: &Schema,
    normal: &NormalProgram,
    catalog: &Catalog,
    terms: &Terms,
    root: Tm,
    params: &[Var],
) -> Result<Plan, Error> {
    let input = Input {
        schema,
        normal,
        catalog,
        terms,
        root,
        params,
    };
    plan_in(&input, Options::default(), true)
}

/// What [`plan`] plans: `root`, a normal form in `terms` whose free variables are `params`.
struct Input<'a> {
    schema: &'a Schema,
    normal: &'a NormalProgram,
    catalog: &'a Catalog,
    terms: &'a Terms,
    root: Tm,
    params: &'a [Var],
}

fn plan_in(input: &Input<'_>, options: Options, bounded: bool) -> Result<Plan, Error> {
    let Input {
        schema,
        normal,
        catalog,
        terms,
        root,
        params,
    } = *input;
    user_code(schema, terms, root)?;
    let mut p = Planner {
        schema,
        normal,
        catalog,
        rw: match forced() {
            Some(rw) if !bounded => rw,
            _ => options.rewrites,
        },
        bounded,
        past: false,
        t: terms.clone(),
        total: Totality::new(schema, options.hiding),
        facts: Facts::default(),
        vars: HashMap::new(),
        outer: Vec::new(),
    };
    let root = p.eval(root)?;
    let mut reads = Reads::default();
    collect_eval(catalog, &root, &mut reads);
    Ok(Plan {
        terms: p.t,
        params: params.to_vec(),
        root,
        ty: terms.ty(input.root).clone(),
        reads,
    })
}

/// Refuse a term reaching a call of a function other than a host function: a Wasm function or
/// one that takes or returns a function, which no plan runs.
fn user_code(schema: &Schema, terms: &Terms, root: Tm) -> Result<(), Error> {
    let mut seen = std::collections::HashSet::new();
    let mut todo = vec![root];
    while let Some(t) = todo.pop() {
        if !seen.insert(t) {
            continue;
        }
        if let Kind::Call { func, .. } = terms.kind(t) {
            let def = schema.function(*func);
            if !matches!(def.body, FnBody::Host { .. }) {
                return Err(Error::UserCode(def.name.clone()));
            }
        }
        todo.extend(terms.children(t));
    }
    Ok(())
}

struct Planner<'a> {
    schema: &'a Schema,
    normal: &'a NormalProgram,
    catalog: &'a Catalog,
    rw: Rewrites,
    /// Set to read every constructor relation only through an index probe.
    bounded: bool,
    /// Set while planning the body of an `at` block, which reads no index but key indexes, and
    /// those only by key lookups.
    past: bool,
    t: Terms,
    total: Totality<'a>,
    facts: Facts,
    vars: HashMap<Var, Tm>,
    /// One entry per chain being planned, innermost last.
    outer: Vec<Outer>,
}

/// What a query planned inside a chain may add to the query that holds the chain.
struct Outer {
    /// The variables bound where the holding query is evaluated.
    env: Rc<BTreeSet<Var>>,
    /// The rows the fixed cost model assumes reach the chain.
    rows: f64,
    /// Bindings for the holding query's spine.
    lets: Vec<(Var, Eval)>,
}

/// What is known at one point of a query's pipelines.
#[derive(Clone, Debug, Default)]
struct Scope {
    /// The variables bound inside the query.
    local: BTreeSet<Var>,
    /// Variables whose constructor is fixed.
    known: BTreeMap<Var, u32>,
    /// The rows the fixed cost model assumes reach this point per evaluation of the query.
    rows: f64,
}

impl Scope {
    fn new() -> Scope {
        Scope {
            rows: 1.0,
            ..Scope::default()
        }
    }

    fn bind(&self, v: Var) -> Scope {
        let mut s = self.clone();
        s.local.insert(v);
        s.known.remove(&v);
        s
    }

    fn bind_known(&self, v: Var, ctor: Option<u32>) -> Scope {
        let mut s = self.bind(v);
        if let Some(c) = ctor {
            s.known.insert(v, c);
        }
        s
    }

    fn times(mut self, f: f64) -> Scope {
        self.rows *= f;
        self
    }
}

#[derive(Default)]
struct QueryCx {
    builds: Vec<Build>,
    memo: HashMap<BuildKey, BuildId>,
    /// The variables bound where the query is evaluated.
    env: Rc<BTreeSet<Var>>,
    /// Bindings that wrap the query's spine.
    lets: Vec<(Var, Eval)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct BuildKey {
    rel: RelId,
    ctors: CtorSet,
    var: Var,
    key: Vec<Tm>,
    filters: Vec<Tm>,
}

/// An index probe: the index, the values of its leading fields, the bounds of the next one, and
/// the conditions the values replace.
struct IndexProbe {
    index: IndexDesc,
    key: Vec<Tm>,
    range: Option<Span>,
    used: Vec<usize>,
}

/// The bounds a range probe puts on the index field after its key.
#[derive(Clone, Copy, Debug)]
struct Span {
    lo: Option<RangeBound>,
    hi: Option<RangeBound>,
}

/// A bound a comparison puts on a field of a row: the field, whether from below, and the bound.
type Limit = (String, bool, RangeBound);

impl IndexProbe {
    fn unique(&self) -> bool {
        self.index.unique && self.key.len() == self.index.elements.len()
    }
}

/// What an equality condition fixes of a row.
#[derive(Clone, Debug)]
struct Fixed {
    /// The path from the row the fixed side reads, when it reads one.
    path: Option<Vec<PathStep>>,
    term: Tm,
}

/// What conditions fix of a row, each with its value.
type Fixes = Vec<(Fixed, Tm)>;

impl Fixed {
    fn field(f: String, term: Tm) -> Fixed {
        Fixed {
            path: Some(vec![PathStep::Field(f)]),
            term,
        }
    }
}

/// One way to read a constructor relation inside a pipeline.
enum Access {
    Index(IndexProbe),
    Build {
        keys: Vec<(usize, Tm, Tm)>,
        filters: Vec<usize>,
    },
}

fn chain(steps: Vec<Step>, end: End) -> Chain {
    Chain { steps, end }
}

fn nothing() -> Chain {
    chain(Vec::new(), End::Nothing)
}

fn prepend(mut steps: Vec<Step>, c: Chain) -> Chain {
    steps.extend(c.steps);
    Chain { steps, end: c.end }
}

fn union(mut ss: Vec<Spine>) -> Spine {
    match ss.len() {
        1 => ss.pop().expect("one spine"),
        _ => Spine::Union(ss),
    }
}

fn wrap_eval(lets: Vec<(Var, Eval)>, body: Eval) -> Eval {
    lets.into_iter()
        .rev()
        .fold(body, |b, (v, e)| Eval::Let(v, Box::new(e), Box::new(b)))
}

fn wrap_spine(lets: Vec<(Var, Eval)>, body: Spine) -> Spine {
    lets.into_iter()
        .rev()
        .fold(body, |b, (v, e)| Spine::Let(v, e, Box::new(b)))
}

fn binds(lets: Vec<(Var, Eval)>) -> Vec<Step> {
    lets.into_iter().map(|(v, e)| Step::Bind(v, e)).collect()
}

/// Returns whether the chain neither evaluates nor emits anything.
fn is_nothing(c: &Chain) -> bool {
    c.steps.is_empty() && matches!(c.end, End::Nothing)
}

/// Returns the type with every constructor-set restriction removed.
fn erase(t: &Ty) -> Ty {
    match t {
        Ty::Data { def, args, .. } => Ty::Data {
            def: *def,
            args: args.iter().map(erase).collect(),
            ctors: None,
        },
        Ty::Key { rel, .. } => Ty::Key {
            rel: *rel,
            ctors: None,
        },
        Ty::Sum { ctors, .. } => Ty::Sum {
            ctors: ctors
                .iter()
                .map(|(c, fs)| {
                    (
                        c.clone(),
                        fs.iter().map(|(n, t)| (n.clone(), erase(t))).collect(),
                    )
                })
                .collect(),
            sel: None,
        },
        Ty::Set(t) | Ty::NonEmpty(t) => Ty::Set(Box::new(erase(t))),
        Ty::Record(fs) => Ty::Record(fs.iter().map(|(n, t)| (n.clone(), erase(t))).collect()),
        _ => t.clone(),
    }
}

impl Planner<'_> {
    fn kind(&self, t: Tm) -> Kind {
        self.t.kind(t).clone()
    }

    fn ty(&self, t: Tm) -> Ty {
        self.t.ty(t).clone()
    }

    fn add(&mut self, kind: Kind, ty: Ty) -> Tm {
        self.t.add(kind, ty)
    }

    fn fresh(&mut self, name: &str, ty: Ty) -> Var {
        self.t.var(name, ty)
    }

    fn var_tm(&mut self, v: Var) -> Tm {
        if let Some(t) = self.vars.get(&v) {
            return *t;
        }
        let ty = self.t.var_info(v).ty.clone();
        let t = self.add(Kind::Var(v), ty);
        self.vars.insert(v, t);
        t
    }

    fn scalar(&mut self, t: Tm) -> bool {
        self.facts.scalar(&self.t, t)
    }

    fn total(&mut self, t: Tm) -> bool {
        self.total.total(&self.t, t)
    }

    fn mentions(&mut self, t: Tm, vs: &[Var]) -> bool {
        self.facts.mentions(&self.t, t, vs)
    }

    fn free_in(&mut self, t: Tm, local: &BTreeSet<Var>) -> bool {
        self.facts
            .free(&self.t, t)
            .iter()
            .any(|v| local.contains(v))
    }

    fn bool_ty(&self) -> Ty {
        Ty::Data {
            def: self.schema.prelude_type("Bool"),
            args: Vec::new(),
            ctors: None,
        }
    }

    /// Returns the value of `t` when it is a literal `True` or `False`.
    fn literal(&self, t: Tm) -> Option<bool> {
        match self.t.kind(t) {
            Kind::Ctor { ctor, fields }
                if fields.is_empty() && erase(self.t.ty(t)) == self.bool_ty() =>
            {
                let def = self.schema.ty(self.schema.prelude_type("Bool"));
                Some(Some(*ctor) == def.ctor_index("True"))
            }
            _ => None,
        }
    }

    fn is_empty_set(&self, t: Tm) -> bool {
        matches!(self.t.kind(t), Kind::EmptySet)
    }

    fn empty(&mut self, ty: Ty) -> Tm {
        self.add(Kind::EmptySet, ty)
    }

    fn ctors_of(&self, r: RelId, s: &Option<CtorSet>) -> CtorSet {
        match s {
            Some(s) => s.clone(),
            None => self.all_ctors(r),
        }
    }

    fn all_ctors(&self, r: RelId) -> CtorSet {
        let n = self
            .schema
            .ty(self.schema.relation(r).elem_def)
            .ctors()
            .len();
        (0..n as u32).collect()
    }

    /// Returns the relation and constructor set of a source that reads one constructor relation.
    fn rel_source(&self, src: Tm) -> Option<(RelId, CtorSet)> {
        match self.t.kind(src) {
            Kind::Rel(r, s) => Some((*r, self.ctors_of(*r, s))),
            Kind::Restrict(inner, s) => {
                let (r, cs) = self.rel_source(*inner)?;
                Some((r, cs.into_iter().filter(|c| s.contains(c)).collect()))
            }
            _ => None,
        }
    }

    /// Splits a set term into the conditions guarding it, in evaluation order, and the term they guard.
    fn peel(&mut self, mut t: Tm) -> (Vec<Tm>, Tm) {
        let mut conds = Vec::new();
        while let &Kind::If(c, a, b) = self.t.kind(t)
            && self.is_empty_set(b)
        {
            self.conjuncts(c, &mut conds);
            t = a;
        }
        (conds, t)
    }

    /// Add the conjuncts of `c` to `out`, a `let` of a copyable value at the root moved onto each.
    fn conjuncts(&mut self, c: Tm, out: &mut Vec<Tm>) {
        match *self.t.kind(c) {
            Kind::If(a, b, f) if self.literal(f) == Some(false) => {
                self.conjuncts(a, out);
                self.conjuncts(b, out);
            }
            Kind::Let(x, e, b) if self.facts.copyable(&mut self.total, &self.t, e) => {
                let from = out.len();
                self.conjuncts(b, out);
                for k in &mut out[from..] {
                    *k = self.facts.sink_let(&mut self.t, x, e, *k);
                }
            }
            _ => out.push(c),
        }
    }

    /// Returns `rest` guarded by `conds` in order, a set term of type `ty`.
    fn guarded(&mut self, conds: &[Tm], rest: Tm, ty: &Ty) -> Tm {
        let mut out = rest;
        for c in conds.iter().rev() {
            match self.literal(*c) {
                Some(true) => {}
                Some(false) => out = self.empty(ty.clone()),
                None => {
                    let e = self.empty(ty.clone());
                    out = self.add(Kind::If(*c, out, e), ty.clone());
                }
            }
        }
        out
    }

    /// Splits a set term as `peel` does, adding the total conditions of a comprehension over a constructor relation directly under it that do not depend on its row.
    fn pulled(&mut self, body: Tm) -> (Vec<Tm>, Tm) {
        let (mut conds, rest) = self.peel(body);
        let Kind::For(y, src, inner) = self.kind(rest) else {
            return (conds, rest);
        };
        if self.rel_source(src).is_none() {
            return (conds, rest);
        }
        let (ic, ir) = self.peel(inner);
        let n = self.total_prefix(&ic);
        let mut kept = Vec::new();
        let before = conds.len();
        for (i, c) in ic.iter().enumerate() {
            if i < n && !self.mentions(*c, &[y]) {
                conds.push(*c);
            } else {
                kept.push(*c);
            }
        }
        if conds.len() == before {
            return (conds, rest);
        }
        let ty = self.ty(rest);
        let inner = self.guarded(&kept, ir, &ty);
        let rest = self.add(Kind::For(y, src, inner), ty);
        (conds, rest)
    }

    /// Returns scalar `t` with each case over a variable of fixed constructor replaced by its arm, when static routing is on.
    fn here(&mut self, t: Tm, sc: &Scope) -> Result<Tm, Error> {
        if !self.rw.static_route || sc.known.is_empty() {
            return Ok(t);
        }
        self.specialise(t, &sc.known, &mut HashMap::new())
    }

    fn specialise(
        &mut self,
        t: Tm,
        known: &BTreeMap<Var, u32>,
        memo: &mut HashMap<Tm, Tm>,
    ) -> Result<Tm, Error> {
        if let Some(n) = memo.get(&t) {
            return Ok(*n);
        }
        let out = match self.kind(t) {
            Kind::Case { scrutinee, arms }
                if let Kind::Var(v) = self.kind(scrutinee)
                    && let Some(c) = known.get(&v) =>
            {
                let (_, x, arm) = arms.into_iter().find(|(d, _, _)| d == c).ok_or_else(|| {
                    Error::Internal("a case names the constructor its scrutinee holds".into())
                })?;
                let mut inner = known.clone();
                inner.insert(x, *c);
                let arm = self.specialise(arm, &inner, &mut HashMap::new())?;
                self.add(Kind::Let(x, scrutinee, arm), self.ty(t))
            }
            kind => {
                let cs = self.t.children(t);
                let new: Vec<Tm> = cs
                    .iter()
                    .map(|c| self.specialise(*c, known, memo))
                    .collect::<Result<_, _>>()?;
                if new == cs {
                    t
                } else {
                    self.add(with_children(&kind, &new), self.ty(t))
                }
            }
        };
        memo.insert(t, out);
        Ok(out)
    }

    /// Returns `t` as a scalar expression at a point of a chain of `q`, with the bindings that compute what it reads.
    fn lift_here(
        &mut self,
        t: Tm,
        q: &mut QueryCx,
        sc: &Scope,
    ) -> Result<(Vec<(Var, Eval)>, Tm), Error> {
        let (lets, t) = self.in_chain(q, sc, |p| p.lift_eval(t))?;
        Ok((lets, self.here(t, sc)?))
    }

    /// Returns `f`'s result, with the bindings the queries it plans flatten into `q` added to `q`.
    fn in_chain<T>(&mut self, q: &mut QueryCx, sc: &Scope, f: impl FnOnce(&mut Self) -> T) -> T {
        self.outer.push(Outer {
            env: q.env.clone(),
            rows: sc.rows,
            lets: Vec::new(),
        });
        let out = f(self);
        let o = self.outer.pop().expect("the entry pushed above");
        q.lets.extend(o.lets);
        out
    }

    /// Returns how many conditions from the first are scalar and total.
    fn total_prefix(&mut self, conds: &[Tm]) -> usize {
        conds
            .iter()
            .take_while(|c| {
                let c = **c;
                self.scalar(c) && self.total(c)
            })
            .count()
    }

    fn eval(&mut self, t: Tm) -> Result<Eval, Error> {
        self.refuse(t)?;
        if self.scalar(t) {
            return Ok(Eval::Expr(t));
        }
        let ty = self.ty(t);
        match self.kind(t) {
            Kind::Fix(..) if self.bounded => Err(Error::Outside("fix")),
            Kind::Fix(binds, body) => self.fix(&binds, body),
            Kind::Let(x, e, b) => Ok(Eval::Let(
                x,
                Box::new(self.eval(e)?),
                Box::new(self.eval(b)?),
            )),
            Kind::If(c, a, b) => {
                let (lets, c) = self.lift_eval(c)?;
                let body = Eval::If(c, Box::new(self.eval(a)?), Box::new(self.eval(b)?));
                Ok(wrap_eval(lets, body))
            }
            Kind::Case { scrutinee, arms } => {
                let (lets, s) = self.lift_eval(scrutinee)?;
                let mut out = Vec::new();
                for (c, v, a) in arms {
                    out.push((c, v, self.eval(a)?));
                }
                Ok(wrap_eval(lets, Eval::Case(s, out)))
            }
            Kind::Try(e) => Ok(Eval::Try(Box::new(self.eval(e)?))),
            Kind::At {
                instant,
                point,
                pins,
                body,
            } => self.at(instant, point, pins, body),
            Kind::Deref { key, .. } => {
                let (lets, k) = self.lift_eval(key)?;
                let lookup = self.lookup(k, &ty)?;
                Ok(wrap_eval(lets, Eval::Lookup(lookup)))
            }
            Kind::Agg {
                op: Agg::User(a),
                var,
                source,
                body,
            } => {
                let sink = Sink::Fold {
                    var,
                    body: self.eval(body)?,
                    fold: self.fold(a, &ty)?,
                };
                self.query(source, sink, Some(&[body]))
            }
            Kind::Agg {
                op,
                var,
                source,
                body,
            } => {
                let body_ty = self.ty(body);
                let sink = Sink::Agg {
                    op,
                    var,
                    body: self.eval(body)?,
                    ty,
                    body_ty,
                };
                self.query(source, sink, Some(&[body]))
            }
            Kind::Exists(s) => self.query(s, Sink::Exists, Some(&[])),
            Kind::IsEmpty(s) => self.query(s, Sink::IsEmpty, Some(&[])),
            Kind::Group { var, source, by } => {
                let sink = Sink::Group {
                    var,
                    by: self.eval(by)?,
                    rows: true,
                    aggs: Vec::new(),
                };
                self.query(source, sink, Some(&[by]))
            }
            Kind::Top {
                count,
                keys,
                var,
                source,
            } => {
                let count = self.eval(count)?;
                let parts: Vec<Tm> = keys.iter().map(|(k, _)| *k).collect();
                let keys = self.keys(&keys)?;
                self.query(source, Sink::Top { count, keys, var }, Some(&parts))
            }
            Kind::Present {
                var,
                source,
                keys,
                take,
                after,
            } => {
                let parts: Vec<Tm> = keys.iter().map(|(k, _)| *k).collect();
                let keys = self.keys(&keys)?;
                let take = take.map(|x| self.eval(x)).transpose()?;
                let after = after.map(|x| self.eval(x)).transpose()?;
                self.query(
                    source,
                    Sink::Present {
                        var,
                        keys,
                        take,
                        after,
                    },
                    Some(&parts),
                )
            }
            _ if matches!(ty, Ty::Set(_) | Ty::NonEmpty(_)) && self.streams(t) => {
                self.query(t, Sink::Set, Some(&[]))
            }
            kind => {
                let cs = self.t.children(t);
                let mut last = None;
                for (i, c) in cs.iter().enumerate() {
                    if !self.scalar(*c) {
                        last = Some(i);
                    }
                }
                let last = last.expect("a term that is not scalar has a child that is not");
                let mut lets = Vec::new();
                let mut new = cs.clone();
                for (i, c) in cs.iter().enumerate().take(last + 1) {
                    let v = self.fresh("%t", self.ty(*c));
                    lets.push((v, self.eval(*c)?));
                    new[i] = self.var_tm(v);
                }
                let e = self.add(with_children(&kind, &new), ty);
                Ok(wrap_eval(lets, Eval::Expr(e)))
            }
        }
    }

    /// Returns the read of `body` at the state `point` names, planned apart from every query
    /// that holds it.
    fn at(&mut self, instant: bool, point: Tm, pins: Option<u64>, body: Tm) -> Result<Eval, Error> {
        if self.bounded {
            return Err(Error::Outside("at"));
        }
        if self.past {
            return Err(Error::NotAQuery("a read of the past inside another"));
        }
        let (lets, point) = self.lift_eval(point)?;
        let outer = std::mem::take(&mut self.outer);
        self.past = true;
        let body = self.eval(body);
        self.past = false;
        self.outer = outer;
        let body = body?;
        let mut reads = Reads::default();
        collect_eval(self.catalog, &body, &mut reads);
        let at = AtRead {
            instant,
            point,
            pins,
            body,
            heaps: reads.heaps,
        };
        Ok(wrap_eval(lets, Eval::At(Box::new(at))))
    }

    /// Returns whether a chain reads the set term `t` by its root form, without evaluating it first.
    fn streams(&self, t: Tm) -> bool {
        self.rel_source(t).is_some()
            || matches!(
                self.t.kind(t),
                Kind::EmptySet
                    | Kind::Singleton(_)
                    | Kind::Union(..)
                    | Kind::If(..)
                    | Kind::Case { .. }
                    | Kind::Let(..)
                    | Kind::For(..)
                    | Kind::Restrict(..)
            )
    }

    /// Refuses a term whose root form a plan does not compute.
    fn refuse(&self, t: Tm) -> Result<(), Error> {
        match self.t.kind(t) {
            Kind::Transition(..) => Err(Error::NotAQuery("a read of a statement's transition")),
            Kind::Lam(..) | Kind::App(..) => Err(Error::Internal(
                "a normal form holds no function value".into(),
            )),
            Kind::Skip
            | Kind::Insert(..)
            | Kind::Delete(..)
            | Kind::Replace(..)
            | Kind::Upsert { .. }
            | Kind::Rejects { .. }
            | Kind::Par(_)
            | Kind::Fail(_)
            | Kind::Fresh(..)
            | Kind::Only(_)
            | Kind::LetBang(..)
            | Kind::Block(..)
            | Kind::Transact(..) => Err(Error::NotAQuery("a write")),
            _ => Ok(()),
        }
    }

    /// Returns the parts of user-defined aggregate `a` copied into the plan's arena, for a result of type `ty`.
    fn fold(&mut self, a: u32, ty: &Ty) -> Result<Fold, Error> {
        let (schema, normal) = (self.schema, self.normal);
        let def = &schema.aggregates[a as usize];
        let refused = || Error::UserCode(def.name.clone());
        let parts = normal.aggregates[a as usize].as_ref().ok_or_else(refused)?;
        let (&[st, x], &[fin]) = (parts.step.0.as_slice(), parts.finish.0.as_slice()) else {
            return Err(Error::Internal(
                "an aggregate's step takes two values and its finish one".into(),
            ));
        };
        let state = self.import_var(st);
        let value = self.import_var(x);
        let mut vars = HashMap::from([(st, state), (x, value), (fin, state)]);
        let mut memo = HashMap::new();
        let init = self.import(parts.init, &mut vars, &mut memo);
        let step = self.import(parts.step.1, &mut vars, &mut memo);
        let finish = self.import(parts.finish.1, &mut vars, &mut memo);
        if ![init, step, finish].into_iter().all(|t| self.scalar(t)) {
            return Err(refused());
        }
        Ok(Fold {
            init,
            step,
            finish,
            state,
            value,
            optional: *ty != def.result,
        })
    }

    fn import_var(&mut self, v: Var) -> Var {
        let normal = self.normal;
        let info = &normal.terms.vars[v.0 as usize];
        self.t.var(&info.name, info.ty.clone())
    }

    /// Returns term `t` of the normal program's arena copied into the plan's, each variable renamed by `vars` or to a fresh one.
    fn import(&mut self, t: Tm, vars: &mut HashMap<Var, Var>, memo: &mut HashMap<Tm, Tm>) -> Tm {
        if let Some(n) = memo.get(&t) {
            return *n;
        }
        let normal = self.normal;
        let from = &normal.terms;
        let cs: Vec<Tm> = from
            .children(t)
            .into_iter()
            .map(|c| self.import(c, vars, memo))
            .collect();
        let kind = with_children(from.kind(t), &cs);
        let kind = with_vars(&kind, &mut |v| {
            *vars.entry(v).or_insert_with(|| self.import_var(v))
        });
        let out = self.add(kind, from.ty(t).clone());
        memo.insert(t, out);
        out
    }

    fn keys(&mut self, keys: &[(Tm, bool)]) -> Result<Vec<(Eval, bool)>, Error> {
        keys.iter().map(|(k, d)| Ok((self.eval(*k)?, *d))).collect()
    }

    /// Returns `t` as a scalar expression, with the bindings that compute what it reads.
    fn lift_eval(&mut self, t: Tm) -> Result<(Vec<(Var, Eval)>, Tm), Error> {
        if self.scalar(t) {
            return Ok((Vec::new(), t));
        }
        let v = self.fresh("%t", self.ty(t));
        let e = self.eval(t)?;
        Ok((vec![(v, e)], self.var_tm(v)))
    }

    /// Returns the query forming `sink` from the set `source`. `parts` are the sink's terms
    /// evaluated per element, `None` when the sink may abort on an element otherwise.
    fn query(&mut self, source: Tm, sink: Sink, parts: Option<&[Tm]>) -> Result<Eval, Error> {
        let Some((s, rows, source)) = self.flatten(source)? else {
            return self.flat_query(source, sink, parts);
        };
        let q = self.flat_query(source, sink, parts)?;
        Ok(Eval::Let(s, Box::new(rows), Box::new(q)))
    }

    fn flat_query(&mut self, source: Tm, sink: Sink, parts: Option<&[Tm]>) -> Result<Eval, Error> {
        let total = self.rw.early_stop
            && parts.is_some_and(|ps| ps.iter().all(|t| self.total(*t)))
            && self.total(source);
        let mut q = QueryCx {
            env: self.facts.free(&self.t, source),
            ..QueryCx::default()
        };
        let spine = self.spine(source, &mut q, &Scope::new())?;
        Ok(Eval::Query(Box::new(Query {
            builds: q.builds,
            spine: wrap_spine(q.lets, spine),
            sink,
            total,
        })))
    }

    /// Returns `source`, a comprehension over a constructor relation inside a chain, as a
    /// comprehension over the variable it binds first to the rows of one group of a grouping
    /// added to the spine of the query holding the chain, when the fixed cost model prefers it.
    fn flatten(&mut self, source: Tm) -> Result<Option<(Var, Eval, Tm)>, Error> {
        if !self.rw.flatten || self.bounded {
            return Ok(None);
        }
        let Some(outer) = self.outer.last() else {
            return Ok(None);
        };
        let (env, rows) = (outer.env.clone(), outer.rows);
        let Kind::For(y, src, body) = self.kind(source) else {
            return Ok(None);
        };
        let Some((r, ctors)) = self.rel_source(src) else {
            return Ok(None);
        };
        let (conds, rest) = self.peel(body);
        let n = self.total_prefix(&conds);
        let outside = |p: &mut Self, t: Tm| {
            p.facts
                .free(&p.t, t)
                .iter()
                .all(|v| *v == y || env.contains(v))
        };
        let (mut filters, mut keys, mut probes, mut kept) = (vec![], vec![], vec![], vec![]);
        for (i, c) in conds.iter().enumerate() {
            if i < n && self.mentions(*c, &[y]) && outside(self, *c) {
                filters.push(*c);
                continue;
            }
            if i < n
                && let Kind::Prim(Prim::Eq, args) = self.kind(*c)
                && let [a, b] = args[..]
                && erase(&self.ty(a)) == erase(&self.ty(b))
                && let Some((build, probe)) = [(a, b), (b, a)].into_iter().find(|(build, probe)| {
                    self.mentions(*build, &[y])
                        && outside(self, *build)
                        && !self.mentions(*probe, &[y])
                })
            {
                keys.push(build);
                probes.push(probe);
                continue;
            }
            kept.push(*c);
        }
        if keys.is_empty() {
            return Ok(None);
        }
        let heap_rows = ctors.len() as f64 * HEAP_ROWS;
        let per_row = match self.rw.index_scan {
            true => match self.index_for(r, &ctors, y, &conds, rest) {
                Some(p) if p.unique() => PROBE_COST,
                Some(_) => PROBE_COST + PROBE_ROWS,
                None => heap_rows,
            },
            false => heap_rows,
        };
        if heap_rows + rows >= rows * per_row {
            return Ok(None);
        }
        let (by, probe) = self.group_key(&keys, &probes);
        let row_ty = self.elem_ty(src)?;
        let set_ty = Ty::Set(Box::new(row_ty.clone()));
        let one = self.var_tm(y);
        let one = self.add(Kind::Singleton(one), set_ty.clone());
        let picked = self.guarded(&filters, one, &set_ty);
        let picked = self.add(Kind::For(y, src, picked), set_ty.clone());
        let group_ty = Ty::Record(vec![
            ("key".into(), self.ty(by)),
            ("rows".into(), Ty::NonEmpty(Box::new(row_ty))),
        ]);
        let sink = Sink::Group {
            var: y,
            by: Eval::Expr(by),
            rows: true,
            aggs: Vec::new(),
        };
        let grouped = self.flat_query(picked, sink, Some(&[by]))?;
        let g = self.fresh("%groups", Ty::Set(Box::new(group_ty)));
        self.outer
            .last_mut()
            .expect("the entry read above")
            .lets
            .push((g, grouped));
        let s = self.fresh("%rows", set_ty);
        let groups = self.var_tm(g);
        let body_ty = self.ty(body);
        let each = self.guarded(&kept, rest, &body_ty);
        let from = self.var_tm(s);
        let source = self.add(Kind::For(y, from, each), self.ty(source));
        Ok(Some((s, Eval::GroupRows { groups, key: probe }, source)))
    }

    /// Returns the key a grouping keys its rows by and the key a probe of it looks up: the one
    /// term of each, or a record of them.
    fn group_key(&mut self, keys: &[Tm], probes: &[Tm]) -> (Tm, Tm) {
        if let ([k], [p]) = (keys, probes) {
            return (*k, *p);
        }
        let names: Vec<String> = (0..keys.len()).map(|i| format!("k{i:04}")).collect();
        let record = |p: &mut Self, ts: &[Tm]| {
            let fields: Vec<(String, Tm)> = names.iter().cloned().zip(ts.iter().copied()).collect();
            let ty = Ty::Record(fields.iter().map(|(n, t)| (n.clone(), p.ty(*t))).collect());
            p.add(Kind::Record(fields), ty)
        };
        (record(self, keys), record(self, probes))
    }

    fn spine(&mut self, t: Tm, q: &mut QueryCx, sc: &Scope) -> Result<Spine, Error> {
        self.refuse(t)?;
        match self.kind(t) {
            Kind::Union(a, b) => {
                let mut parts = Vec::new();
                for x in [a, b] {
                    match self.spine(x, q, sc)? {
                        Spine::Union(ps) => parts.extend(ps),
                        p => parts.push(p),
                    }
                }
                Ok(Spine::Union(parts))
            }
            Kind::EmptySet => Ok(Spine::Union(Vec::new())),
            Kind::If(c, a, b) => {
                let (lets, c) = self.lift_eval(c)?;
                let s = match self.literal(c) {
                    Some(true) => self.spine(a, q, sc)?,
                    Some(false) => self.spine(b, q, sc)?,
                    None => Spine::If(
                        c,
                        Box::new(self.spine(a, q, sc)?),
                        Box::new(self.spine(b, q, sc)?),
                    ),
                };
                Ok(wrap_spine(lets, s))
            }
            Kind::Case { scrutinee, arms } => {
                let (lets, s) = self.lift_eval(scrutinee)?;
                let mut out = Vec::new();
                for (c, v, a) in arms {
                    let inner = sc.bind_known(v, Some(c));
                    out.push((c, v, self.spine(a, q, &inner)?));
                }
                Ok(wrap_spine(lets, Spine::Case(s, out)))
            }
            Kind::Let(x, e, b) => {
                let e = self.eval(e)?;
                Ok(Spine::Let(x, e, Box::new(self.spine(b, q, &sc.bind(x))?)))
            }
            Kind::For(x, src, body) => {
                if let Some((r, ctors)) = self.rel_source(src) {
                    if self.rw.unswitch
                        && let Some(u) = self.unswitch(t, x, src, body)
                    {
                        return self.spine(u, q, sc);
                    }
                    return self.scan(r, &ctors, x, body, q, sc);
                }
                Ok(Spine::Pipeline(Pipeline {
                    source: Source::Unit,
                    chain: self.chain(t, q, sc)?,
                }))
            }
            _ => Ok(Spine::Pipeline(Pipeline {
                source: Source::Unit,
                chain: self.chain(t, q, sc)?,
            })),
        }
    }

    /// Returns the chain applying `conds` in order, then `rest`.
    fn filtered(
        &mut self,
        conds: &[Tm],
        rest: Tm,
        q: &mut QueryCx,
        sc: &Scope,
    ) -> Result<Chain, Error> {
        let mut sc = sc.clone();
        let mut steps = Vec::new();
        let mut silent = true;
        for c in conds {
            let mut c = *c;
            while self.rw.static_route
                && let Kind::Case { scrutinee, arms } = self.kind(c)
                && let Kind::Var(v) = self.kind(scrutinee)
                && let Some(&k) = sc.known.get(&v)
            {
                let (_, x, arm) = arms.into_iter().find(|(d, _, _)| *d == k).ok_or_else(|| {
                    Error::Internal("a case names the constructor its scrutinee holds".into())
                })?;
                steps.push(Step::Bind(x, Eval::Expr(scrutinee)));
                sc = sc.bind_known(x, Some(k));
                c = arm;
            }
            match self.literal(c) {
                Some(true) => continue,
                Some(false) if silent => return Ok(nothing()),
                Some(false) => return Ok(chain(steps, End::Nothing)),
                None => {}
            }
            silent = silent && self.scalar(c) && self.total(c);
            let (lets, c) = self.lift_here(c, q, &sc)?;
            steps.extend(binds(lets));
            steps.push(Step::Filter(c));
        }
        let rest = self.chain(rest, q, &sc)?;
        if silent && is_nothing(&rest) {
            return Ok(nothing());
        }
        Ok(prepend(steps, rest))
    }

    fn chain(&mut self, t: Tm, q: &mut QueryCx, sc: &Scope) -> Result<Chain, Error> {
        self.refuse(t)?;
        match self.kind(t) {
            Kind::EmptySet => Ok(nothing()),
            Kind::Singleton(e) => {
                let (lets, e) = self.lift_here(e, q, sc)?;
                Ok(chain(binds(lets), End::Emit(e)))
            }
            Kind::Union(a, b) => {
                let mut parts = Vec::new();
                for x in [a, b] {
                    let c = self.chain(x, q, sc)?;
                    match c {
                        Chain {
                            steps,
                            end: End::Fork(cs),
                        } if steps.is_empty() => parts.extend(cs),
                        c if is_nothing(&c) && self.rw.prune => {}
                        c => parts.push(c),
                    }
                }
                Ok(match parts.len() {
                    0 => nothing(),
                    1 => parts.pop().expect("one chain"),
                    _ => chain(Vec::new(), End::Fork(parts)),
                })
            }
            Kind::If(_, _, b) if self.is_empty_set(b) => {
                let (conds, rest) = self.peel(t);
                self.filtered(&conds, rest, q, sc)
            }
            Kind::If(c, a, b) => {
                let (lets, c) = self.lift_here(c, q, sc)?;
                let end = match self.literal(c) {
                    Some(true) => return Ok(prepend(binds(lets), self.chain(a, q, sc)?)),
                    Some(false) => return Ok(prepend(binds(lets), self.chain(b, q, sc)?)),
                    None => End::If(
                        c,
                        Box::new(self.chain(a, q, sc)?),
                        Box::new(self.chain(b, q, sc)?),
                    ),
                };
                Ok(chain(binds(lets), end))
            }
            Kind::Case { scrutinee, arms } => {
                if self.rw.static_route
                    && let Kind::Var(v) = self.kind(scrutinee)
                    && let Some(&c) = sc.known.get(&v)
                {
                    let (_, x, body) =
                        arms.iter()
                            .find(|(d, _, _)| *d == c)
                            .cloned()
                            .ok_or_else(|| {
                                Error::Internal(
                                    "a case names the constructor its scrutinee holds".into(),
                                )
                            })?;
                    let rest = self.chain(body, q, &sc.bind_known(x, Some(c)))?;
                    return Ok(prepend(vec![Step::Bind(x, Eval::Expr(scrutinee))], rest));
                }
                let (lets, s) = self.lift_here(scrutinee, q, sc)?;
                let mut out = Vec::new();
                for (c, x, body) in arms {
                    out.push((c, x, self.chain(body, q, &sc.bind_known(x, Some(c)))?));
                }
                Ok(chain(binds(lets), End::Case(s, out)))
            }
            Kind::Let(x, e, b) => {
                let e = self.in_chain(q, sc, |p| p.eval(e))?;
                let rest = self.chain(b, q, &sc.bind(x))?;
                Ok(prepend(vec![Step::Bind(x, e)], rest))
            }
            Kind::For(y, src, body) => self.for_chain(y, src, body, q, sc),
            _ => {
                let y = self.fresh("%e", self.elem_ty(t)?);
                let body = self.var_tm(y);
                let body = self.add(Kind::Singleton(body), self.ty(t));
                self.for_chain(y, t, body, q, sc)
            }
        }
    }

    fn elem_ty(&self, set: Tm) -> Result<Ty, Error> {
        match self.t.ty(set) {
            Ty::Set(e) | Ty::NonEmpty(e) => Ok((**e).clone()),
            other => Err(Error::Internal(format!(
                "a comprehension ranges over a set, not {other:?}"
            ))),
        }
    }

    fn for_chain(
        &mut self,
        y: Var,
        src: Tm,
        body: Tm,
        q: &mut QueryCx,
        sc: &Scope,
    ) -> Result<Chain, Error> {
        self.refuse(src)?;
        if let Some((r, ctors)) = self.rel_source(src) {
            if (self.rw.unswitch || self.bounded)
                && let Some(split) = self.split_join(y, src, body)
            {
                return self.chain(split, q, sc);
            }
            return self.join(r, &ctors, y, body, q, sc);
        }
        if self.rw.group_agg
            && !self.bounded
            && let Some(c) = self.fused_group(y, src, body, q, sc)?
        {
            return Ok(c);
        }
        if let Kind::Restrict(inner, s) = self.kind(src)
            && !self.scalar(inner)
        {
            let z = self.fresh("%e", self.elem_ty(inner)?);
            let one = self.var_tm(z);
            let one = self.add(Kind::Singleton(one), self.ty(inner));
            let kept = self.add(Kind::Restrict(one, s), self.ty(src));
            let each = self.add(Kind::For(y, kept, body), self.ty(body));
            return self.for_chain(z, inner, each, q, sc);
        }
        let (lets, s) = self.lift_here(src, q, sc)?;
        let mut steps = binds(lets);
        steps.push(Step::Unnest(y, s));
        let rest = self.chain(body, q, &sc.bind(y).times(SET_ROWS))?;
        Ok(prepend(steps, rest))
    }

    /// Returns `for x in src do body` with a condition or a scrutinee that does not depend on `x` taken out of the loop, when there is one.
    fn unswitch(&mut self, t: Tm, x: Var, src: Tm, body: Tm) -> Option<Tm> {
        let ty = self.ty(t);
        let (conds, rest) = self.peel(body);
        let n = self.total_prefix(&conds);
        let invariant = |p: &mut Self, c: Tm| p.scalar(c) && p.total(c) && !p.mentions(c, &[x]);
        for i in 0..conds.len() {
            let c = conds[i];
            if i <= n && invariant(self, c) {
                let mut others = conds.clone();
                others.remove(i);
                let inner = self.guarded(&others, rest, &ty);
                let looped = self.add(Kind::For(x, src, inner), ty.clone());
                return Some(self.guarded(&[c], looped, &ty));
            }
            if let Kind::Case { scrutinee, arms } = self.kind(c)
                && invariant(self, scrutinee)
            {
                let mut out = Vec::new();
                for (k, v, arm) in arms {
                    let mut cs = conds.clone();
                    cs[i] = arm;
                    let inner = self.guarded(&cs, rest, &ty);
                    out.push((k, v, self.add(Kind::For(x, src, inner), ty.clone())));
                }
                return Some(self.add(
                    Kind::Case {
                        scrutinee,
                        arms: out,
                    },
                    ty,
                ));
            }
        }
        if n < conds.len() {
            return None;
        }
        match self.kind(rest) {
            Kind::Case { scrutinee, arms } if invariant(self, scrutinee) => {
                let mut out = Vec::new();
                for (k, v, arm) in arms {
                    let inner = self.guarded(&conds, arm, &ty);
                    out.push((k, v, self.add(Kind::For(x, src, inner), ty.clone())));
                }
                Some(self.add(
                    Kind::Case {
                        scrutinee,
                        arms: out,
                    },
                    ty,
                ))
            }
            Kind::If(c, a, b) if invariant(self, c) => {
                let a = self.guarded(&conds, a, &ty);
                let b = self.guarded(&conds, b, &ty);
                let a = self.add(Kind::For(x, src, a), ty.clone());
                let b = self.add(Kind::For(x, src, b), ty.clone());
                Some(self.add(Kind::If(c, a, b), ty))
            }
            _ => None,
        }
    }

    fn fix(&mut self, binds: &[(Var, Tm)], body: Tm) -> Result<Eval, Error> {
        let xs: Vec<Var> = binds.iter().map(|(x, _)| *x).collect();
        let deltas: Vec<(Var, Var)> = binds
            .iter()
            .map(|(x, _)| {
                let ty = self.t.var_info(*x).ty.clone();
                (*x, self.fresh("%delta", ty))
            })
            .collect();
        let mut steps = Vec::new();
        if self.rw.semi_naive {
            for (_, e) in binds {
                steps.push(self.delta(*e, &xs, &deltas));
            }
        }
        let semi = self.rw.semi_naive && steps.iter().all(Option::is_some);
        let mut out = Vec::new();
        for (i, (x, e)) in binds.iter().enumerate() {
            let step = if semi {
                Some(self.eval(steps[i].expect("every step exists"))?)
            } else {
                None
            };
            out.push(FixBind {
                var: *x,
                body: self.eval(*e)?,
                delta: deltas[i].1,
                step,
            });
        }
        Ok(Eval::Fix(Box::new(Fix {
            binds: out,
            body: self.eval(body)?,
        })))
    }

    /// Returns a set term holding every element `e` gains when the variables `xs` gain their deltas, over the grown variables, or `None` when an occurrence of them is not a generator or a branch.
    fn delta(&mut self, e: Tm, xs: &[Var], deltas: &[(Var, Var)]) -> Option<Tm> {
        let ty = self.ty(e);
        if !self.mentions(e, xs) {
            return Some(self.empty(ty));
        }
        let union = |p: &mut Self, a: Tm, b: Tm| {
            if p.is_empty_set(a) {
                b
            } else if p.is_empty_set(b) {
                a
            } else {
                p.add(Kind::Union(a, b), ty.clone())
            }
        };
        match self.kind(e) {
            Kind::Var(v) => {
                let d = deltas.iter().find(|(x, _)| *x == v)?.1;
                Some(self.var_tm(d))
            }
            Kind::Union(a, b) => {
                let a = self.delta(a, xs, deltas)?;
                let b = self.delta(b, xs, deltas)?;
                Some(union(self, a, b))
            }
            Kind::Restrict(a, s) => {
                let a = self.delta(a, xs, deltas)?;
                Some(self.add(Kind::Restrict(a, s), ty))
            }
            Kind::For(y, s, b) => {
                let ds = self.delta(s, xs, deltas)?;
                let db = self.delta(b, xs, deltas)?;
                let left = if self.is_empty_set(ds) {
                    ds
                } else {
                    self.add(Kind::For(y, ds, b), ty.clone())
                };
                let right = if self.is_empty_set(db) {
                    db
                } else {
                    self.add(Kind::For(y, s, db), ty.clone())
                };
                Some(union(self, left, right))
            }
            Kind::If(c, a, b) if !self.mentions(c, xs) => {
                let a = self.delta(a, xs, deltas)?;
                let b = self.delta(b, xs, deltas)?;
                Some(self.add(Kind::If(c, a, b), ty))
            }
            Kind::Case { scrutinee, arms } if !self.mentions(scrutinee, xs) => {
                let mut out = Vec::new();
                for (c, v, a) in arms {
                    out.push((c, v, self.delta(a, xs, deltas)?));
                }
                Some(self.add(
                    Kind::Case {
                        scrutinee,
                        arms: out,
                    },
                    ty,
                ))
            }
            Kind::Let(y, a, b) if !self.mentions(a, xs) => {
                let b = self.delta(b, xs, deltas)?;
                Some(self.add(Kind::Let(y, a, b), ty))
            }
            Kind::Fix(inner, b) if inner.iter().all(|(_, x)| !self.mentions(*x, xs)) => {
                let b = self.delta(b, xs, deltas)?;
                Some(self.add(Kind::Fix(inner, b), ty))
            }
            _ => None,
        }
    }
}
