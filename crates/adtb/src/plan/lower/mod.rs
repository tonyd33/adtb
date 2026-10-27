use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use adtb_lang::analysis::Totality;
use adtb_lang::kernel::{Agg, Kind, PathStep, Prim, Terms, Tm, Var};
use adtb_lang::norm::NormalProgram;
use adtb_lang::schema::Schema;
use adtb_lang::types::{CtorSet, RelId, Ty};

use super::catalog::{Catalog, Element, IndexDesc};
use super::terms::{Facts, with_children, with_vars};
use super::*;

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
    let mut p = Planner {
        schema,
        normal,
        catalog,
        rw: match forced() {
            Some(rw) if !bounded => rw,
            _ => options.rewrites,
        },
        bounded,
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

struct Planner<'a> {
    schema: &'a Schema,
    normal: &'a NormalProgram,
    catalog: &'a Catalog,
    rw: Rewrites,
    /// Set to read every constructor relation only through an index probe.
    bounded: bool,
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
    fn peel(&self, mut t: Tm) -> (Vec<Tm>, Tm) {
        let mut conds = Vec::new();
        while let Kind::If(c, a, b) = self.t.kind(t)
            && self.is_empty_set(*b)
        {
            self.conjuncts(*c, &mut conds);
            t = *a;
        }
        (conds, t)
    }

    fn conjuncts(&self, c: Tm, out: &mut Vec<Tm>) {
        match self.t.kind(c) {
            Kind::If(a, b, f) if self.literal(*f) == Some(false) => {
                self.conjuncts(*a, out);
                self.conjuncts(*b, out);
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
            Kind::Call { func, .. } => {
                Err(Error::UserCode(self.schema.function(*func).name.clone()))
            }
            Kind::At { .. } => Err(Error::NotAQuery("a read of history")),
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

    fn lookup(&mut self, key: Tm, row_ty: &Ty) -> Result<Lookup, Error> {
        let Ty::Key { rel, ctors } = self.ty(key) else {
            return Err(Error::Internal("a dereference follows a key".into()));
        };
        let ctors = match row_ty {
            Ty::Data { ctors: Some(s), .. } => s.clone(),
            _ => self.ctors_of(rel, &ctors),
        };
        let desc = self.catalog.relation(rel);
        Ok(Lookup {
            index: desc.key_index,
            key: self
                .key_values(rel, key)
                .into_iter()
                .map(|(_, k)| k)
                .collect(),
            routes: ctors
                .iter()
                .map(|c| (*c, self.catalog.heap(rel, *c)))
                .collect(),
        })
    }

    /// Returns the value of each key field of `rel` in the key `k`.
    fn key_values(&mut self, rel: RelId, k: Tm) -> Vec<(String, Tm)> {
        let r = self.schema.relation(rel);
        let def = self.schema.ty(r.elem_def);
        let args = match &r.elem {
            Ty::Data { args, .. } => args.clone(),
            _ => Vec::new(),
        };
        let field_ty = |f: &str| {
            def.field(0, f)
                .unwrap_or_else(|| panic!("key field `{f}` is a field of the relation's rows"))
                .ty
                .subst(&args)
        };
        match r.key.as_slice() {
            [f] if matches!(field_ty(f), Ty::Key { rel: own, .. } if own == rel) => {
                vec![(f.clone(), k)]
            }
            [f] => {
                let ty = field_ty(f);
                vec![(f.clone(), self.add(Kind::Prim(Prim::KeyValue, vec![k]), ty))]
            }
            fs => {
                let mut tys: Vec<(String, Ty)> =
                    fs.iter().map(|f| (f.clone(), field_ty(f))).collect();
                tys.sort_by(|a, b| a.0.cmp(&b.0));
                let rec = self.add(Kind::Prim(Prim::KeyValue, vec![k]), Ty::Record(tys.clone()));
                fs.iter()
                    .map(|f| {
                        let ty = field_ty(f);
                        (f.clone(), self.add(Kind::Field(rec, f.clone()), ty))
                    })
                    .collect()
            }
        }
    }

    /// Returns what of a row `x` of `rel` condition `c` fixes, each with its value, when `c` is an
    /// equality whose other side mentions neither `x` nor a variable of `arms`, each the value at
    /// a path from `x`.
    fn equality(
        &mut self,
        rel: RelId,
        c: Tm,
        x: Var,
        arms: &HashMap<Var, Vec<PathStep>>,
    ) -> Option<Fixes> {
        let Kind::Prim(Prim::Eq, args) = self.kind(c) else {
            return None;
        };
        let [a, b] = args[..] else { return None };
        if erase(&self.ty(a)) != erase(&self.ty(b)) {
            return None;
        }
        let bound: Vec<Var> = std::iter::once(x).chain(arms.keys().copied()).collect();
        for (this, other) in [(a, b), (b, a)] {
            if self.mentions(other, &bound) || !self.mentions(this, &bound) {
                continue;
            }
            if let Kind::Key(row) = self.kind(this)
                && matches!(self.t.kind(row), Kind::Var(v) if *v == x)
            {
                let fields = self.key_values(rel, other).into_iter();
                return Some(fields.map(|(f, v)| (Fixed::field(f, v), v)).collect());
            }
            let path = self.path_of(this, x, arms).filter(|p| !p.is_empty());
            return Some(vec![(Fixed { path, term: this }, other)]);
        }
        None
    }

    /// Returns the path from `x` that `t` reads, each variable of `arms` standing for the value at
    /// its path.
    fn path_of(&self, t: Tm, x: Var, arms: &HashMap<Var, Vec<PathStep>>) -> Option<Vec<PathStep>> {
        match self.t.kind(t) {
            Kind::Var(v) if *v == x => Some(Vec::new()),
            Kind::Var(v) => arms.get(v).cloned(),
            Kind::Field(row, f) => {
                let mut p = self.path_of(*row, x, arms)?;
                p.push(PathStep::Field(f.clone()));
                Some(p)
            }
            _ => None,
        }
    }

    /// Add to `out` what of row `x` of `rel` every result of set term `t` needs fixed: under a
    /// case over a path from `x` with one arm that is not empty, the total conditions guarding
    /// that arm's results, and so on inside it, and inside the source of a comprehension over
    /// such a case.
    fn arm_equalities(
        &mut self,
        rel: RelId,
        t: Tm,
        x: Var,
        arms: &mut HashMap<Var, Vec<PathStep>>,
        out: &mut Fixes,
    ) {
        match self.kind(t) {
            Kind::For(_, src, _) if self.rel_source(src).is_none() => {
                self.arm_equalities(rel, src, x, arms, out);
            }
            Kind::Case { .. } => {
                let Some(body) = self.live_arm(t, x, arms, Planner::is_empty_set) else {
                    return;
                };
                let (conds, rest) = self.peel(body);
                let n = self.total_prefix(&conds);
                for cond in &conds[..n] {
                    self.cond_equalities(rel, *cond, x, arms, out);
                }
                self.arm_equalities(rel, rest, x, arms, out);
            }
            _ => {}
        }
    }

    /// Add to `out` what of row `x` of `rel` total condition `c` needs fixed to hold: an
    /// equality, the conjuncts of a conjunction, or under a case over a path from `x` with one
    /// arm that is not false, what that arm needs.
    fn cond_equalities(
        &mut self,
        rel: RelId,
        c: Tm,
        x: Var,
        arms: &mut HashMap<Var, Vec<PathStep>>,
        out: &mut Fixes,
    ) {
        if let Some(pairs) = self.equality(rel, c, x, arms) {
            out.extend(pairs);
            return;
        }
        let Some(body) = self.live_arm(c, x, arms, |p, a| p.literal(a) == Some(false)) else {
            return;
        };
        let from = out.len();
        let mut conjuncts = Vec::new();
        self.conjuncts(body, &mut conjuncts);
        for d in conjuncts {
            self.cond_equalities(rel, d, x, arms, out);
        }
        if let Kind::Case { scrutinee, .. } = self.kind(c)
            && let Some(path) = self.path_of(scrutinee, x, arms)
            && let Some(at) = self.arm_ctor(c, |p, a| p.literal(a) == Some(false))
        {
            self.whole_ctor(scrutinee, &path, at, from, out);
        }
    }

    /// Returns the body of the one arm of case `t` over a path from `x` that `dead` does not
    /// hold of, binding its variable in `arms` to the path through its constructor.
    fn live_arm(
        &self,
        t: Tm,
        x: Var,
        arms: &mut HashMap<Var, Vec<PathStep>>,
        dead: impl Fn(&Self, Tm) -> bool,
    ) -> Option<Tm> {
        let Kind::Case {
            scrutinee,
            arms: cases,
        } = self.t.kind(t)
        else {
            return None;
        };
        let mut path = self
            .path_of(*scrutinee, x, arms)
            .filter(|p| !p.is_empty())?;
        let live: Vec<&(u32, Var, Tm)> = cases.iter().filter(|(_, _, a)| !dead(self, *a)).collect();
        let [(c, v, body)] = live[..] else {
            return None;
        };
        path.push(PathStep::Ctor(*c));
        arms.insert(*v, path);
        Some(*body)
    }

    /// Returns whether `fixed`, of a row `x` of `rel`, is key element `e` of one of its indexes.
    fn is_element(&self, rel: RelId, x: Var, e: &Element, fixed: &Fixed) -> bool {
        match e {
            Element::Field(f) => {
                matches!(fixed.path.as_deref(), Some([PathStep::Field(g)]) if g == f)
            }
            Element::Derived { index, element } => {
                let (x0, f) = &self.normal.index_fields[&(rel, *index, *element)];
                if !f.path.is_empty() {
                    return fixed.path.as_ref() == Some(&f.path);
                }
                let mut vars = HashMap::from([(*x0, x)]);
                self.same_term(fixed.term, f.value, &mut vars)
            }
        }
    }

    /// Returns whether term `a` of the plan's arena is term `b` of the normal program's, each
    /// variable of `b` renamed by `vars` or bound alike in both.
    fn same_term(&self, a: Tm, b: Tm, vars: &mut HashMap<Var, Var>) -> bool {
        let from = &self.normal.terms;
        let (ca, cb) = (self.t.children(a), from.children(b));
        if ca.len() != cb.len() || erase(self.t.ty(a)) != erase(from.ty(b)) {
            return false;
        }
        let zeros = vec![Tm(0); ca.len()];
        let ka = with_children(self.t.kind(a), &zeros);
        let kb = with_children(from.kind(b), &zeros);
        let (mut va, mut vb) = (Vec::new(), Vec::new());
        with_vars(&ka, &mut |v| {
            va.push(v);
            v
        });
        with_vars(&kb, &mut |v| {
            vb.push(v);
            v
        });
        if va.len() != vb.len() {
            return false;
        }
        for (u, w) in vb.iter().zip(&va) {
            if *vars.entry(*u).or_insert(*w) != *w {
                return false;
            }
        }
        if with_vars(&kb, &mut |v| vars[&v]) != ka {
            return false;
        }
        ca.into_iter()
            .zip(cb)
            .all(|(x, y)| self.same_term(x, y, vars))
    }

    /// Returns the best index of `rel` covering `ctors` whose elements the total prefix of
    /// `conds`, or the arms of `rest` as [`Planner::arm_equalities`] reads them, fix for row `x`.
    fn index_for(
        &mut self,
        rel: RelId,
        ctors: &CtorSet,
        x: Var,
        conds: &[Tm],
        rest: Tm,
    ) -> Option<IndexProbe> {
        let n = self.total_prefix(conds);
        let mut fixed: Vec<(Option<usize>, Fixes)> = Vec::new();
        let mut in_arms = Vec::new();
        for (i, c) in conds.iter().enumerate().take(n) {
            match self.equality(rel, *c, x, &HashMap::new()) {
                Some(pairs) => fixed.push((Some(i), pairs)),
                None => self.cond_equalities(rel, *c, x, &mut HashMap::new(), &mut in_arms),
            }
        }
        self.arm_equalities(rel, rest, x, &mut HashMap::new(), &mut in_arms);
        if !in_arms.is_empty() {
            fixed.push((None, in_arms));
        }
        let limits: Vec<Limit> = if self.rw.range_probe && !self.bounded {
            conds[..n]
                .iter()
                .filter_map(|c| self.comparison(*c, x))
                .collect()
        } else {
            Vec::new()
        };
        if fixed.is_empty() && limits.is_empty() {
            return None;
        }
        let mut best: Option<((bool, usize, bool), IndexProbe)> = None;
        for index in &self.catalog.relation(rel).indexes {
            if !ctors.iter().all(|c| index.ctors.contains(c)) {
                continue;
            }
            let mut key = Vec::new();
            for e in &index.elements {
                let found = fixed
                    .iter()
                    .flat_map(|(_, ps)| ps)
                    .find(|(f, _)| self.is_element(rel, x, e, f));
                match found {
                    Some((_, v)) => key.push(*v),
                    None => break,
                }
            }
            let full = key.len() == index.elements.len();
            let range = match index.elements.get(key.len()) {
                Some(Element::Field(f)) if index.ordered => {
                    let bound = |below: bool| {
                        limits
                            .iter()
                            .find(|(g, b, _)| g == f && *b == below)
                            .map(|l| l.2)
                    };
                    let span = Span {
                        lo: bound(true),
                        hi: bound(false),
                    };
                    (span.lo.is_some() || span.hi.is_some()).then_some(span)
                }
                _ => None,
            };
            if (key.is_empty() && range.is_none()) || (!full && !index.ordered) {
                continue;
            }
            let covered = &index.elements[..key.len()];
            let used = fixed
                .iter()
                .filter(|(_, ps)| {
                    ps.iter().all(|(f, v)| {
                        covered.iter().zip(&key).any(|(e, k)| {
                            matches!(e, Element::Field(_))
                                && self.is_element(rel, x, e, f)
                                && k == v
                        })
                    })
                })
                .filter_map(|(i, _)| *i)
                .collect();
            let rank = (full && index.unique, key.len(), range.is_some());
            if best.as_ref().is_none_or(|b| rank > b.0) {
                let probe = IndexProbe {
                    index: index.clone(),
                    key,
                    range,
                    used,
                };
                best = Some((rank, probe));
            }
        }
        best.map(|(_, p)| p)
    }

    /// Returns the bound comparison `c` puts on a field of row `x`, when it compares the field with
    /// a term of the same type that does not mention `x`.
    fn comparison(&mut self, c: Tm, x: Var) -> Option<Limit> {
        let Kind::Prim(op, args) = self.kind(c) else {
            return None;
        };
        let (below, inclusive) = match op {
            Prim::Gt => (true, false),
            Prim::Ge => (true, true),
            Prim::Lt => (false, false),
            Prim::Le => (false, true),
            _ => return None,
        };
        let [a, b] = args[..] else { return None };
        if erase(&self.ty(a)) != erase(&self.ty(b)) {
            return None;
        }
        for (this, other, below) in [(a, b, below), (b, a, !below)] {
            if let Kind::Field(row, f) = self.kind(this)
                && matches!(self.t.kind(row), Kind::Var(v) if *v == x)
                && !self.mentions(other, &[x])
            {
                let bound = RangeBound {
                    value: other,
                    inclusive,
                };
                return Some((f, below, bound));
            }
        }
        None
    }

    /// Returns the refusal of a read of the rows `x` of `ctors` of `rel` under `conds` that no
    /// index probe reaches.
    fn unbounded(&mut self, rel: RelId, ctors: &CtorSet, x: Var, conds: &[Tm]) -> Error {
        let n = self.total_prefix(conds);
        let mut fields: Vec<String> = Vec::new();
        for c in &conds[..n] {
            for (f, _) in self
                .equality(rel, *c, x, &HashMap::new())
                .unwrap_or_default()
            {
                if let Some([PathStep::Field(f)]) = f.path.as_deref()
                    && !fields.contains(f)
                {
                    fields.push(f.clone());
                }
            }
        }
        Error::Unbounded {
            rel,
            ctors: ctors.clone(),
            fields,
        }
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

    /// Returns the pipelines reading the rows `x` of `ctors` of `r` at the start of a query.
    fn scan(
        &mut self,
        r: RelId,
        ctors: &CtorSet,
        x: Var,
        body: Tm,
        q: &mut QueryCx,
        sc: &Scope,
    ) -> Result<Spine, Error> {
        let (conds, rest) = self.pulled(body);
        if (self.rw.index_scan || self.bounded)
            && let Some(probe) = self.index_for(r, ctors, x, &conds, rest)
        {
            let rows = if probe.unique() { 1.0 } else { PROBE_ROWS };
            let IndexProbe {
                index,
                key,
                range,
                used,
            } = probe;
            let left: Vec<Tm> = (0..conds.len())
                .filter(|i| !used.contains(i))
                .map(|i| conds[i])
                .collect();
            let routes = self.routes(
                r,
                ctors,
                &index.ctors,
                x,
                &left,
                rest,
                q,
                &sc.clone().times(rows),
            )?;
            let ends = self.index_ends(&index, key, range, x, routes);
            return Ok(union(
                ends.into_iter()
                    .map(|end| {
                        Spine::Pipeline(Pipeline {
                            source: Source::Unit,
                            chain: chain(Vec::new(), end),
                        })
                    })
                    .collect(),
            ));
        }
        if self.bounded {
            return Err(self.unbounded(r, ctors, x, &conds));
        }
        let heaps = if self.rw.prune {
            ctors.clone()
        } else {
            self.all_ctors(r)
        };
        let mut out = Vec::new();
        for c in heaps {
            let inner = sc.bind_known(x, Some(c)).times(HEAP_ROWS);
            let ch = if ctors.contains(&c) {
                self.filtered(&conds, rest, q, &inner)?
            } else {
                nothing()
            };
            if self.rw.prune && is_nothing(&ch) {
                continue;
            }
            out.push(Spine::Pipeline(Pipeline {
                source: Source::Scan {
                    heap: self.catalog.heap(r, c),
                    ctor: c,
                    var: x,
                },
                chain: ch,
            }));
        }
        Ok(union(out))
    }

    /// Returns one route per constructor an index probe reaches, each continuing with `conds` then `rest`.
    #[expect(
        clippy::too_many_arguments,
        reason = "the probe's relation, constructor sets, binder and continuation would have to travel as one struct"
    )]
    fn routes(
        &mut self,
        r: RelId,
        ctors: &CtorSet,
        covered: &CtorSet,
        x: Var,
        conds: &[Tm],
        rest: Tm,
        q: &mut QueryCx,
        sc: &Scope,
    ) -> Result<Vec<Route>, Error> {
        let reached = if self.rw.prune { ctors } else { covered };
        let mut out = Vec::new();
        for c in reached {
            let ch = if ctors.contains(c) {
                self.filtered(conds, rest, q, &sc.bind_known(x, Some(*c)))?
            } else {
                nothing()
            };
            if self.rw.prune && is_nothing(&ch) {
                continue;
            }
            out.push(Route {
                ctor: *c,
                heap: self.catalog.heap(r, *c),
                chain: ch,
            });
        }
        Ok(out)
    }

    /// Returns the ends probing `index`: one routing to every route, or one per route.
    fn index_ends(
        &mut self,
        index: &IndexDesc,
        key: Vec<Tm>,
        range: Option<Span>,
        x: Var,
        routes: Vec<Route>,
    ) -> Vec<End> {
        let end = |routes| match range {
            Some(Span { lo, hi }) => End::Range {
                index: index.id,
                key: key.clone(),
                lo,
                hi,
                var: x,
                routes,
            },
            None => End::Index {
                index: index.id,
                key: key.clone(),
                var: x,
                routes,
            },
        };
        if routes.is_empty() {
            Vec::new()
        } else if self.rw.fuse_probe {
            vec![end(routes)]
        } else {
            routes.into_iter().map(|r| end(vec![r])).collect()
        }
    }

    /// Returns `s` with its bounds specialised as [`Planner::here`] does.
    fn span_here(&mut self, s: Span, sc: &Scope) -> Result<Span, Error> {
        let mut at = |b: Option<RangeBound>| -> Result<Option<RangeBound>, Error> {
            b.map(|b| {
                Ok(RangeBound {
                    value: self.here(b.value, sc)?,
                    inclusive: b.inclusive,
                })
            })
            .transpose()
        };
        Ok(Span {
            lo: at(s.lo)?,
            hi: at(s.hi)?,
        })
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
            && let Kind::Group { var, source, by } = self.kind(src)
            && let Some(c) = self.fused_group(y, var, source, by, body, q, sc)?
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

    /// Returns the chain reading the rows `y` of `ctors` of `r` for each row reaching it, then `body`.
    fn join(
        &mut self,
        r: RelId,
        ctors: &CtorSet,
        y: Var,
        body: Tm,
        q: &mut QueryCx,
        sc: &Scope,
    ) -> Result<Chain, Error> {
        let outer = sc;
        let (all, rest) = self.pulled(body);
        let n = self.total_prefix(&all);
        let mut pre = Vec::new();
        let mut conds = Vec::new();
        for (i, c) in all.iter().enumerate() {
            if i < n && !self.mentions(*c, &[y]) {
                pre.push(Step::Filter(self.here(*c, sc)?));
            } else {
                conds.push(*c);
            }
        }
        let sc = &sc.clone().times(0.5f64.powi(pre.len() as i32));
        let n = self.total_prefix(&conds);
        let mut options: Vec<(f64, Access)> = Vec::new();
        if (self.rw.index_join || self.bounded)
            && let Some(probe) = self.index_for(r, ctors, y, &conds, rest)
        {
            let per = if probe.unique() {
                PROBE_COST
            } else {
                PROBE_COST + PROBE_ROWS
            };
            options.push((sc.rows * per, Access::Index(probe)));
        }
        if options.is_empty() && ctors.len() > 1 && (self.rw.index_join || self.bounded) {
            let each: Vec<CtorSet> = ctors.iter().map(|&c| CtorSet::from([c])).collect();
            if each
                .iter()
                .all(|one| self.index_for(r, one, y, &conds, rest).is_some())
            {
                let arms = each
                    .iter()
                    .map(|one| self.join(r, one, y, body, q, outer))
                    .collect::<Result<_, _>>()?;
                return Ok(chain(Vec::new(), End::Fork(arms)));
            }
        }
        if self.bounded && options.is_empty() {
            return Err(self.unbounded(r, ctors, y, &conds));
        }
        let mut keys = Vec::new();
        let mut filters = Vec::new();
        for (i, c) in conds.iter().enumerate().take(n) {
            if self.free_in(*c, &sc.local) {
                if self.rw.hash_join
                    && let Kind::Prim(Prim::Eq, args) = self.kind(*c)
                    && let [a, b] = args[..]
                    && erase(&self.ty(a)) == erase(&self.ty(b))
                {
                    for (build, probe) in [(a, b), (b, a)] {
                        if self.mentions(build, &[y])
                            && !self.free_in(build, &sc.local)
                            && !self.mentions(probe, &[y])
                        {
                            keys.push((i, build, probe));
                            break;
                        }
                    }
                }
            } else if self.mentions(*c, &[y]) {
                filters.push(i);
            }
        }
        let build_rows = ctors.len() as f64 * HEAP_ROWS;
        let kept = build_rows * 0.5f64.powi(filters.len() as i32);
        if !keys.is_empty() && !self.bounded {
            options.push((
                build_rows + sc.rows,
                Access::Build {
                    keys: keys.clone(),
                    filters: filters.clone(),
                },
            ));
        }
        if !self.bounded {
            options.push((
                build_rows + sc.rows * kept,
                Access::Build {
                    keys: Vec::new(),
                    filters: filters.clone(),
                },
            ));
        }
        let (_, access) = options
            .into_iter()
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .expect("a cross join is always an option");
        match access {
            Access::Index(probe) => {
                let rows = if probe.unique() { 1.0 } else { PROBE_ROWS };
                let IndexProbe {
                    index,
                    key,
                    range,
                    used,
                } = probe;
                let left: Vec<Tm> = (0..conds.len())
                    .filter(|i| !used.contains(i))
                    .map(|i| conds[i])
                    .collect();
                let inner = sc.clone().times(rows);
                let routes = self.routes(r, ctors, &index.ctors, y, &left, rest, q, &inner)?;
                let key = key
                    .into_iter()
                    .map(|k| self.here(k, sc))
                    .collect::<Result<_, _>>()?;
                let range = range.map(|s| self.span_here(s, sc)).transpose()?;
                let mut ends = self.index_ends(&index, key, range, y, routes);
                Ok(match ends.len() {
                    0 if pre.is_empty() => nothing(),
                    0 => chain(pre, End::Nothing),
                    1 => chain(pre, ends.pop().expect("one end")),
                    _ => chain(
                        pre,
                        End::Fork(ends.into_iter().map(|e| chain(Vec::new(), e)).collect()),
                    ),
                })
            }
            Access::Build { keys, filters } => {
                let key_conds: Vec<usize> = keys.iter().map(|(i, _, _)| *i).collect();
                let build_filters: Vec<Tm> = filters.iter().map(|i| conds[*i]).collect();
                let build_keys: Vec<Tm> = keys.iter().map(|(_, b, _)| *b).collect();
                let probe: Vec<Tm> = keys.iter().map(|(_, _, p)| *p).collect();
                let build = self.build(r, ctors, y, build_keys, build_filters, q)?;
                let left: Vec<Tm> = (0..conds.len())
                    .filter(|i| !filters.contains(i) && !key_conds.contains(i))
                    .map(|i| conds[i])
                    .collect();
                let rows = if keys.is_empty() { kept } else { 1.0 };
                let known = (ctors.len() == 1).then(|| ctors[0]);
                let inner = sc.bind_known(y, known).times(rows);
                let rest = self.filtered(&left, rest, q, &inner)?;
                let probe = probe
                    .into_iter()
                    .map(|k| self.here(k, sc))
                    .collect::<Result<_, _>>()?;
                pre.push(Step::Probe {
                    build,
                    key: probe,
                    var: y,
                });
                Ok(prepend(pre, rest))
            }
        }
    }

    fn build(
        &mut self,
        r: RelId,
        ctors: &CtorSet,
        y: Var,
        key: Vec<Tm>,
        filters: Vec<Tm>,
        q: &mut QueryCx,
    ) -> Result<BuildId, Error> {
        let memo = BuildKey {
            rel: r,
            ctors: ctors.clone(),
            var: y,
            key: key.clone(),
            filters: filters.clone(),
        };
        if let Some(b) = q.memo.get(&memo) {
            return Ok(*b);
        }
        let heaps = if self.rw.prune {
            ctors.clone()
        } else {
            self.all_ctors(r)
        };
        let row = self.var_tm(y);
        let mut pipelines = Vec::new();
        for c in heaps {
            let end = if ctors.contains(&c) {
                End::Emit(row)
            } else {
                End::Nothing
            };
            let steps = if ctors.contains(&c) {
                filters.iter().map(|f| Step::Filter(*f)).collect()
            } else {
                Vec::new()
            };
            if self.rw.prune && matches!(end, End::Nothing) {
                continue;
            }
            pipelines.push(Pipeline {
                source: Source::Scan {
                    heap: self.catalog.heap(r, c),
                    ctor: c,
                    var: y,
                },
                chain: chain(steps, end),
            });
        }
        let id = BuildId(q.builds.len() as u32);
        q.builds.push(Build {
            var: y,
            key,
            pipelines,
        });
        q.memo.insert(memo, id);
        Ok(id)
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

    /// Returns the chain for `for g in group (var in source by by) do body` whose aggregates over `g`'s rows are computed by the group's sink, when there is one to compute.
    #[expect(
        clippy::too_many_arguments,
        reason = "the group's binders and terms would have to travel as one struct"
    )]
    fn fused_group(
        &mut self,
        g: Var,
        var: Var,
        source: Tm,
        by: Tm,
        body: Tm,
        q: &mut QueryCx,
        sc: &Scope,
    ) -> Result<Option<Chain>, Error> {
        let mut found = Found::default();
        if !self.uses(body, g, &mut found, &mut BTreeSet::new()) {
            return Ok(None);
        }
        let inside = self.facts.binders(&self.t, body);
        let strict = self.strict(body);
        let mut fused: Vec<(Tm, bool)> = Vec::new();
        for a in &found.aggs {
            let free = self.facts.free(&self.t, *a);
            if free.iter().any(|v| inside.contains(v)) {
                continue;
            }
            let total = self.total(*a);
            if total || strict.contains(a) {
                fused.push((*a, total));
            }
        }
        if fused.iter().filter(|(_, t)| !t).count() > 1 {
            fused.retain(|(_, t)| *t);
        }
        let Ty::Record(group_fields) = self.t.var_info(g).ty.clone() else {
            return Err(Error::Internal(
                "a group is a record of its key and rows".into(),
            ));
        };
        let field_ty = |name: &str| {
            group_fields
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, t)| t.clone())
                .ok_or_else(|| Error::Internal("a group has a key and rows".into()))
        };
        loop {
            if fused.is_empty() {
                return Ok(None);
            }
            let stop: BTreeSet<Tm> = fused.iter().map(|(a, _)| *a).collect();
            let rows_used = self.reaches_rows(body, g, &stop, &mut BTreeSet::new());
            let names: Vec<String> = (0..fused.len()).map(|i| format!("agg{i}")).collect();
            let mut fields: Vec<(String, Ty)> = fused
                .iter()
                .zip(&names)
                .map(|((a, _), n)| (n.clone(), self.ty(*a)))
                .collect();
            fields.push(("key".into(), field_ty("key")?));
            if rows_used {
                fields.push(("rows".into(), field_ty("rows")?));
            }
            fields.sort_by(|a, b| a.0.cmp(&b.0));
            let rec_ty = Ty::Record(fields);
            let g2 = self.fresh("%group", rec_ty.clone());
            let g2_tm = self.var_tm(g2);
            let mut map: HashMap<Tm, Tm> = HashMap::new();
            for ((a, _), n) in fused.iter().zip(&names) {
                let f = self.add(Kind::Field(g2_tm, n.clone()), self.ty(*a));
                map.insert(*a, f);
            }
            for (f, name) in [(&found.keys, "key"), (&found.rows, "rows")] {
                for t in f {
                    let n = self.add(Kind::Field(g2_tm, name.into()), self.ty(*t));
                    map.insert(*t, n);
                }
            }
            let new_body = self.replace(body, &map, &mut HashMap::new());
            if fused.iter().any(|(_, t)| !t) && !self.total(new_body) {
                fused.retain(|(_, t)| *t);
                continue;
            }
            let mut aggs = Vec::new();
            for ((a, _), n) in fused.iter().zip(&names) {
                aggs.push(self.group_agg(*a, n.clone())?);
            }
            let sink = Sink::Group {
                var,
                by: self.eval(by)?,
                rows: rows_used,
                aggs,
            };
            let groups = self.query(source, sink, None)?;
            let v = self.fresh("%groups", Ty::Set(Box::new(rec_ty)));
            let v_tm = self.var_tm(v);
            let rest = self.chain(new_body, q, &sc.bind(g2).times(SET_ROWS))?;
            return Ok(Some(prepend(
                vec![Step::Bind(v, groups), Step::Unnest(g2, v_tm)],
                rest,
            )));
        }
    }

    /// Returns whether `t` uses the rows of group `g` outside the terms of `stop`.
    fn reaches_rows(
        &mut self,
        t: Tm,
        g: Var,
        stop: &BTreeSet<Tm>,
        seen: &mut BTreeSet<Tm>,
    ) -> bool {
        if stop.contains(&t) || !seen.insert(t) {
            return false;
        }
        if let Kind::Field(x, l) = self.kind(t)
            && l == "rows"
            && matches!(self.t.kind(x), Kind::Var(v) if *v == g)
        {
            return true;
        }
        self.t
            .children(t)
            .into_iter()
            .any(|c| self.reaches_rows(c, g, stop, seen))
    }

    fn group_agg(&mut self, a: Tm, field: String) -> Result<GroupAgg, Error> {
        let Kind::Agg {
            op,
            var,
            source,
            body,
        } = self.kind(a)
        else {
            panic!("a fused aggregate is an aggregate")
        };
        let Kind::For(row, _, inner) = self.kind(source) else {
            panic!("a fused aggregate ranges over the group's rows")
        };
        let (conds, elem) = self.peel(inner);
        let Kind::Singleton(element) = self.kind(elem) else {
            panic!("a fused aggregate's range selects one element per row")
        };
        let mut filter = Vec::new();
        for c in conds {
            filter.push(self.eval(c)?);
        }
        Ok(GroupAgg {
            field,
            op,
            ty: self.ty(a),
            body_ty: self.ty(body),
            row,
            filter,
            elem: var,
            element: self.eval(element)?,
            body: self.eval(body)?,
        })
    }

    /// Records the uses of group variable `g` in `t`; returns false when one is neither its key, its rows nor an aggregate over its rows.
    fn uses(&mut self, t: Tm, g: Var, found: &mut Found, seen: &mut BTreeSet<Tm>) -> bool {
        if !seen.insert(t) {
            return true;
        }
        let is_g = |p: &Self, x: Tm| matches!(p.t.kind(x), Kind::Var(v) if *v == g);
        match self.kind(t) {
            Kind::Var(v) => v != g,
            Kind::Field(x, l) if is_g(self, x) => {
                match l.as_str() {
                    "key" => found.keys.push(t),
                    _ => found.rows.push(t),
                }
                true
            }
            Kind::Agg {
                op, source, body, ..
            } if !matches!(op, Agg::User(_))
                && self.rows_agg(source, g)
                && !self.mentions(body, &[g]) =>
            {
                found.aggs.push(t);
                if let Kind::For(_, rows, _) = self.kind(source) {
                    found.rows.push(rows);
                }
                true
            }
            _ => self
                .t
                .children(t)
                .into_iter()
                .all(|c| self.uses(c, g, found, seen)),
        }
    }

    /// Returns whether `source` selects one distinct element per row of group `g` that passes its conditions.
    fn rows_agg(&mut self, source: Tm, g: Var) -> bool {
        let Kind::For(row, rows, inner) = self.kind(source) else {
            return false;
        };
        let Kind::Field(x, l) = self.kind(rows) else {
            return false;
        };
        if l != "rows"
            || !matches!(self.t.kind(x), Kind::Var(v) if *v == g)
            || self.mentions(inner, &[g])
        {
            return false;
        }
        let (_, elem) = self.peel(inner);
        let Kind::Singleton(e) = self.kind(elem) else {
            return false;
        };
        let is_row = |p: &Self, t: Tm| matches!(p.t.kind(t), Kind::Var(v) if *v == row);
        match self.kind(e) {
            Kind::Var(v) => v == row,
            Kind::Record(fs) | Kind::Ctor { fields: fs, .. } => {
                fs.iter().any(|(_, f)| is_row(self, *f))
            }
            _ => false,
        }
    }

    /// Returns the subterms of `t` evaluated whenever `t` is.
    fn strict(&self, t: Tm) -> BTreeSet<Tm> {
        let mut out = BTreeSet::new();
        let mut stack = vec![t];
        while let Some(t) = stack.pop() {
            if !out.insert(t) {
                continue;
            }
            match self.t.kind(t) {
                Kind::If(c, ..) => stack.push(*c),
                Kind::Case { scrutinee, .. } => stack.push(*scrutinee),
                Kind::For(_, s, _)
                | Kind::Agg { source: s, .. }
                | Kind::Group { source: s, .. } => stack.push(*s),
                Kind::Fix(..) | Kind::Top { .. } | Kind::Present { .. } | Kind::Try(_) => {}
                _ => stack.extend(self.t.children(t)),
            }
        }
        out
    }

    fn replace(&mut self, t: Tm, map: &HashMap<Tm, Tm>, memo: &mut HashMap<Tm, Tm>) -> Tm {
        if let Some(n) = map.get(&t) {
            memo.insert(t, *n);
            return *n;
        }
        if let Some(n) = memo.get(&t) {
            return *n;
        }
        let cs = self.t.children(t);
        let new: Vec<Tm> = cs.iter().map(|c| self.replace(*c, map, memo)).collect();
        let out = if new == cs {
            t
        } else {
            let kind = with_children(self.t.kind(t), &new);
            self.add(kind, self.ty(t))
        };
        memo.insert(t, out);
        out
    }
}

#[derive(Default)]
struct Found {
    keys: Vec<Tm>,
    rows: Vec<Tm>,
    aggs: Vec<Tm>,
}

fn collect_eval(c: &Catalog, e: &Eval, out: &mut Reads) {
    match e {
        Eval::Expr(_) | Eval::GroupRows { .. } => {}
        Eval::Let(_, a, b) | Eval::If(_, a, b) => {
            collect_eval(c, a, out);
            collect_eval(c, b, out);
        }
        Eval::Case(_, arms) => arms.iter().for_each(|(_, _, a)| collect_eval(c, a, out)),
        Eval::Try(a) => collect_eval(c, a, out),
        Eval::Lookup(l) => {
            out.indexes.insert(l.index);
            out.heaps.extend(l.routes.iter().map(|(_, h)| *h));
        }
        Eval::Query(q) => {
            for b in &q.builds {
                b.pipelines.iter().for_each(|p| collect_pipeline(c, p, out));
            }
            collect_spine(c, &q.spine, out);
            match &q.sink {
                Sink::Set | Sink::Exists | Sink::IsEmpty => {}
                Sink::Agg { body, .. } | Sink::Fold { body, .. } => collect_eval(c, body, out),
                Sink::Group { by, aggs, .. } => {
                    collect_eval(c, by, out);
                    for a in aggs {
                        a.filter.iter().for_each(|f| collect_eval(c, f, out));
                        collect_eval(c, &a.element, out);
                        collect_eval(c, &a.body, out);
                    }
                }
                Sink::Top { count, keys, .. } => {
                    collect_eval(c, count, out);
                    keys.iter().for_each(|(k, _)| collect_eval(c, k, out));
                }
                Sink::Present {
                    keys, take, after, ..
                } => {
                    keys.iter().for_each(|(k, _)| collect_eval(c, k, out));
                    take.iter()
                        .chain(after)
                        .for_each(|x| collect_eval(c, x, out));
                }
            }
        }
        Eval::Fix(f) => {
            for b in &f.binds {
                collect_eval(c, &b.body, out);
                if let Some(s) = &b.step {
                    collect_eval(c, s, out);
                }
            }
            collect_eval(c, &f.body, out);
        }
    }
}

fn collect_spine(c: &Catalog, s: &Spine, out: &mut Reads) {
    match s {
        Spine::Pipeline(p) => collect_pipeline(c, p, out),
        Spine::Union(ss) => ss.iter().for_each(|s| collect_spine(c, s, out)),
        Spine::If(_, a, b) => {
            collect_spine(c, a, out);
            collect_spine(c, b, out);
        }
        Spine::Case(_, arms) => arms.iter().for_each(|(_, _, s)| collect_spine(c, s, out)),
        Spine::Let(_, e, b) => {
            collect_eval(c, e, out);
            collect_spine(c, b, out);
        }
    }
}

fn collect_pipeline(c: &Catalog, p: &Pipeline, out: &mut Reads) {
    if let Source::Scan { heap, .. } = p.source {
        out.heaps.insert(heap);
    }
    collect_chain(c, &p.chain, out);
}

fn collect_chain(c: &Catalog, ch: &Chain, out: &mut Reads) {
    for s in &ch.steps {
        if let Step::Bind(_, e) = s {
            collect_eval(c, e, out);
        }
    }
    match &ch.end {
        End::Nothing | End::Emit(_) => {}
        End::Fork(cs) => cs.iter().for_each(|x| collect_chain(c, x, out)),
        End::If(_, a, b) => {
            collect_chain(c, a, out);
            collect_chain(c, b, out);
        }
        End::Case(_, arms) => arms.iter().for_each(|(_, _, x)| collect_chain(c, x, out)),
        End::Index { index, routes, .. } | End::Range { index, routes, .. } => {
            out.indexes.insert(*index);
            for r in routes {
                out.heaps.insert(r.heap);
                collect_chain(c, &r.chain, out);
            }
        }
    }
}
