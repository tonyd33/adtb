//! Normalise checked programs to the terms planners and the reference interpreter run.

use std::collections::HashMap;
use std::rc::Rc;

use crate::analysis::Totality;
use crate::capability::{self, RootKind};
use crate::check::{Adhoc, AdhocCommand, Program, Query, Txn};
use crate::kernel::{Agg, Build, Frees, Invalid, Kind, Node, Terms, Tm, Var};
use crate::schema::Schema;
use crate::types::*;

mod limit;
mod normal;

pub use limit::expands_within_bound;
use limit::{Budget, TOO_LARGE, planned_within_bound};
pub use normal::{AdhocParts, NormalAdhoc, NormalAggregate, NormalProgram, NormalQuery};

/// A body of a program or of ad hoc text, by the object it belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Root {
    Function(FnId),
    Constraint(ConstraintIdx),
    Check(CheckIdx),
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
    invalid: Option<Invalid>,
}

impl Error {
    /// Returns the error of a term of `terms` that breaks an invariant of the kernel.
    fn invalid(schema: &Schema, root: Root, terms: &Terms, e: Invalid) -> Error {
        Error {
            root,
            message: format!(
                "kernel term {:?} breaks the invariant that the checker guarantees: {} ({:?} : {})",
                e.at(),
                e.reason(),
                terms.kind(e.at()),
                schema.show_ty(terms.ty(e.at()))
            ),
            invalid: Some(e),
        }
    }

    /// Returns the kernel term that breaks an invariant of the kernel, when this is the error.
    pub fn broken(&self) -> Option<&Invalid> {
        self.invalid.as_ref()
    }

    /// Returns whether the root was refused for passing a limit on what is normalised.
    pub fn past_limit(&self) -> bool {
        self.message == TOO_LARGE
    }
}

fn at<T>(root: Root, r: Result<T, String>) -> Result<T, Error> {
    r.map_err(|message| Error {
        root,
        message,
        invalid: None,
    })
}

/// Normalise every body of a checked schema.
pub fn program(schema: &Schema, p: &Program) -> Result<NormalProgram, Error> {
    let program = NormalProgram::normalised(schema, p)?;
    planned_within_bound(&program)?;
    Ok(program)
}

/// Normalise checked ad hoc text, inlining the functions of `p`.
pub fn adhoc(schema: &Schema, p: &Program, a: &Adhoc) -> Result<NormalAdhoc, Error> {
    let mut n = Normaliser::new(schema, [&a.terms, &p.terms], p);
    n.text = a.kind();
    let params = a
        .params
        .iter()
        .map(|(name, v)| (name.clone(), n.free_var(MAIN, *v)))
        .collect();
    let mut items = Vec::with_capacity(a.items.len());
    for (i, t) in a.items.iter().enumerate() {
        items.push(n.root(Root::Item(i), *t)?);
    }
    let mut commands = Vec::with_capacity(a.commands.len());
    for (i, c) in a.commands.iter().enumerate() {
        let terms = c
            .command
            .terms()
            .into_iter()
            .map(|t| n.root(Root::Command(i), t))
            .collect::<Result<Vec<Tm>, Error>>()?;
        commands.push(AdhocCommand {
            command: c.command.with_terms(&terms),
            ..c.clone()
        });
    }
    NormalAdhoc::validated(
        schema,
        &[&a.terms, &p.terms],
        (n.out, items, commands, params),
    )
}

/// Normalise a checked registered query, inlining the functions of `p`.
pub fn query(schema: &Schema, p: &Program, q: &Query) -> Result<NormalQuery, Error> {
    query_as(schema, p, q, RootKind::Query)
}

/// Normalise a checked registered query as [`query`] does, as a root of `kind`.
pub fn query_as(
    schema: &Schema,
    p: &Program,
    q: &Query,
    kind: RootKind,
) -> Result<NormalQuery, Error> {
    registered(schema, p, &q.terms, q.term, kind)
}

/// Normalise a checked registered transaction, inlining the functions of `p`.
pub fn txn(schema: &Schema, p: &Program, t: &Txn) -> Result<NormalQuery, Error> {
    txn_as(schema, p, t, RootKind::Query)
}

/// Normalise a checked registered transaction as [`txn`] does, as a root of `kind`.
pub fn txn_as(schema: &Schema, p: &Program, t: &Txn, kind: RootKind) -> Result<NormalQuery, Error> {
    registered(schema, p, &t.terms, t.term, kind)
}

fn registered(
    schema: &Schema,
    p: &Program,
    terms: &Terms,
    term: Tm,
    kind: RootKind,
) -> Result<NormalQuery, Error> {
    let mut n = Normaliser::new(schema, [terms, &p.terms], p);
    n.text = kind;
    let (params, body) = n.under_lams(MAIN, term, usize::MAX);
    let params: Vec<Var> = params.into_iter().map(|v| n.free_var(MAIN, v)).collect();
    let body = n.root(Root::Query, body)?;
    if let Some(e) = terms.broken() {
        return Err(Error::invalid(schema, Root::Query, terms, e.clone()));
    }
    NormalQuery::validated(schema, n.out, params, body)
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

/// Each free variable of a term with its number of occurrences.
type FreeVars = Rc<Vec<(Var, usize)>>;

struct Normaliser<'a> {
    schema: &'a Schema,
    src: [&'a Terms; 2],
    fns: &'a [Option<Tm>],
    program: &'a Program,
    out: Terms,
    out_frees: Frees,
    tot: Totality<'a>,
    free: HashMap<(usize, Var), Var>,
    fv: [HashMap<Tm, FreeVars>; 2],
    memo: HashMap<MemoKey, Tm>,
    budget: Budget,
    /// The kind of the text normalised: its ad hoc items and commands, or its registered body.
    text: RootKind,
    /// Whether every deref of the root being normalised is partial.
    unchecked: bool,
    /// Whether the root being normalised reads through a pin hiding new constructors.
    hiding: bool,
}

impl<'a> Normaliser<'a> {
    fn new(schema: &'a Schema, src: [&'a Terms; 2], p: &'a Program) -> Self {
        let fns = &p.functions;
        Normaliser {
            schema,
            src,
            fns,
            program: p,
            out: Terms::default(),
            out_frees: Frees::default(),
            tot: Totality::new(schema, false).with_aggregates(&p.terms, p.aggregate_parts()),
            free: HashMap::new(),
            fv: [HashMap::new(), HashMap::new()],
            memo: HashMap::new(),
            budget: Budget::new(src[0].len() + src[1].len()),
            text: RootKind::Adhoc,
            unchecked: false,
            hiding: false,
        }
    }

    /// Normalise what follows as the body of `root`.
    fn enter(&mut self, root: Root) {
        let kind = match root {
            Root::Item(_) | Root::Command(_) | Root::Query => self.text,
            _ => capability::kind_of(self.schema, root),
        };
        let entry = capability::entry(kind);
        self.mode(entry.derefs_partial(), entry.hiding());
        self.budget.start(&self.out);
    }

    /// Normalise what follows with every deref partial when `unchecked`, judging totality as
    /// under a hiding pin when `hiding`. Clear the memo when the mode changes: a nullary
    /// function's memo key is the same in every root.
    fn mode(&mut self, unchecked: bool, hiding: bool) {
        if hiding != self.hiding {
            self.hiding = hiding;
            self.tot = Totality::new(self.schema, hiding)
                .with_aggregates(&self.program.terms, self.program.aggregate_parts());
            self.memo.clear();
        }
        if unchecked != self.unchecked {
            self.unchecked = unchecked;
            self.memo.clear();
        }
    }

    /// Normalise an `at` block, its derefs as the entry of an `at` body states.
    fn at_step(&mut self, kind: &Kind, ty: Ty, cx: &Cx) -> Result<Val, String> {
        let (unchecked, hiding) = (self.unchecked, self.hiding);
        let body = capability::entry(RootKind::AtBody);
        self.mode(unchecked || body.derefs_partial(), hiding);
        let v = self.congruent(kind, ty, cx);
        self.mode(unchecked, hiding);
        v
    }

    fn node(&self, arena: usize, t: Tm) -> &'a Node {
        let terms: &'a Terms = self.src[arena];
        terms.node(t)
    }

    fn copyable(&mut self, t: Tm) -> bool {
        self.tot.copyable(&self.out, t)
    }

    fn draws(&mut self, t: Tm) -> bool {
        self.tot.draws(&self.out, t)
    }

    fn b(&mut self) -> Build<'_> {
        Build::new(self.schema, &mut self.out)
    }

    /// Returns `t` with each variable its root binds replaced by a new one.
    fn bound_afresh(&mut self, t: Tm) -> Tm {
        let mut bound = Vec::new();
        self.out.kind(t).with_vars(&mut |v| {
            bound.push(v);
            v
        });
        let map = bound
            .into_iter()
            .map(|v| (v, self.out.var_like(v)))
            .collect();
        self.out.renamed(t, &map, &mut self.out_frees)
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

    fn root(&mut self, root: Root, t: Tm) -> Result<Tm, Error> {
        self.enter(root);
        let v = at(root, self.norm(t, &Cx::new(MAIN, Vec::new())))?;
        at(root, self.first_order(v))
    }

    /// Split `t` into the variables of at most `n` leading `Lam`s and the body under them.
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

    /// Normalise the body under the first `arity` `Lam`s of `t`, or return `None` when a parameter or the body has a function type.
    fn function(
        &mut self,
        root: Root,
        arena: usize,
        t: Tm,
        arity: usize,
    ) -> Result<Option<(Vec<Var>, Tm)>, Error> {
        let (vars, body) = self.under_lams(arena, t, arity);
        let first_order = vars
            .iter()
            .all(|v| !self.src[arena].var_info(*v).ty.mentions_fn())
            && !self.node(arena, body).ty.mentions_fn();
        if !first_order {
            return Ok(None);
        }
        let params: Vec<Var> = vars.iter().map(|v| self.free_var(arena, *v)).collect();
        self.enter(root);
        let v = at(root, self.norm(body, &Cx::new(arena, Vec::new())))?;
        Ok(Some((params, at(root, self.first_order(v))?)))
    }

    /// Normalise `t`, a `Lam` over one row, to its variable and body.
    fn row_function(&mut self, root: Root, t: Tm) -> Result<(Var, Tm), Error> {
        match self.function(root, MAIN, t, 1)? {
            Some((vars, body)) => Ok((vars[0], body)),
            None => at(root, Err("a function of a row has a function type".into())),
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

    fn free_vars(&mut self, arena: usize, t: Tm) -> FreeVars {
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

    /// Returns the free variables of `t` with their numbers of occurrences, whose children's are known.
    fn own_free_vars(&self, arena: usize, t: Tm) -> FreeVars {
        let node = self.node(arena, t);
        let mut out: Vec<(Var, usize)> = Vec::new();
        let add = |n: &Self, s: Tm, bound: &[Var], out: &mut Vec<(Var, usize)>| {
            for &(v, k) in n.fv[arena][&s].iter() {
                if bound.contains(&v) {
                    continue;
                }
                match out.iter_mut().find(|(w, _)| *w == v) {
                    Some((_, c)) => *c = c.saturating_add(k),
                    None => out.push((v, k)),
                }
            }
        };
        match &node.kind {
            Kind::Var(v) => out.push((*v, 1)),
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
        for (v, _) in fv.iter() {
            match cx.env.get(*v) {
                Some(Val::Tm(x)) => vals.push(Some(*x)),
                Some(_) => return None,
                None => vals.push(None),
            }
        }
        Some((cx.arena, t, cx.targs.to_vec(), vals))
    }

    fn norm(&mut self, t: Tm, cx: &Cx) -> Result<Val, String> {
        self.budget.spend(&self.out)?;
        self.budget.depth += 1;
        let v = self.norm_step(t, cx);
        self.budget.depth -= 1;
        v
    }

    fn norm_step(&mut self, t: Tm, cx: &Cx) -> Result<Val, String> {
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

    /// Normalise `body` with `var` bound to a fresh output variable, returning both.
    fn under(&mut self, var: Var, body: Tm, cx: &Cx) -> Result<(Var, Tm), String> {
        let x = self.fresh(var, cx);
        let xt = self.var_tm(x);
        let cx = cx.with(var, Val::Tm(xt));
        Ok((x, self.tm(body, &cx)?))
    }

    fn var_tm(&mut self, x: Var) -> Tm {
        self.b().var(x)
    }

    fn step(&mut self, t: Tm, cx: &Cx) -> Result<Val, String> {
        let node = self.node(cx.arena, t);
        let ty = node.ty.subst(&cx.targs);
        match &node.kind {
            Kind::Var(v) => Ok(self.var_val(*v, cx)),
            Kind::Deref {
                key,
                reference: true,
                admits,
            } if self.unchecked => {
                let partial = Kind::Deref {
                    key: *key,
                    reference: false,
                    admits: admits.clone(),
                };
                self.congruent(&partial, ty, cx)
            }
            Kind::Const(_)
            | Kind::Caller
            | Kind::Rel(..)
            | Kind::Transition(..)
            | Kind::EmptySet
            | Kind::Fresh(..)
            | Kind::Unreachable
            | Kind::Skip => Ok(self.kept(node.kind.clone(), &ty)),
            Kind::Field(x, l) => self.field_step(t, *x, l, &ty, cx),
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
            } => self.call_step(*func, type_args, args, &ty, cx),
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
            Kind::Group { var, source, by } => self.group_step(*var, *source, *by, &ty, cx),
            Kind::Top {
                count,
                keys,
                var,
                source,
            } => self.top_step(*count, keys, *var, *source, ty, cx),
            Kind::Present { .. } => self.present_step(&node.kind, ty, cx),
            Kind::At { .. } => self.at_step(&node.kind, ty, cx),
            Kind::Fix(binds, body) => self.fix_step(binds, *body, ty, cx),
            Kind::Replace(r, rows, f) => self.replace_step(*r, *rows, *f, cx),
            Kind::Upsert { .. } => self.upsert_step(&node.kind, cx),
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
            | Kind::Reject(_)
            | Kind::Insert(..)
            | Kind::Delete(..)
            | Kind::Rejects { .. }
            | Kind::Par(_)
            | Kind::Fail(_)
            | Kind::Transact(..)
            | Kind::Placeholder { .. } => self.congruent(&node.kind, ty, cx),
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
                        Val::Tm(bound) if self.substitutes(bound, *v, *b, &cx) => {
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
                (Val::Tm(a), Val::Tm(b)) => Val::Tm(self.b().if_(c, a, b, ty)),
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
                Val::Tm(self.b().block(a, b, ty))
            }
            Wrap::LetBang(x, a, ty) => {
                let b = self.first_order(body)?;
                Val::Tm(self.b().let_bang(x, a, b, ty))
            }
            Wrap::Let(x, bound, ty) => self.kept_let(x, bound, body, ty),
        })
    }

    fn kept_let(&mut self, x: Var, bound: Tm, body: Val, ty: Ty) -> Val {
        match body {
            Val::Tm(r) => Val::Tm(self.b().let_(x, bound, r, ty)),
            v => Val::Let(x, bound, Rc::new(v)),
        }
    }

    /// Returns whether binding `var` to `t` in `body` substitutes `t` at each use: `t` is copyable,
    /// and it does not branch or `body` uses `var` at most once.
    fn substitutes(&mut self, t: Tm, var: Var, body: Tm, cx: &Cx) -> bool {
        self.copyable(t)
            && (!matches!(self.out.kind(t), Kind::If(..) | Kind::Case { .. })
                || self.uses(cx.arena, body, var) <= 1)
    }

    fn uses(&mut self, arena: usize, t: Tm, var: Var) -> usize {
        self.free_vars(arena, t)
            .iter()
            .find_map(|&(v, k)| (v == var).then_some(k))
            .unwrap_or(0)
    }

    /// Returns `kind`, its children normalised, at the type `ty` it was checked at.
    fn kept(&mut self, kind: Kind, ty: &Ty) -> Val {
        Val::Tm(self.b().kept(kind, ty))
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

    /// Normalise each child of `kind` in order, binding nothing.
    fn congruent(&mut self, kind: &Kind, ty: Ty, cx: &Cx) -> Result<Val, String> {
        let cs = self.tms(&kind.children(), cx)?;
        Ok(self.kept(kind.with_children(&cs), &ty))
    }

    fn field_step(&mut self, source: Tm, x: Tm, l: &str, ty: &Ty, cx: &Cx) -> Result<Val, String> {
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
                    return Ok(self.kept(node.kind, &node.ty));
                }
                return Ok(Val::Tm(e));
            }
        }
        let tm = self.b().kept(Kind::Field(x, l.into()), ty);
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
        if let Some(c) = sole_ctor(self.schema, self.out.ty(s))
            && self.copyable(s)
            && let Some((_, v, e)) = arms.iter().find(|(a, _, _)| *a == c)
        {
            let cx = cx.with(*v, Val::Tm(s));
            return self.norm(*e, &cx);
        }
        let named = match self.node(cx.arena, scrutinee).kind {
            Kind::Var(y) => Some(y),
            _ => None,
        };
        let mut out = Vec::with_capacity(arms.len());
        for (c, v, e) in arms {
            let x = self.fresh(*v, cx);
            let xt = self.var_tm(x);
            let mut cx = cx.with(*v, Val::Tm(xt));
            if let Some(y) = named {
                cx = cx.with(y, Val::Tm(xt));
            }
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
            return Ok(Val::Tm(self.b().kept_case(s, arms, ty)));
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
        ty: &Ty,
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
            let call = Kind::Call {
                func,
                type_args,
                args,
            };
            return Ok(Val::Tm(self.b().kept(call, ty)));
        };
        if vals.is_empty() {
            let unit = self.b().unit();
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
        let agg = Kind::Agg {
            op,
            var,
            source,
            body,
        };
        Ok(Val::Tm(self.b().kept(agg, &ty)))
    }

    fn group_step(
        &mut self,
        var: Var,
        source: Tm,
        by: Tm,
        ty: &Ty,
        cx: &Cx,
    ) -> Result<Val, String> {
        let source = self.tm(source, cx)?;
        let (var, by) = self.under(var, by, cx)?;
        Ok(Val::Tm(self.b().kept(Kind::Group { var, source, by }, ty)))
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
        Ok(Val::Tm(self.b().top(count, keys, var, source, ty)))
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
        Ok(Val::Tm(
            self.b().present(var, source, keys, take, after, ty),
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
            Val::Tm(b) => Ok(Val::Tm(self.b().fix(out, b, ty))),
            v => Ok(Val::Fix(out, Rc::new(v))),
        }
    }

    fn replace_step(&mut self, r: RelId, rows: Tm, f: Tm, cx: &Cx) -> Result<Val, String> {
        let rows = self.tm(rows, cx)?;
        let fty = self.node(cx.arena, f).ty.subst(&cx.targs);
        let f = self.norm(f, cx)?;
        let f = self.eta(f, fty)?;
        Ok(Val::Tm(self.b().replace(r, rows, f)))
    }

    fn upsert_step(&mut self, kind: &Kind, cx: &Cx) -> Result<Val, String> {
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
        Ok(Val::Tm(self.b().upsert(
            *rel,
            rows,
            *on,
            (o, n),
            otherwise,
            *single,
        )))
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
        matches!(self.out.ty(t), Ty::Data { def, .. } if *def == self.tot.schema().prelude().bool())
    }

    /// Bind `var` to `bound` in `body`: substituting a copyable value that does not branch or is
    /// used at most once, keeping a `let` for any other.
    fn bind(&mut self, var: Var, bound: Val, body: Tm, cx: &Cx, ty: Ty) -> Result<Val, String> {
        match bound {
            Val::Tm(b) if self.substitutes(b, var, body, cx) => {
                let cx = cx.with(var, Val::Tm(b));
                self.norm(body, &cx)
            }
            Val::Tm(b) => {
                let x = self.fresh(var, cx);
                let xt = self.var_tm(x);
                let cx = cx.with(var, Val::Tm(xt));
                let body = self.norm(body, &cx)?;
                Ok(self.kept_let(x, b, body, ty))
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
        Ok(self.b().lam(x, body, ty))
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

    /// Continue with the function `f` by `k`, pushing `k` into the branches that choose it.
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
                    (Val::Tm(a), Val::Tm(b)) => Ok(Val::Tm(self.b().if_(c, a, b, ty))),
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
                    let t = self.b().kept_case(s, arms, ty);
                    return Ok(Val::Tm(self.bound_afresh(t)));
                }
                Ok(Val::Case(s, Rc::new(out)))
            }
            Val::Let(x, b, v) => {
                let ty = k.ty().clone();
                match self.then((*v).clone(), k)? {
                    Val::Tm(r) => {
                        let t = self.b().let_(x, b, r, ty);
                        Ok(Val::Tm(self.bound_afresh(t)))
                    }
                    v => Ok(Val::Let(x, b, Rc::new(v))),
                }
            }
            Val::Fix(binds, v) => {
                let ty = k.ty().clone();
                match self.then((*v).clone(), k)? {
                    Val::Tm(r) => {
                        let t = self.b().fix(binds, r, ty);
                        Ok(Val::Tm(self.bound_afresh(t)))
                    }
                    v => Ok(Val::Fix(binds, Rc::new(v))),
                }
            }
        }
    }

    /// Normalise `for var in s do body` for an already normalised source `s`.
    fn for_over(&mut self, var: Var, s: Tm, body: Tm, cx: &Cx, ty: &Ty) -> Result<Tm, String> {
        match self.out.kind(s).clone() {
            Kind::EmptySet => Ok(self.b().empty_set(ty.clone())),
            Kind::Singleton(e) if self.copyable(e) => {
                let cx = cx.with(var, Val::Tm(e));
                self.tm(body, &cx)
            }
            Kind::Union(a, b) => {
                let a = self.for_over(var, a, body, cx, ty)?;
                let b = self.for_over(var, b, body, cx, ty)?;
                Ok(self.b().union(a, b, ty.clone()))
            }
            Kind::For(y, a, b) => {
                let inner = self.for_over(var, b, body, cx, ty)?;
                let t = self.b().for_(y, a, inner, ty.clone());
                Ok(self.bound_afresh(t))
            }
            Kind::If(c, a, b) => {
                let a = self.for_over(var, a, body, cx, ty)?;
                let b = self.for_over(var, b, body, cx, ty)?;
                Ok(self.b().if_(c, a, b, ty.clone()))
            }
            _ => {
                let (x, b) = self.under(var, body, cx)?;
                Ok(self.b().for_(x, s, b, ty.clone()))
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

/// Returns the one constructor a value of `ty` can have, when its type has one or narrows it to one.
fn sole_ctor(schema: &Schema, ty: &Ty) -> Option<u32> {
    let one = |cs: &[u32]| match cs {
        [c] => Some(*c),
        _ => None,
    };
    match ty {
        Ty::Data {
            ctors: Some(cs), ..
        }
        | Ty::Sum { sel: Some(cs), .. } => one(cs),
        Ty::Data {
            def, ctors: None, ..
        } => (schema.ty(*def).ctors().len() == 1).then_some(0),
        Ty::Sum { ctors, sel: None } => (ctors.len() == 1).then_some(0),
        Ty::Tag(inner) => sole_ctor(schema, inner),
        _ => None,
    }
}
