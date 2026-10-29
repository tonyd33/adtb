use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use crate::analysis::Totality;
use crate::check::{Adhoc, AdhocCommand, Program, Query, Txn};
use crate::kernel::{Agg, IndexField, Kind, Node, Terms, Tm, Var};
use crate::schema::Schema;
use crate::types::*;

/// The normal forms of a schema's bodies, in one arena.
#[derive(Debug)]
pub struct NormalProgram {
    pub terms: Terms,
    /// Each function's parameters and normalised body; `None` for Wasm and host functions and for a function that takes or returns a function.
    pub functions: Vec<Option<(Vec<Var>, Tm)>>,
    pub constraints: Vec<Tm>,
    pub checks: Vec<Tm>,
    pub domains: BTreeMap<TypeId, (Var, Tm)>,
    /// Each `on delete retag` new value over the referrer's row.
    pub retags: BTreeMap<RelId, (Var, Tm)>,
    pub defaults: BTreeMap<(RelId, usize), Tm>,
    pub computed: BTreeMap<(RelId, usize), (Var, Tm)>,
    /// Each index field, its value a body over the row's variable.
    pub index_fields: BTreeMap<(RelId, usize, usize), (Var, IndexField)>,
    /// Each user-defined aggregate's parts; `None` for one that did not check or whose parts are not first-order.
    pub aggregates: Vec<Option<NormalAggregate>>,
}

/// The normal forms of a user-defined aggregate's parts, each over its variables.
#[derive(Debug)]
pub struct NormalAggregate {
    pub init: Tm,
    pub step: (Vec<Var>, Tm),
    pub merge: (Vec<Var>, Tm),
    pub finish: (Vec<Var>, Tm),
}

/// The normal forms of the items of ad hoc text.
#[derive(Debug)]
pub struct NormalAdhoc {
    pub terms: Terms,
    pub items: Vec<Tm>,
    /// Each command, its terms normalised.
    pub commands: Vec<AdhocCommand>,
    pub params: BTreeMap<String, Var>,
}

/// The normal form of a registered query's or transaction's body over its parameters.
#[derive(Debug)]
pub struct NormalQuery {
    pub terms: Terms,
    pub params: Vec<Var>,
    pub body: Tm,
}

/// A body whose normalisation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Root {
    Function(FnId),
    Constraint(usize),
    Check(usize),
    Domain(TypeId),
    Retag(RelId),
    Default(RelId, usize),
    Computed(RelId, usize),
    IndexField(RelId, usize, usize),
    Aggregate(usize),
    Item(usize),
    /// A term of the command at this position among the text's commands.
    Command(usize),
    Query,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub root: Root,
    pub message: String,
}

fn at<T>(root: Root, r: Result<T, String>) -> Result<T, Error> {
    r.map_err(|message| Error { root, message })
}

/// Normalises every body of a checked schema.
pub fn program(schema: &Schema, p: &Program) -> Result<NormalProgram, Error> {
    let mut n = Normaliser::new(schema, [&p.terms, &p.terms], &p.functions);
    let mut functions = Vec::with_capacity(p.functions.len());
    for (i, (f, def)) in p.functions.iter().zip(&schema.functions).enumerate() {
        functions.push(match f {
            Some(t) => at(
                Root::Function(FnId(i as u32)),
                n.function(FNS, *t, def.params.len().max(1)),
            )?,
            None => None,
        });
    }
    let mut constraints = Vec::with_capacity(p.constraints.len());
    for (i, t) in p.constraints.iter().enumerate() {
        constraints.push(at(Root::Constraint(i), n.root(*t))?);
    }
    let mut checks = Vec::with_capacity(p.checks.len());
    for (i, t) in p.checks.iter().enumerate() {
        checks.push(at(Root::Check(i), n.root(*t))?);
    }
    let mut domains = BTreeMap::new();
    for (id, (v, t)) in &p.domains {
        let x = n.free_var(MAIN, *v);
        domains.insert(*id, (x, at(Root::Domain(*id), n.root(*t))?));
    }
    let mut retags = BTreeMap::new();
    for (r, t) in &p.retags {
        retags.insert(*r, at(Root::Retag(*r), n.row_function(*t))?);
    }
    let mut defaults = BTreeMap::new();
    for (&(r, i), t) in &p.defaults {
        defaults.insert((r, i), at(Root::Default(r, i), n.root(*t))?);
    }
    let mut computed = BTreeMap::new();
    for (&(r, i), (v, t)) in &p.computed {
        let x = n.free_var(MAIN, *v);
        computed.insert((r, i), (x, at(Root::Computed(r, i), n.root(*t))?));
    }
    let mut index_fields = BTreeMap::new();
    for (&(r, i, j), f) in &p.index_fields {
        let (x, value) = at(Root::IndexField(r, i, j), n.row_function(f.value))?;
        let path = f.path.clone();
        index_fields.insert((r, i, j), (x, IndexField { value, path }));
    }
    let mut aggregates = Vec::with_capacity(p.aggregates.len());
    for (i, a) in p.aggregates.iter().enumerate() {
        let Some(a) = a else {
            aggregates.push(None);
            continue;
        };
        let root = Root::Aggregate(i);
        let init = at(root, n.root(a.init))?;
        let step = at(root, n.function(MAIN, a.step, 2))?;
        let merge = at(root, n.function(MAIN, a.merge, 2))?;
        let finish = at(root, n.function(MAIN, a.finish, 1))?;
        aggregates.push(match (step, merge, finish) {
            (Some(step), Some(merge), Some(finish)) => Some(NormalAggregate {
                init,
                step,
                merge,
                finish,
            }),
            _ => None,
        });
    }
    Ok(NormalProgram {
        terms: n.out,
        functions,
        constraints,
        checks,
        domains,
        retags,
        defaults,
        computed,
        index_fields,
        aggregates,
    })
}

/// Normalises checked ad hoc text, inlining the functions of `p`.
pub fn adhoc(schema: &Schema, p: &Program, a: &Adhoc) -> Result<NormalAdhoc, Error> {
    let mut n = Normaliser::new(schema, [&a.terms, &p.terms], &p.functions);
    let params = a
        .params
        .iter()
        .map(|(name, v)| (name.clone(), n.free_var(MAIN, *v)))
        .collect();
    let mut items = Vec::with_capacity(a.items.len());
    for (i, t) in a.items.iter().enumerate() {
        items.push(at(Root::Item(i), n.root(*t))?);
    }
    let mut commands = Vec::with_capacity(a.commands.len());
    for (i, c) in a.commands.iter().enumerate() {
        let terms = c
            .command
            .terms()
            .into_iter()
            .map(|t| at(Root::Command(i), n.root(t)))
            .collect::<Result<Vec<Tm>, Error>>()?;
        commands.push(AdhocCommand {
            command: c.command.with_terms(&terms),
            ..c.clone()
        });
    }
    Ok(NormalAdhoc {
        terms: n.out,
        items,
        commands,
        params,
    })
}

/// Normalises a checked registered query, inlining the functions of `p`.
pub fn query(schema: &Schema, p: &Program, q: &Query) -> Result<NormalQuery, Error> {
    registered(schema, p, &q.terms, q.term)
}

/// Normalises a checked registered transaction, inlining the functions of `p`.
pub fn txn(schema: &Schema, p: &Program, t: &Txn) -> Result<NormalQuery, Error> {
    registered(schema, p, &t.terms, t.term)
}

fn registered(schema: &Schema, p: &Program, terms: &Terms, term: Tm) -> Result<NormalQuery, Error> {
    let mut n = Normaliser::new(schema, [terms, &p.terms], &p.functions);
    let (params, body) = n.under_lams(MAIN, term, usize::MAX);
    let params: Vec<Var> = params.into_iter().map(|v| n.free_var(MAIN, v)).collect();
    let body = at(Root::Query, n.root(body))?;
    Ok(NormalQuery {
        terms: n.out,
        params,
        body,
    })
}

const MAIN: usize = 0;
const FNS: usize = 1;

/// A value during normalisation: a first-order term of the output, or a function.
#[derive(Clone, Debug)]
enum Val {
    Tm(Tm),
    Lam { var: Var, body: Tm, cx: Cx },
    If(Tm, Rc<Val>, Rc<Val>),
    Case(Tm, Rc<Vec<(u32, Var, Val)>>),
    Let(Var, Tm, Rc<Val>),
    Fix(Vec<(Var, Tm)>, Rc<Val>),
}

/// The values of the variables of one arena's terms.
#[derive(Clone, Debug, Default)]
struct Env(Option<Rc<Frame>>);

#[derive(Debug)]
struct Frame {
    var: Var,
    val: Val,
    next: Env,
}

impl Drop for Env {
    fn drop(&mut self) {
        let mut next = self.0.take();
        while let Some(frame) = next {
            next = match Rc::try_unwrap(frame) {
                Ok(mut f) => f.next.0.take(),
                Err(_) => None,
            };
        }
    }
}

impl Env {
    fn get(&self, var: Var) -> Option<&Val> {
        let mut e = self;
        while let Some(f) = &e.0 {
            if f.var == var {
                return Some(&f.val);
            }
            e = &f.next;
        }
        None
    }
}

/// Where a source term is normalised: its arena, the values of its variables, and the types of its type parameters.
#[derive(Clone, Debug)]
struct Cx {
    arena: usize,
    env: Env,
    targs: Rc<Vec<Ty>>,
}

impl Cx {
    fn new(arena: usize, targs: Vec<Ty>) -> Cx {
        Cx {
            arena,
            env: Env::default(),
            targs: Rc::new(targs),
        }
    }

    fn with(&self, var: Var, val: Val) -> Cx {
        Cx {
            arena: self.arena,
            env: Env(Some(Rc::new(Frame {
                var,
                val,
                next: self.env.clone(),
            }))),
            targs: self.targs.clone(),
        }
    }
}

/// What to do with a function once its value is known.
#[derive(Clone)]
enum Then {
    /// Apply it to this argument; the result has type `ty`.
    Apply(Val, Ty),
    /// Bind it to `var` in `body`; the result has type `ty`.
    Bind { var: Var, body: Tm, cx: Cx, ty: Ty },
}

/// A binder, a write or a condition of a [`Normaliser::spine`] already normalised, waiting for
/// its body.
enum Wrap {
    Block(Tm, Ty),
    LetBang(Var, Tm, Ty),
    Let(Var, Tm, Ty),
    /// An `If` with its condition and then branch; the body is its else branch.
    If(Tm, Val, Ty),
    /// The tail whose normal form the body is, memoised once known.
    Memo(MemoKey),
}

type MemoKey = (usize, Tm, Vec<Ty>, Vec<Option<Tm>>);

struct Normaliser<'a> {
    schema: &'a Schema,
    src: [&'a Terms; 2],
    fns: &'a [Option<Tm>],
    out: Terms,
    tot: Totality<'a>,
    draws: HashMap<Tm, bool>,
    free: HashMap<(usize, Var), Var>,
    fv: [HashMap<Tm, Rc<Vec<Var>>>; 2],
    memo: HashMap<MemoKey, Tm>,
}

impl<'a> Normaliser<'a> {
    fn new(schema: &'a Schema, src: [&'a Terms; 2], fns: &'a [Option<Tm>]) -> Self {
        Normaliser {
            schema,
            src,
            fns,
            out: Terms::default(),
            tot: Totality::new(schema, false),
            draws: HashMap::new(),
            free: HashMap::new(),
            fv: [HashMap::new(), HashMap::new()],
            memo: HashMap::new(),
        }
    }

    fn node(&self, arena: usize, t: Tm) -> &'a Node {
        let terms: &'a Terms = self.src[arena];
        terms.node(t)
    }

    /// Returns whether `t` may be duplicated or dropped: it is total, draws no fresh value and
    /// calls no volatile host function.
    fn copyable(&mut self, t: Tm) -> bool {
        self.tot.total(&self.out, t) && !self.draws(t)
    }

    fn draws(&mut self, t: Tm) -> bool {
        if let Some(&d) = self.draws.get(&t) {
            return d;
        }
        let d = match self.out.kind(t) {
            Kind::Fresh(..) => true,
            Kind::Call { func, .. }
                if matches!(
                    self.schema.function(*func).body,
                    crate::ast::FnBody::Host {
                        deterministic: false,
                        ..
                    }
                ) =>
            {
                true
            }
            _ => self.out.children(t).into_iter().any(|c| self.draws(c)),
        };
        self.draws.insert(t, d);
        d
    }

    fn add(&mut self, kind: Kind, ty: Ty) -> Tm {
        self.out.add(kind, ty)
    }

    /// Returns a new output variable for binder `v` of `cx`'s arena.
    fn fresh(&mut self, v: Var, cx: &Cx) -> Var {
        let info = self.src[cx.arena].var_info(v);
        self.out.var(&info.name, info.ty.subst(&cx.targs))
    }

    /// Returns the output variable standing for free variable `v` of `arena`.
    fn free_var(&mut self, arena: usize, v: Var) -> Var {
        if let Some(x) = self.free.get(&(arena, v)) {
            return *x;
        }
        let info = self.src[arena].var_info(v);
        let x = self.out.var(&info.name, info.ty.clone());
        self.free.insert((arena, v), x);
        x
    }

    fn root(&mut self, t: Tm) -> Result<Tm, String> {
        let v = self.norm(t, &Cx::new(MAIN, Vec::new()))?;
        self.first_order(v)
    }

    /// Splits `t` into the variables of at most `n` leading `Lam`s and the body under them.
    fn under_lams(&self, arena: usize, mut t: Tm, n: usize) -> (Vec<Var>, Tm) {
        let mut vars = Vec::new();
        while vars.len() < n
            && let Kind::Lam(v, b) = &self.node(arena, t).kind
        {
            vars.push(*v);
            t = *b;
        }
        (vars, t)
    }

    /// Normalises the body under the first `arity` `Lam`s of `t`, or returns `None` when a parameter or the body has a function type.
    fn function(
        &mut self,
        arena: usize,
        t: Tm,
        arity: usize,
    ) -> Result<Option<(Vec<Var>, Tm)>, String> {
        let (vars, body) = self.under_lams(arena, t, arity);
        let first_order = vars
            .iter()
            .all(|v| !self.src[arena].var_info(*v).ty.mentions_fn())
            && !self.node(arena, body).ty.mentions_fn();
        if !first_order {
            return Ok(None);
        }
        let params: Vec<Var> = vars.iter().map(|v| self.free_var(arena, *v)).collect();
        let v = self.norm(body, &Cx::new(arena, Vec::new()))?;
        Ok(Some((params, self.first_order(v)?)))
    }

    /// Normalises `t`, a `Lam` over one row, to its variable and body.
    fn row_function(&mut self, t: Tm) -> Result<(Var, Tm), String> {
        match self.function(MAIN, t, 1)? {
            Some((vars, body)) => Ok((vars[0], body)),
            None => Err("a function of a row has a function type".into()),
        }
    }

    fn first_order(&self, v: Val) -> Result<Tm, String> {
        match v {
            Val::Tm(t) => Ok(t),
            _ => Err(
                "a function value remains where the normal form needs a first-order value".into(),
            ),
        }
    }

    fn free_vars(&mut self, arena: usize, t: Tm) -> Rc<Vec<Var>> {
        let mut stack = vec![t];
        while let Some(&s) = stack.last() {
            if self.fv[arena].contains_key(&s) {
                stack.pop();
                continue;
            }
            let open: Vec<Tm> = self.src[arena]
                .children(s)
                .into_iter()
                .filter(|c| !self.fv[arena].contains_key(c))
                .collect();
            if open.is_empty() {
                stack.pop();
                let fv = self.own_free_vars(arena, s);
                self.fv[arena].insert(s, fv);
            } else {
                stack.extend(open);
            }
        }
        self.fv[arena][&t].clone()
    }

    /// Returns the free variables of `t`, whose children's are known.
    fn own_free_vars(&self, arena: usize, t: Tm) -> Rc<Vec<Var>> {
        let node = self.node(arena, t);
        let mut out: Vec<Var> = Vec::new();
        let add = |n: &Self, s: Tm, bound: &[Var], out: &mut Vec<Var>| {
            for v in n.fv[arena][&s].iter() {
                if !bound.contains(v) && !out.contains(v) {
                    out.push(*v);
                }
            }
        };
        match &node.kind {
            Kind::Var(v) => out.push(*v),
            Kind::Lam(v, b) => add(self, *b, &[*v], &mut out),
            Kind::Let(v, a, b) | Kind::LetBang(v, a, b) | Kind::For(v, a, b) => {
                add(self, *a, &[], &mut out);
                add(self, *b, &[*v], &mut out);
            }
            Kind::Upsert {
                rows,
                old,
                new,
                otherwise,
                ..
            } => {
                add(self, *rows, &[], &mut out);
                add(self, *otherwise, &[*old, *new], &mut out);
            }
            Kind::Case { scrutinee, arms } => {
                add(self, *scrutinee, &[], &mut out);
                for (_, v, e) in arms {
                    add(self, *e, &[*v], &mut out);
                }
            }
            Kind::Agg {
                var, source, body, ..
            } => {
                add(self, *source, &[], &mut out);
                add(self, *body, &[*var], &mut out);
            }
            Kind::Group { var, source, by } => {
                add(self, *source, &[], &mut out);
                add(self, *by, &[*var], &mut out);
            }
            Kind::Top {
                count,
                keys,
                var,
                source,
            } => {
                add(self, *count, &[], &mut out);
                add(self, *source, &[], &mut out);
                for (k, _) in keys {
                    add(self, *k, &[*var], &mut out);
                }
            }
            Kind::Present {
                var,
                source,
                keys,
                take,
                after,
            } => {
                add(self, *source, &[], &mut out);
                for (k, _) in keys {
                    add(self, *k, &[*var], &mut out);
                }
                for t in take.iter().chain(after.iter()) {
                    add(self, *t, &[], &mut out);
                }
            }
            Kind::Fix(binds, body) => {
                let xs: Vec<Var> = binds.iter().map(|(x, _)| *x).collect();
                for (_, e) in binds {
                    add(self, *e, &xs, &mut out);
                }
                add(self, *body, &xs, &mut out);
            }
            _ => {
                for c in self.src[arena].children(t) {
                    add(self, c, &[], &mut out);
                }
            }
        }
        Rc::new(out)
    }

    /// Returns the memo key of `t` in `cx`, or `None` when a free variable of `t` is bound to a function.
    fn key(&mut self, t: Tm, cx: &Cx) -> Option<MemoKey> {
        let fv = self.free_vars(cx.arena, t);
        let mut vals = Vec::with_capacity(fv.len());
        for v in fv.iter() {
            match cx.env.get(*v) {
                Some(Val::Tm(x)) => vals.push(Some(*x)),
                Some(_) => return None,
                None => vals.push(None),
            }
        }
        Some((cx.arena, t, cx.targs.to_vec(), vals))
    }

    fn norm(&mut self, t: Tm, cx: &Cx) -> Result<Val, String> {
        let key = self.key(t, cx);
        if let Some(k) = &key
            && let Some(o) = self.memo.get(k)
        {
            return Ok(Val::Tm(*o));
        }
        let v = self.step(t, cx)?;
        if let (Some(k), Val::Tm(o)) = (key, &v)
            && !self.draws(*o)
        {
            self.memo.insert(k, *o);
        }
        Ok(v)
    }

    fn tm(&mut self, t: Tm, cx: &Cx) -> Result<Tm, String> {
        let v = self.norm(t, cx)?;
        self.first_order(v)
    }

    fn tms(&mut self, ts: &[Tm], cx: &Cx) -> Result<Vec<Tm>, String> {
        ts.iter().map(|t| self.tm(*t, cx)).collect()
    }

    /// Normalises `body` with `var` bound to a fresh output variable, returning both.
    fn under(&mut self, var: Var, body: Tm, cx: &Cx) -> Result<(Var, Tm), String> {
        let x = self.fresh(var, cx);
        let xt = self.var_tm(x);
        let cx = cx.with(var, Val::Tm(xt));
        Ok((x, self.tm(body, &cx)?))
    }

    fn var_tm(&mut self, x: Var) -> Tm {
        let ty = self.out.var_info(x).ty.clone();
        self.add(Kind::Var(x), ty)
    }

    fn step(&mut self, t: Tm, cx: &Cx) -> Result<Val, String> {
        let node = self.node(cx.arena, t);
        let ty = node.ty.subst(&cx.targs);
        match &node.kind {
            Kind::Var(v) => Ok(self.var_val(*v, cx)),
            Kind::Const(_)
            | Kind::Caller
            | Kind::Rel(..)
            | Kind::Transition(..)
            | Kind::EmptySet
            | Kind::Fresh(..)
            | Kind::Unreachable
            | Kind::Skip => Ok(self.val(node.kind.clone(), ty)),
            Kind::Field(x, l) => self.field_step(t, *x, l, ty, cx),
            Kind::Case { scrutinee, arms } => self.case_step(*scrutinee, arms, ty, cx),
            Kind::If(..) | Kind::Let(..) | Kind::LetBang(..) | Kind::Block(..) => self.spine(t, cx),
            Kind::Lam(v, b) => Ok(Val::Lam {
                var: *v,
                body: *b,
                cx: cx.clone(),
            }),
            Kind::App(f, a) => self.app_step(*f, *a, ty, cx),
            Kind::Call {
                func,
                type_args,
                args,
            } => self.call_step(*func, type_args, args, ty, cx),
            Kind::For(v, s, b) => {
                let s = self.tm(*s, cx)?;
                Ok(Val::Tm(self.for_over(*v, s, *b, cx, &ty)?))
            }
            Kind::Agg {
                op,
                var,
                source,
                body,
            } => self.agg_step(*op, *var, *source, *body, ty, cx),
            Kind::Group { var, source, by } => self.group_step(*var, *source, *by, ty, cx),
            Kind::Top {
                count,
                keys,
                var,
                source,
            } => self.top_step(*count, keys, *var, *source, ty, cx),
            Kind::Present { .. } => self.present_step(&node.kind, ty, cx),
            Kind::Fix(binds, body) => self.fix_step(binds, *body, ty, cx),
            Kind::Replace(r, rows, f) => self.replace_step(*r, *rows, *f, ty, cx),
            Kind::Upsert { .. } => self.upsert_step(&node.kind, ty, cx),
            Kind::Prim(..)
            | Kind::Record(_)
            | Kind::Ctor { .. }
            | Kind::Key(_)
            | Kind::KeyOf(..)
            | Kind::Deref { .. }
            | Kind::Tag(_)
            | Kind::Try(_)
            | Kind::Restrict(..)
            | Kind::Singleton(_)
            | Kind::Union(..)
            | Kind::IsEmpty(_)
            | Kind::Exists(_)
            | Kind::Only(_)
            | Kind::At { .. }
            | Kind::Reject(_)
            | Kind::Insert(..)
            | Kind::Delete(..)
            | Kind::Rejects { .. }
            | Kind::Par(_)
            | Kind::Fail(_)
            | Kind::Transact(..) => self.congruent(&node.kind, ty, cx),
        }
    }

    /// Normalise a chain of `Let`, `LetBang`, `Block` and `If` through their bodies and else
    /// branches in a loop, memoising each tail where the chain turns between `If` and the others.
    fn spine(&mut self, mut t: Tm, cx: &Cx) -> Result<Val, String> {
        let mut cx = cx.clone();
        let mut wraps = Vec::new();
        let mut in_if = matches!(self.node(cx.arena, t).kind, Kind::If(..));
        let inner = loop {
            let node = self.node(cx.arena, t);
            let is_if = matches!(node.kind, Kind::If(..));
            let links = matches!(
                node.kind,
                Kind::If(..) | Kind::Let(..) | Kind::LetBang(..) | Kind::Block(..)
            );
            if links && is_if != in_if {
                in_if = is_if;
                let key = self.key(t, &cx);
                if let Some(k) = &key
                    && let Some(o) = self.memo.get(k)
                {
                    break Val::Tm(*o);
                }
                if let Some(k) = key {
                    wraps.push(Wrap::Memo(k));
                }
            }
            let ty = node.ty.subst(&cx.targs);
            match &node.kind {
                Kind::If(c, a, b) => {
                    let c = self.tm(*c, &cx)?;
                    if let Some((k, _)) = self.ctor_value(c)
                        && self.is_bool(c)
                    {
                        t = if k == 1 { *a } else { *b };
                        continue;
                    }
                    let a = self.norm(*a, &cx)?;
                    wraps.push(Wrap::If(c, a, ty));
                    t = *b;
                }
                Kind::Block(a, b) => {
                    let a = self.tm(*a, &cx)?;
                    wraps.push(Wrap::Block(a, ty));
                    t = *b;
                }
                Kind::LetBang(v, a, b) => {
                    let a = self.tm(*a, &cx)?;
                    let x = self.fresh(*v, &cx);
                    let xt = self.var_tm(x);
                    cx = cx.with(*v, Val::Tm(xt));
                    wraps.push(Wrap::LetBang(x, a, ty));
                    t = *b;
                }
                Kind::Let(v, a, b) => {
                    match self.norm(*a, &cx)? {
                        Val::Tm(bound) if self.copyable(bound) => {
                            cx = cx.with(*v, Val::Tm(bound));
                        }
                        Val::Tm(bound) => {
                            let x = self.fresh(*v, &cx);
                            let xt = self.var_tm(x);
                            cx = cx.with(*v, Val::Tm(xt));
                            wraps.push(Wrap::Let(x, bound, ty));
                        }
                        f if self.spine_copyable(&f) => cx = cx.with(*v, f),
                        f => {
                            let k = Then::Bind {
                                var: *v,
                                body: *b,
                                cx: cx.clone(),
                                ty,
                            };
                            break self.then(f, k)?;
                        }
                    }
                    t = *b;
                }
                _ => break self.norm(t, &cx)?,
            }
        };
        wraps
            .into_iter()
            .rev()
            .try_fold(inner, |v, w| self.wrap(w, v))
    }

    /// Returns `body` under the binder, the write or the condition `w` stands for.
    fn wrap(&mut self, w: Wrap, body: Val) -> Result<Val, String> {
        Ok(match w {
            Wrap::If(c, a, ty) => match (a, body) {
                (Val::Tm(a), Val::Tm(b)) => self.val(Kind::If(c, a, b), ty),
                (a, b) => Val::If(c, Rc::new(a), Rc::new(b)),
            },
            Wrap::Memo(k) => {
                if let Val::Tm(o) = &body
                    && !self.draws(*o)
                {
                    self.memo.insert(k, *o);
                }
                body
            }
            Wrap::Block(a, ty) => {
                let b = self.first_order(body)?;
                self.val(Kind::Block(a, b), ty)
            }
            Wrap::LetBang(x, a, ty) => {
                let b = self.first_order(body)?;
                self.val(Kind::LetBang(x, a, b), ty)
            }
            Wrap::Let(x, bound, ty) => match body {
                Val::Tm(r) => self.val(Kind::Let(x, bound, r), ty),
                v => Val::Let(x, bound, Rc::new(v)),
            },
        })
    }

    fn val(&mut self, kind: Kind, ty: Ty) -> Val {
        Val::Tm(self.add(kind, ty))
    }

    fn var_val(&mut self, v: Var, cx: &Cx) -> Val {
        match cx.env.get(v) {
            Some(val) => val.clone(),
            None => {
                let x = self.free_var(cx.arena, v);
                Val::Tm(self.var_tm(x))
            }
        }
    }

    /// Normalises each child of `kind` in order, binding nothing.
    fn congruent(&mut self, kind: &Kind, ty: Ty, cx: &Cx) -> Result<Val, String> {
        let cs = self.tms(&kind.children(), cx)?;
        Ok(self.val(kind.with_children(&cs), ty))
    }

    fn field_step(&mut self, source: Tm, x: Tm, l: &str, ty: Ty, cx: &Cx) -> Result<Val, String> {
        let carry = self.src[cx.arena].transport_fields.contains(&source);
        let x = self.tm(x, cx)?;
        let fs = match self.out.kind(x) {
            Kind::Record(fs) | Kind::Ctor { fields: fs, .. } => Some(fs.clone()),
            _ => None,
        };
        if let Some(fs) = fs
            && let Some((_, e)) = fs.iter().find(|(n, _)| n == l)
        {
            let e = *e;
            let others: Vec<Tm> = fs.iter().filter(|(n, _)| n != l).map(|(_, t)| *t).collect();
            if others.into_iter().all(|o| self.copyable(o)) {
                if !carry && self.out.transport_fields.contains(&e) {
                    let node = self.out.node(e).clone();
                    return Ok(self.val(node.kind, node.ty));
                }
                return Ok(Val::Tm(e));
            }
        }
        let tm = self.add(Kind::Field(x, l.to_string()), ty);
        if carry {
            self.out.transport_fields.insert(tm);
        }
        Ok(Val::Tm(tm))
    }

    fn case_step(
        &mut self,
        scrutinee: Tm,
        arms: &[(u32, Var, Tm)],
        ty: Ty,
        cx: &Cx,
    ) -> Result<Val, String> {
        let s = self.tm(scrutinee, cx)?;
        if let Some((c, fields)) = self.ctor_value(s)
            && fields.into_iter().all(|f| self.copyable(f))
            && let Some((_, v, e)) = arms.iter().find(|(a, _, _)| *a == c)
        {
            let cx = cx.with(*v, Val::Tm(s));
            return self.norm(*e, &cx);
        }
        let mut out = Vec::with_capacity(arms.len());
        for (c, v, e) in arms {
            let x = self.fresh(*v, cx);
            let xt = self.var_tm(x);
            let cx = cx.with(*v, Val::Tm(xt));
            out.push((*c, x, self.norm(*e, &cx)?));
        }
        if out.iter().all(|(_, _, v)| matches!(v, Val::Tm(_))) {
            let arms = out
                .into_iter()
                .map(|(c, x, v)| match v {
                    Val::Tm(t) => (c, x, t),
                    _ => unreachable!("checked first-order"),
                })
                .collect();
            return Ok(self.val(Kind::Case { scrutinee: s, arms }, ty));
        }
        Ok(Val::Case(s, Rc::new(out)))
    }

    fn app_step(&mut self, f: Tm, a: Tm, ty: Ty, cx: &Cx) -> Result<Val, String> {
        let f = self.norm(f, cx)?;
        let a = self.norm(a, cx)?;
        self.then(f, Then::Apply(a, ty))
    }

    fn call_step(
        &mut self,
        func: FnId,
        type_args: &[Ty],
        args: &[Tm],
        ty: Ty,
        cx: &Cx,
    ) -> Result<Val, String> {
        let type_args: Vec<Ty> = type_args.iter().map(|t| t.subst(&cx.targs)).collect();
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(self.norm(*a, cx)?);
        }
        let Some(lam) = self.fns.get(func.0 as usize).copied().flatten() else {
            let args = vals
                .into_iter()
                .map(|v| self.first_order(v))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(self.val(
                Kind::Call {
                    func,
                    type_args,
                    args,
                },
                ty,
            ));
        };
        if vals.is_empty() {
            let unit = self.add(Kind::Record(Vec::new()), Ty::Unit);
            vals.push(Val::Tm(unit));
        }
        let mut fty = self.node(FNS, lam).ty.subst(&type_args);
        let mut f = self.norm(lam, &Cx::new(FNS, type_args))?;
        for a in vals {
            let Ty::Fn(_, r) = fty else {
                return Err("a call with more arguments than its function takes".into());
            };
            fty = *r;
            f = self.then(f, Then::Apply(a, fty.clone()))?;
        }
        Ok(f)
    }

    fn agg_step(
        &mut self,
        op: Agg,
        var: Var,
        source: Tm,
        body: Tm,
        ty: Ty,
        cx: &Cx,
    ) -> Result<Val, String> {
        let source = self.tm(source, cx)?;
        let (var, body) = self.under(var, body, cx)?;
        Ok(self.val(
            Kind::Agg {
                op,
                var,
                source,
                body,
            },
            ty,
        ))
    }

    fn group_step(&mut self, var: Var, source: Tm, by: Tm, ty: Ty, cx: &Cx) -> Result<Val, String> {
        let source = self.tm(source, cx)?;
        let (var, by) = self.under(var, by, cx)?;
        Ok(self.val(Kind::Group { var, source, by }, ty))
    }

    fn top_step(
        &mut self,
        count: Tm,
        keys: &[(Tm, bool)],
        var: Var,
        source: Tm,
        ty: Ty,
        cx: &Cx,
    ) -> Result<Val, String> {
        let count = self.tm(count, cx)?;
        let source = self.tm(source, cx)?;
        let (var, keys) = self.keys(var, keys, cx)?;
        Ok(self.val(
            Kind::Top {
                count,
                keys,
                var,
                source,
            },
            ty,
        ))
    }

    fn present_step(&mut self, kind: &Kind, ty: Ty, cx: &Cx) -> Result<Val, String> {
        let Kind::Present {
            var,
            source,
            keys,
            take,
            after,
        } = kind
        else {
            unreachable!("present_step is given a present")
        };
        let source = self.tm(*source, cx)?;
        let (var, keys) = self.keys(*var, keys, cx)?;
        let take = take.map(|t| self.tm(t, cx)).transpose()?;
        let after = after.map(|t| self.tm(t, cx)).transpose()?;
        Ok(self.val(
            Kind::Present {
                var,
                source,
                keys,
                take,
                after,
            },
            ty,
        ))
    }

    fn fix_step(&mut self, binds: &[(Var, Tm)], body: Tm, ty: Ty, cx: &Cx) -> Result<Val, String> {
        let mut inner = cx.clone();
        let mut xs = Vec::with_capacity(binds.len());
        for (v, _) in binds {
            let x = self.fresh(*v, cx);
            let xt = self.var_tm(x);
            inner = inner.with(*v, Val::Tm(xt));
            xs.push(x);
        }
        let mut out = Vec::with_capacity(binds.len());
        for (x, (_, e)) in xs.into_iter().zip(binds) {
            out.push((x, self.tm(*e, &inner)?));
        }
        match self.norm(body, &inner)? {
            Val::Tm(b) => Ok(self.val(Kind::Fix(out, b), ty)),
            v => Ok(Val::Fix(out, Rc::new(v))),
        }
    }

    fn replace_step(&mut self, r: RelId, rows: Tm, f: Tm, ty: Ty, cx: &Cx) -> Result<Val, String> {
        let rows = self.tm(rows, cx)?;
        let fty = self.node(cx.arena, f).ty.subst(&cx.targs);
        let f = self.norm(f, cx)?;
        let f = self.eta(f, fty)?;
        Ok(self.val(Kind::Replace(r, rows, f), ty))
    }

    fn upsert_step(&mut self, kind: &Kind, ty: Ty, cx: &Cx) -> Result<Val, String> {
        let Kind::Upsert {
            rel,
            rows,
            on,
            old,
            new,
            otherwise,
            single,
        } = kind
        else {
            unreachable!("upsert_step is given an upsert")
        };
        let rows = self.tm(*rows, cx)?;
        let o = self.fresh(*old, cx);
        let n = self.fresh(*new, cx);
        let (ot, nt) = (self.var_tm(o), self.var_tm(n));
        let inner = cx.with(*old, Val::Tm(ot)).with(*new, Val::Tm(nt));
        let otherwise = self.tm(*otherwise, &inner)?;
        Ok(self.val(
            Kind::Upsert {
                rel: *rel,
                rows,
                on: *on,
                old: o,
                new: n,
                otherwise,
                single: *single,
            },
            ty,
        ))
    }

    fn keys(
        &mut self,
        var: Var,
        keys: &[(Tm, bool)],
        cx: &Cx,
    ) -> Result<(Var, Vec<(Tm, bool)>), String> {
        let x = self.fresh(var, cx);
        let xt = self.var_tm(x);
        let cx = cx.with(var, Val::Tm(xt));
        let mut out = Vec::with_capacity(keys.len());
        for (k, desc) in keys {
            out.push((self.tm(*k, &cx)?, *desc));
        }
        Ok((x, out))
    }

    /// Returns the constructor and fields of `t` when it is a constructor value.
    fn ctor_value(&self, t: Tm) -> Option<(u32, Vec<Tm>)> {
        match self.out.kind(t) {
            Kind::Ctor { ctor, fields } => Some((*ctor, fields.iter().map(|(_, f)| *f).collect())),
            _ => None,
        }
    }

    fn is_bool(&self, t: Tm) -> bool {
        matches!(self.out.ty(t), Ty::Data { def, .. } if *def == self.tot.schema().prelude_type("Bool"))
    }

    /// Binds `var` to `bound` in `body`: substituting a copyable value, keeping a `let` for any other.
    fn bind(&mut self, var: Var, bound: Val, body: Tm, cx: &Cx, ty: Ty) -> Result<Val, String> {
        match bound {
            Val::Tm(b) if self.copyable(b) => {
                let cx = cx.with(var, Val::Tm(b));
                self.norm(body, &cx)
            }
            Val::Tm(b) => {
                let x = self.fresh(var, cx);
                let xt = self.var_tm(x);
                let cx = cx.with(var, Val::Tm(xt));
                match self.norm(body, &cx)? {
                    Val::Tm(r) => Ok(Val::Tm(self.add(Kind::Let(x, b, r), ty))),
                    v => Ok(Val::Let(x, b, Rc::new(v))),
                }
            }
            f if self.spine_copyable(&f) => {
                let cx = cx.with(var, f);
                self.norm(body, &cx)
            }
            f => self.then(
                f,
                Then::Bind {
                    var,
                    body,
                    cx: cx.clone(),
                    ty,
                },
            ),
        }
    }

    /// Returns `λx. f x` for the function `f` of type `ty`, normalising the application.
    fn eta(&mut self, f: Val, ty: Ty) -> Result<Tm, String> {
        let Ty::Fn(a, r) = &ty else {
            return Err("a row function is not a function".into());
        };
        let x = self.out.var("ξ", (**a).clone());
        let xt = self.var_tm(x);
        let body = self.then(f, Then::Apply(Val::Tm(xt), (**r).clone()))?;
        let body = self.first_order(body)?;
        Ok(self.add(Kind::Lam(x, body), ty))
    }

    /// Returns whether choosing the function `v` evaluates only copyable terms.
    fn spine_copyable(&mut self, v: &Val) -> bool {
        match v {
            Val::Tm(t) => self.copyable(*t),
            Val::Lam { .. } => true,
            Val::If(c, a, b) => {
                self.copyable(*c) && self.spine_copyable(a) && self.spine_copyable(b)
            }
            Val::Case(s, arms) => {
                self.copyable(*s) && arms.iter().all(|(_, _, v)| self.spine_copyable(v))
            }
            Val::Let(_, b, v) => self.copyable(*b) && self.spine_copyable(v),
            Val::Fix(..) => false,
        }
    }

    /// Continues with the function `f` by `k`, pushing `k` into the branches that choose it.
    fn then(&mut self, f: Val, k: Then) -> Result<Val, String> {
        match f {
            Val::Tm(_) => {
                Err("an application of a function that normalisation cannot resolve".into())
            }
            Val::Lam { var, body, cx } => match k {
                Then::Apply(arg, ty) => self.bind(var, arg, body, &cx, ty),
                Then::Bind {
                    var: v,
                    body: b,
                    cx: outer,
                    ..
                } => {
                    let lam = Val::Lam { var, body, cx };
                    self.norm(b, &outer.with(v, lam))
                }
            },
            Val::If(c, a, b) => {
                let ty = k.ty().clone();
                let a = self.then((*a).clone(), k.clone())?;
                let b = self.then((*b).clone(), k)?;
                match (a, b) {
                    (Val::Tm(a), Val::Tm(b)) => Ok(Val::Tm(self.add(Kind::If(c, a, b), ty))),
                    (a, b) => Ok(Val::If(c, Rc::new(a), Rc::new(b))),
                }
            }
            Val::Case(s, arms) => {
                let ty = k.ty().clone();
                let mut out = Vec::with_capacity(arms.len());
                for (c, x, v) in arms.iter() {
                    out.push((*c, *x, self.then(v.clone(), k.clone())?));
                }
                if out.iter().all(|(_, _, v)| matches!(v, Val::Tm(_))) {
                    let arms = out
                        .into_iter()
                        .map(|(c, x, v)| match v {
                            Val::Tm(t) => (c, x, t),
                            _ => unreachable!("checked first-order"),
                        })
                        .collect();
                    return Ok(Val::Tm(self.add(Kind::Case { scrutinee: s, arms }, ty)));
                }
                Ok(Val::Case(s, Rc::new(out)))
            }
            Val::Let(x, b, v) => {
                let ty = k.ty().clone();
                match self.then((*v).clone(), k)? {
                    Val::Tm(r) => Ok(Val::Tm(self.add(Kind::Let(x, b, r), ty))),
                    v => Ok(Val::Let(x, b, Rc::new(v))),
                }
            }
            Val::Fix(binds, v) => {
                let ty = k.ty().clone();
                match self.then((*v).clone(), k)? {
                    Val::Tm(r) => Ok(Val::Tm(self.add(Kind::Fix(binds, r), ty))),
                    v => Ok(Val::Fix(binds, Rc::new(v))),
                }
            }
        }
    }

    /// Normalises `for var in s do body` for an already normalised source `s`.
    fn for_over(&mut self, var: Var, s: Tm, body: Tm, cx: &Cx, ty: &Ty) -> Result<Tm, String> {
        match self.out.kind(s).clone() {
            Kind::EmptySet => Ok(self.add(Kind::EmptySet, ty.clone())),
            Kind::Singleton(e) if self.copyable(e) => {
                let cx = cx.with(var, Val::Tm(e));
                self.tm(body, &cx)
            }
            Kind::Union(a, b) => {
                let a = self.for_over(var, a, body, cx, ty)?;
                let b = self.for_over(var, b, body, cx, ty)?;
                Ok(self.add(Kind::Union(a, b), ty.clone()))
            }
            Kind::For(y, a, b) => {
                let inner = self.for_over(var, b, body, cx, ty)?;
                Ok(self.add(Kind::For(y, a, inner), ty.clone()))
            }
            Kind::If(c, a, b) => {
                let a = self.for_over(var, a, body, cx, ty)?;
                let b = self.for_over(var, b, body, cx, ty)?;
                Ok(self.add(Kind::If(c, a, b), ty.clone()))
            }
            _ => {
                let (x, b) = self.under(var, body, cx)?;
                Ok(self.add(Kind::For(x, s, b), ty.clone()))
            }
        }
    }
}

impl Then {
    fn ty(&self) -> &Ty {
        match self {
            Then::Apply(_, ty) | Then::Bind { ty, .. } => ty,
        }
    }
}
