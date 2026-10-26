use std::collections::{BTreeMap, BTreeSet, HashMap};

use adtb_lang::analysis::Totality;
use adtb_lang::kernel::{Agg, Kind, Prim, Terms, Tm, Var};
use adtb_lang::schema::Schema;
use adtb_lang::types::{CtorSet, RelId, Ty};

use super::catalog::{Catalog, IndexDesc};
use super::terms::{Facts, with_children};
use super::*;

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
/// - `catalog` describes every relation variable `root` reads.
pub fn plan(
    schema: &Schema,
    catalog: &Catalog,
    terms: &Terms,
    root: Tm,
    params: &[Var],
    options: Options,
) -> Result<Plan, Error> {
    let mut p = Planner {
        schema,
        catalog,
        rw: options.rewrites,
        t: terms.clone(),
        total: Totality::new(schema, options.hiding),
        facts: Facts::default(),
        vars: HashMap::new(),
    };
    let root = p.eval(root)?;
    let mut reads = Reads::default();
    collect_eval(catalog, &root, &mut reads);
    Ok(Plan {
        terms: p.t,
        params: params.to_vec(),
        root,
        reads,
    })
}

struct Planner<'a> {
    schema: &'a Schema,
    catalog: &'a Catalog,
    rw: Rewrites,
    t: Terms,
    total: Totality<'a>,
    facts: Facts,
    vars: HashMap<Var, Tm>,
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
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct BuildKey {
    rel: RelId,
    ctors: CtorSet,
    var: Var,
    key: Vec<Tm>,
    filters: Vec<Tm>,
}

/// An index probe: the index, the values of its leading fields, and the conditions they replace.
struct IndexProbe {
    index: IndexDesc,
    key: Vec<Tm>,
    used: Vec<usize>,
}

impl IndexProbe {
    fn unique(&self) -> bool {
        self.index.unique && self.key.len() == self.index.fields.len()
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
    fn here(&mut self, t: Tm, sc: &Scope) -> Tm {
        if !self.rw.static_route || sc.known.is_empty() {
            return t;
        }
        self.specialise(t, &sc.known, &mut HashMap::new())
    }

    fn specialise(&mut self, t: Tm, known: &BTreeMap<Var, u32>, memo: &mut HashMap<Tm, Tm>) -> Tm {
        if let Some(n) = memo.get(&t) {
            return *n;
        }
        let out = match self.kind(t) {
            Kind::Case { scrutinee, arms }
                if let Kind::Var(v) = self.kind(scrutinee)
                    && let Some(c) = known.get(&v) =>
            {
                let (_, x, arm) = arms
                    .into_iter()
                    .find(|(d, _, _)| d == c)
                    .expect("a case names the constructor its scrutinee holds");
                let mut inner = known.clone();
                inner.insert(x, *c);
                let arm = self.specialise(arm, &inner, &mut HashMap::new());
                self.add(Kind::Let(x, scrutinee, arm), self.ty(t))
            }
            kind => {
                let cs = self.t.children(t);
                let new: Vec<Tm> = cs
                    .iter()
                    .map(|c| self.specialise(*c, known, memo))
                    .collect();
                if new == cs {
                    t
                } else {
                    self.add(with_children(&kind, &new), self.ty(t))
                }
            }
        };
        memo.insert(t, out);
        out
    }

    /// Returns `t` as a scalar expression at a point of a chain, with the bindings that compute what it reads.
    fn lift_here(&mut self, t: Tm, sc: &Scope) -> Result<(Vec<(Var, Eval)>, Tm), Error> {
        let (lets, t) = self.lift_eval(t)?;
        Ok((lets, self.here(t, sc)))
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
                let lookup = self.lookup(k, &ty);
                Ok(wrap_eval(lets, Eval::Lookup(lookup)))
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
                self.query(source, sink)
            }
            Kind::Exists(s) => self.query(s, Sink::Exists),
            Kind::IsEmpty(s) => self.query(s, Sink::IsEmpty),
            Kind::Group { var, source, by } => {
                let sink = Sink::Group {
                    var,
                    by: self.eval(by)?,
                    rows: true,
                    aggs: Vec::new(),
                };
                self.query(source, sink)
            }
            Kind::Top {
                count,
                keys,
                var,
                source,
            } => {
                let count = self.eval(count)?;
                let keys = self.keys(&keys)?;
                self.query(source, Sink::Top { count, keys, var })
            }
            Kind::Present {
                var,
                source,
                keys,
                take,
                after,
            } => {
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
                )
            }
            _ if matches!(ty, Ty::Set(_) | Ty::NonEmpty(_)) => self.query(t, Sink::Set),
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

    /// Refuses a term whose root form a plan does not compute.
    fn refuse(&self, t: Tm) -> Result<(), Error> {
        match self.t.kind(t) {
            Kind::Call { func, .. } => {
                Err(Error::UserCode(self.schema.function(*func).name.clone()))
            }
            Kind::Agg {
                op: Agg::User(a), ..
            } => Err(Error::UserCode(
                self.schema.aggregates[*a as usize].name.clone(),
            )),
            Kind::At { .. } => Err(Error::NotAQuery("a read of history")),
            Kind::Transition(..) => Err(Error::NotAQuery("a read of a statement's transition")),
            Kind::Lam(..) | Kind::App(..) => panic!("a normal form holds no function value"),
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

    fn query(&mut self, source: Tm, sink: Sink) -> Result<Eval, Error> {
        let mut q = QueryCx::default();
        let spine = self.spine(source, &mut q, &Scope::new())?;
        Ok(Eval::Query(Box::new(Query {
            builds: q.builds,
            spine,
            sink,
        })))
    }

    fn lookup(&mut self, key: Tm, row_ty: &Ty) -> Lookup {
        let Ty::Key { rel, ctors } = self.ty(key) else {
            panic!("a dereference follows a key")
        };
        let ctors = match row_ty {
            Ty::Data { ctors: Some(s), .. } => s.clone(),
            _ => self.ctors_of(rel, &ctors),
        };
        let desc = self.catalog.relation(rel);
        Lookup {
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
        }
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

    /// Returns the fields of a row `x` of `rel` that condition `c` fixes, each with its value, when `c` is an equality whose other side does not mention `x`.
    fn equality(&mut self, rel: RelId, c: Tm, x: Var) -> Option<Vec<(String, Tm)>> {
        let Kind::Prim(Prim::Eq, args) = self.kind(c) else {
            return None;
        };
        let [a, b] = args[..] else { return None };
        if erase(&self.ty(a)) != erase(&self.ty(b)) {
            return None;
        }
        for (this, other) in [(a, b), (b, a)] {
            if self.mentions(other, &[x]) {
                continue;
            }
            match self.kind(this) {
                Kind::Field(row, f) if matches!(self.t.kind(row), Kind::Var(v) if *v == x) => {
                    return Some(vec![(f, other)]);
                }
                Kind::Key(row) if matches!(self.t.kind(row), Kind::Var(v) if *v == x) => {
                    return Some(self.key_values(rel, other));
                }
                _ => {}
            }
        }
        None
    }

    /// Returns the best index of `rel` covering `ctors` whose fields the total prefix of `conds` fixes for row `x`.
    fn index_for(
        &mut self,
        rel: RelId,
        ctors: &CtorSet,
        x: Var,
        conds: &[Tm],
    ) -> Option<IndexProbe> {
        let n = self.total_prefix(conds);
        let mut fixed: Vec<(usize, Vec<(String, Tm)>)> = Vec::new();
        for (i, c) in conds.iter().enumerate().take(n) {
            if let Some(pairs) = self.equality(rel, *c, x) {
                fixed.push((i, pairs));
            }
        }
        if fixed.is_empty() {
            return None;
        }
        let mut best: Option<((bool, usize), IndexProbe)> = None;
        for index in &self.catalog.relation(rel).indexes {
            if !ctors.iter().all(|c| index.ctors.contains(c)) {
                continue;
            }
            let mut key = Vec::new();
            for f in &index.fields {
                match fixed.iter().flat_map(|(_, ps)| ps).find(|(g, _)| g == f) {
                    Some((_, v)) => key.push(*v),
                    None => break,
                }
            }
            let full = key.len() == index.fields.len();
            if key.is_empty() || (!full && !index.ordered) {
                continue;
            }
            let covered: Vec<&String> = index.fields.iter().take(key.len()).collect();
            let used = fixed
                .iter()
                .filter(|(_, ps)| ps.iter().all(|(g, _)| covered.contains(&g)))
                .map(|(i, _)| *i)
                .collect();
            let rank = (full && index.unique, key.len());
            if best.as_ref().is_none_or(|b| rank > b.0) {
                let probe = IndexProbe {
                    index: index.clone(),
                    key,
                    used,
                };
                best = Some((rank, probe));
            }
        }
        best.map(|(_, p)| p)
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
        if self.rw.index_scan
            && let Some(probe) = self.index_for(r, ctors, x, &conds)
        {
            let rows = if probe.unique() { 1.0 } else { PROBE_ROWS };
            let IndexProbe { index, key, used } = probe;
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
            let ends = self.index_ends(&index, key, x, routes);
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
    #[allow(clippy::too_many_arguments)]
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
        x: Var,
        routes: Vec<Route>,
    ) -> Vec<End> {
        let end = |routes| End::Index {
            index: index.id,
            key: key.clone(),
            var: x,
            routes,
        };
        if routes.is_empty() {
            Vec::new()
        } else if self.rw.fuse_probe {
            vec![end(routes)]
        } else {
            routes.into_iter().map(|r| end(vec![r])).collect()
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
                let (_, x, arm) = arms
                    .into_iter()
                    .find(|(d, _, _)| *d == k)
                    .expect("a case names the constructor its scrutinee holds");
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
            let (lets, c) = self.lift_here(c, &sc)?;
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
                let (lets, e) = self.lift_here(e, sc)?;
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
                let (lets, c) = self.lift_here(c, sc)?;
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
                    let (_, x, body) = arms
                        .iter()
                        .find(|(d, _, _)| *d == c)
                        .cloned()
                        .expect("a case names the constructor its scrutinee holds");
                    let rest = self.chain(body, q, &sc.bind_known(x, Some(c)))?;
                    return Ok(prepend(vec![Step::Bind(x, Eval::Expr(scrutinee))], rest));
                }
                let (lets, s) = self.lift_here(scrutinee, sc)?;
                let mut out = Vec::new();
                for (c, x, body) in arms {
                    out.push((c, x, self.chain(body, q, &sc.bind_known(x, Some(c)))?));
                }
                Ok(chain(binds(lets), End::Case(s, out)))
            }
            Kind::Let(x, e, b) => {
                let e = self.eval(e)?;
                let rest = self.chain(b, q, &sc.bind(x))?;
                Ok(prepend(vec![Step::Bind(x, e)], rest))
            }
            Kind::For(y, src, body) => self.for_chain(y, src, body, q, sc),
            _ => {
                let y = self.fresh("%e", self.elem_ty(t));
                let body = self.var_tm(y);
                let body = self.add(Kind::Singleton(body), self.ty(t));
                self.for_chain(y, t, body, q, sc)
            }
        }
    }

    fn elem_ty(&self, set: Tm) -> Ty {
        match self.t.ty(set) {
            Ty::Set(e) | Ty::NonEmpty(e) => (**e).clone(),
            other => panic!("a comprehension ranges over a set, not {other:?}"),
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
            return self.join(r, &ctors, y, body, q, sc);
        }
        if self.rw.group_agg
            && let Kind::Group { var, source, by } = self.kind(src)
            && let Some(c) = self.fused_group(y, var, source, by, body, q, sc)?
        {
            return Ok(c);
        }
        let (lets, s) = self.lift_here(src, sc)?;
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
        let (all, rest) = self.pulled(body);
        let n = self.total_prefix(&all);
        let mut pre = Vec::new();
        let mut conds = Vec::new();
        for (i, c) in all.iter().enumerate() {
            if i < n && !self.mentions(*c, &[y]) {
                pre.push(Step::Filter(self.here(*c, sc)));
            } else {
                conds.push(*c);
            }
        }
        let sc = &sc.clone().times(0.5f64.powi(pre.len() as i32));
        let n = self.total_prefix(&conds);
        let mut options: Vec<(f64, Access)> = Vec::new();
        if self.rw.index_join
            && let Some(probe) = self.index_for(r, ctors, y, &conds)
        {
            let per = if probe.unique() {
                PROBE_COST
            } else {
                PROBE_COST + PROBE_ROWS
            };
            options.push((sc.rows * per, Access::Index(probe)));
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
        if !keys.is_empty() {
            options.push((
                build_rows + sc.rows,
                Access::Build {
                    keys: keys.clone(),
                    filters: filters.clone(),
                },
            ));
        }
        options.push((
            build_rows + sc.rows * kept,
            Access::Build {
                keys: Vec::new(),
                filters: filters.clone(),
            },
        ));
        let (_, access) = options
            .into_iter()
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .expect("a cross join is always an option");
        match access {
            Access::Index(probe) => {
                let rows = if probe.unique() { 1.0 } else { PROBE_ROWS };
                let IndexProbe { index, key, used } = probe;
                let left: Vec<Tm> = (0..conds.len())
                    .filter(|i| !used.contains(i))
                    .map(|i| conds[i])
                    .collect();
                let inner = sc.clone().times(rows);
                let routes = self.routes(r, ctors, &index.ctors, y, &left, rest, q, &inner)?;
                let key = key.into_iter().map(|k| self.here(k, sc)).collect();
                let mut ends = self.index_ends(&index, key, y, routes);
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
                let probe = probe.into_iter().map(|k| self.here(k, sc)).collect();
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
    #[allow(clippy::too_many_arguments)]
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
            panic!("a group is a record of its key and rows")
        };
        let field_ty = |name: &str| {
            group_fields
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, t)| t.clone())
                .expect("a group has a key and rows")
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
            fields.push(("key".into(), field_ty("key")));
            if rows_used {
                fields.push(("rows".into(), field_ty("rows")));
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
            let groups = self.query(source, sink)?;
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
        Eval::Expr(_) => {}
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
                Sink::Agg { body, .. } => collect_eval(c, body, out),
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
        End::Index { index, routes, .. } => {
            out.indexes.insert(*index);
            for r in routes {
                out.heaps.insert(r.heap);
                collect_chain(c, &r.chain, out);
            }
        }
    }
}
