use super::*;
use crate::ast::{
    BinOp, BlockItem, CallArgs, ConflictTarget, CtorScope, Expr, ExprKind, LetValue, Pattern,
    RecordExpr, TransactBlock, TxnOption, UnOp, Write, WriteArm, WriteKind,
};
use crate::kernel::{Conflict, Durability, TxnOptions};
use crate::schema::OnDelete;

/// Where a constructor value becomes a row of `rel`: omitted fields are filled from the relation, and from `old` for an update.
pub(super) struct Site {
    pub(super) rel: RelId,
    pub(super) old: Option<Tm>,
}

/// A row a nested `key (insert ...)` inserts, bound to `var` around its enclosing write.
pub(super) struct Nested {
    var: Var,
    rel: RelId,
    row: Tm,
}

/// One way a write can end: by `fail`, or with the new values its updates write per relation.
#[derive(Clone, Debug, PartialEq)]
enum Outcome {
    Fail,
    Done(Vec<(RelId, Ty)>),
}

#[derive(Clone, Debug)]
struct Effect {
    outcomes: Vec<Outcome>,
    deletes: Vec<RelId>,
}

impl Effect {
    fn done() -> Effect {
        Effect {
            outcomes: vec![Outcome::Done(Vec::new())],
            deletes: Vec::new(),
        }
    }

    fn fail() -> Effect {
        Effect {
            outcomes: vec![Outcome::Fail],
            deletes: Vec::new(),
        }
    }

    fn wrote(rel: RelId, ty: Ty) -> Effect {
        Effect {
            outcomes: vec![Outcome::Done(vec![(rel, ty)])],
            deletes: Vec::new(),
        }
    }

    fn deleted(rel: RelId) -> Effect {
        Effect {
            outcomes: vec![Outcome::Done(Vec::new())],
            deletes: vec![rel],
        }
    }

    fn either(mut self, other: Effect) -> Effect {
        for o in other.outcomes {
            if !self.outcomes.contains(&o) {
                self.outcomes.push(o);
            }
        }
        self.deletes.extend(other.deletes);
        self
    }

    fn and(self, other: Effect) -> Effect {
        let mut outcomes: Vec<Outcome> = Vec::new();
        for a in &self.outcomes {
            for b in &other.outcomes {
                let o = match (a, b) {
                    (Outcome::Done(x), Outcome::Done(y)) => {
                        Outcome::Done(x.iter().chain(y).cloned().collect())
                    }
                    _ => Outcome::Fail,
                };
                if !outcomes.contains(&o) {
                    outcomes.push(o);
                }
            }
        }
        let mut deletes = self.deletes;
        deletes.extend(other.deletes);
        Effect { outcomes, deletes }
    }
}

/// The rows an insert proposes into `rel`, at the target's row type.
struct Proposed {
    rel: RelId,
    row_ty: Ty,
    /// The single row, or the set of rows of the `from` form.
    value: Tm,
    single: bool,
}

/// How a `let` over a write binds its name.
enum LetKind {
    /// `Let` the rows, then insert them.
    Insert {
        rel: RelId,
        value: Tm,
        single: bool,
    },
    /// `Let` the rows, then delete them.
    Delete {
        rel: RelId,
        rows: Tm,
    },
    Bang(Tm),
}

const ONCE: &str = "`key (insert ...)` is allowed only where it is evaluated once per statement: not under a comprehension, a lambda, a match arm, an `if` branch or the right operand of `and` or `or`, nor in an `update ... in`, a `delete ... in` or an upsert's else branch";

/// Refuses a nested `key (insert ...)` anywhere in `e`.
fn forbid_nested(e: &Expr) -> R<()> {
    let mut found = None;
    crate::visit::each_expr(e, &mut |x| {
        if found.is_none()
            && let ExprKind::Unary(UnOp::Key, w) = &x.kind
            && matches!(w.kind, ExprKind::Write(_))
        {
            found = Some(w.span);
        }
    });
    match found {
        Some(span) => Err(diag(span, ONCE)),
        None => Ok(()),
    }
}

/// Refuses a nested `key (insert ...)` in `e` that would not be evaluated exactly once when `e` is, or at all unless `ok`.
fn once(e: &Expr, ok: bool) -> R<()> {
    if !ok {
        return forbid_nested(e);
    }
    match &e.kind {
        ExprKind::Unary(UnOp::Key, x) if matches!(x.kind, ExprKind::Write(_)) => Ok(()),
        ExprKind::Lit(_)
        | ExprKind::Name(_)
        | ExprKind::Unit
        | ExprKind::Elided
        | ExprKind::Write(_) => Ok(()),
        ExprKind::Call { args, .. } | ExprKind::TypeCall { args, .. } => match args {
            CallArgs::List(es) => es.iter().try_for_each(|x| once(x, true)),
            CallArgs::Aggregate { .. } => forbid_nested(e),
        },
        ExprKind::Ctor { record, .. } => match record {
            Some(r) => once_record(r),
            None => Ok(()),
        },
        ExprKind::Record(r) => once_record(r),
        ExprKind::Set(es) => es.iter().try_for_each(|x| once(x, true)),
        ExprKind::Unary(_, x)
        | ExprKind::Reject(x)
        | ExprKind::Field(x, _)
        | ExprKind::Select(x, _)
        | ExprKind::Backlink(x, ..) => once(x, true),
        ExprKind::Binary(BinOp::And | BinOp::Or, a, b) => {
            once(a, true)?;
            forbid_nested(b)
        }
        ExprKind::Binary(_, a, b) => {
            once(a, true)?;
            once(b, true)
        }
        ExprKind::KeyCall(x, args) => {
            once(x, true)?;
            match args {
                ast::KeyArgs::Value(v) => once(v, true),
                ast::KeyArgs::Named(ns) => ns.iter().try_for_each(|(_, v)| once(v, true)),
            }
        }
        ExprKind::Match(s, _) => {
            if let Some(s) = s {
                once(s, true)?;
            }
            forbid_nested(e)
        }
        ExprKind::If(c, a, b) => {
            once(c, true)?;
            forbid_nested(a)?;
            forbid_nested(b)
        }
        ExprKind::At { point, body, .. } => {
            once(point, true)?;
            once(body, true)
        }
        ExprKind::Comprehension(_)
        | ExprKind::Fix(..)
        | ExprKind::Lambda(..)
        | ExprKind::Exists(..)
        | ExprKind::Top { .. } => forbid_nested(e),
    }
}

fn once_record(r: &RecordExpr) -> R<()> {
    match r {
        RecordExpr::Fields(fs, _) => fs.iter().try_for_each(|(_, x)| once(x, true)),
        RecordExpr::With(b, fs, _) => {
            once(b, true)?;
            fs.iter().try_for_each(|(_, x)| once(x, true))
        }
        RecordExpr::Elided(_) => Ok(()),
    }
}

/// `(w)` is `w`.
fn ungroup(w: &Write) -> &Write {
    match &w.kind {
        WriteKind::Seq(ws) if ws.len() == 1 => ungroup(&ws[0]),
        _ => w,
    }
}

/// Returns whether two expressions are written alike, ignoring where.
fn same_expr(a: &Expr, b: &Expr) -> bool {
    let same_name = |x: &ast::QName, y: &ast::QName| {
        x.name.text == y.name.text
            && x.module.as_ref().map(|m| &m.text) == y.module.as_ref().map(|m| &m.text)
    };
    match (&a.kind, &b.kind) {
        (ExprKind::Lit(x), ExprKind::Lit(y)) => x == y,
        (ExprKind::Name(x), ExprKind::Name(y)) => same_name(x, y),
        (ExprKind::Field(x, f), ExprKind::Field(y, g)) => f.text == g.text && same_expr(x, y),
        (ExprKind::Unary(o, x), ExprKind::Unary(p, y)) => o == p && same_expr(x, y),
        (ExprKind::Binary(o, x1, x2), ExprKind::Binary(p, y1, y2)) => {
            o == p && same_expr(x1, y1) && same_expr(x2, y2)
        }
        (
            ExprKind::Call {
                callee: f,
                args: CallArgs::List(xs),
                pins: None,
            },
            ExprKind::Call {
                callee: g,
                args: CallArgs::List(ys),
                pins: None,
            },
        ) => {
            same_name(f, g)
                && xs.len() == ys.len()
                && xs.iter().zip(ys).all(|(x, y)| same_expr(x, y))
        }
        _ => false,
    }
}

/// `elem` restricted to `ctors`.
fn row_ty(elem: &Ty, ctors: Option<CtorSet>) -> Ty {
    match elem {
        Ty::Data { def, args, .. } => Ty::Data {
            def: *def,
            args: args.clone(),
            ctors,
        },
        t => t.clone(),
    }
}

/// A literal of a retag payload: a scalar literal, negated or not, or a nullary constructor by its written path.
#[derive(Clone, Debug, PartialEq)]
enum PayloadLit {
    Lit(bool, ast::Lit),
    Ctor(Option<String>, String, Option<String>),
}

/// A retag payload's fields, each with its literal, sorted by name.
type Payload = Vec<(String, PayloadLit)>;

fn payload_lits(rec: Option<&RecordExpr>) -> Option<Payload> {
    let fs = match rec {
        None => return Some(Vec::new()),
        Some(RecordExpr::Fields(fs, _)) => fs,
        Some(_) => return None,
    };
    let mut out = Vec::new();
    for (n, e) in fs {
        let lit = match &e.kind {
            ExprKind::Lit(l) => PayloadLit::Lit(false, l.clone()),
            ExprKind::Unary(UnOp::Neg, x) => match &x.kind {
                ExprKind::Lit(l) => PayloadLit::Lit(true, l.clone()),
                _ => return None,
            },
            ExprKind::Ctor {
                path,
                ctor,
                record: None,
            } => PayloadLit::Ctor(
                path.module.as_ref().map(|m| m.text.clone()),
                path.name.text.clone(),
                ctor.as_ref().map(|c| c.text.clone()),
            ),
            _ => return None,
        };
        out.push((n.text.clone(), lit));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Some(out)
}

impl Checker<'_> {
    fn change(&mut self, kind: Kind) -> Tm {
        self.add(kind, Ty::Change)
    }

    fn singleton(&mut self, x: Tm, elem: &Ty) -> Tm {
        self.add(Kind::Singleton(x), Ty::Set(Box::new(elem.clone())))
    }

    /// Checks `e` with nested inserts in it allowed when `ok` and nothing forbids them here.
    fn write_expr(&mut self, e: &Expr, expected: Option<&Ty>, ok: bool) -> R<Out> {
        once(e, ok && self.once)?;
        match expected {
            Some(t) => self.check(e, t),
            None => self.synth(e),
        }
    }

    /// Wraps `w` so the rows of `nested` are inserted beside it.
    fn par_nested(&mut self, nested: Vec<Nested>, mut w: Tm) -> Tm {
        for n in nested.into_iter().rev() {
            let row_ty = self.terms.var_info(n.var).ty.clone();
            let v = self.var_tm(n.var);
            let rows = self.singleton(v, &row_ty);
            let ins = self.change(Kind::Insert(n.rel, rows));
            let both = self.change(Kind::Par(vec![ins, w]));
            w = self.change(Kind::Let(n.var, n.row, both));
        }
        w
    }

    /// Wraps `k`, of type `ty`, so the rows of `nested` are inserted before it.
    fn block_nested(&mut self, nested: Vec<Nested>, mut k: Tm, ty: &Ty) -> Tm {
        for n in nested.into_iter().rev() {
            let row_ty = self.terms.var_info(n.var).ty.clone();
            let v = self.var_tm(n.var);
            let rows = self.singleton(v, &row_ty);
            let ins = self.change(Kind::Insert(n.rel, rows));
            let blk = self.add(Kind::Block(ins, k), ty.clone());
            k = self.add(Kind::Let(n.var, n.row, blk), ty.clone());
        }
        k
    }

    /// Checks a write and elaborates it to a term of type `Change`.
    pub(super) fn write(&mut self, w: &Write) -> R<Tm> {
        Ok(self.write_effect(w)?.0)
    }

    fn write_effect(&mut self, w: &Write) -> R<(Tm, Effect)> {
        self.hoist.push(Vec::new());
        let r = self.write_form(w);
        let nested = self.hoist.pop().expect("pushed");
        let (tm, eff) = r?;
        Ok((self.par_nested(nested, tm), eff))
    }

    fn write_form(&mut self, w: &Write) -> R<(Tm, Effect)> {
        match &w.kind {
            WriteKind::Insert {
                target,
                from,
                source,
                upsert: None,
                rejects,
            } => {
                let p = self.proposed(target, *from, source, w.span)?;
                let rows = self.rows_of(&p);
                let ins = self.change(Kind::Insert(p.rel, rows));
                let tm = match rejects {
                    Some(j) => self.rejects(j, ins, source, *from, w.span)?,
                    None => ins,
                };
                Ok((tm, Effect::done()))
            }
            WriteKind::Insert {
                target,
                from,
                source,
                upsert: Some(u),
                rejects,
            } => {
                let (tm, _, eff, _) = self.upsert(target, *from, source, u, w.span)?;
                let tm = match rejects {
                    Some(j) => self.rejects(j, tm, source, *from, w.span)?,
                    None => tm,
                };
                Ok((tm, eff))
            }
            WriteKind::Update {
                binder,
                source,
                filter,
                value,
            } => {
                let (rel, rows, f, new_ty) =
                    self.update(binder, source.as_ref(), filter.as_ref(), value)?;
                let tm = self.change(Kind::Replace(rel, rows, f));
                Ok((tm, Effect::wrote(rel, new_ty)))
            }
            WriteKind::Delete {
                binder,
                source,
                filter,
            } => {
                let (rel, rows) = self.delete(binder, source.as_ref(), filter.as_ref())?;
                let tm = self.change(Kind::Delete(rel, rows));
                Ok((tm, Effect::deleted(rel)))
            }
            WriteKind::Fail(e) => {
                let o = self.write_expr(e, Some(&Ty::Base(Base::Text)), true)?;
                Ok((self.change(Kind::Fail(o.tm)), Effect::fail()))
            }
            WriteKind::Skip => Ok((self.change(Kind::Skip), Effect::done())),
            WriteKind::Seq(ws) if ws.len() == 1 => self.write_form(&ws[0]),
            WriteKind::Seq(ws) => {
                let mut tms = Vec::new();
                let mut eff = Effect::done();
                for x in ws {
                    let (t, e) = self.write_effect(x)?;
                    tms.push(t);
                    eff = eff.and(e);
                }
                Ok((self.change(Kind::Par(tms)), eff))
            }
            WriteKind::If(c, a, b) => {
                let bt = self.bool_ty();
                let c = self.write_expr(c, Some(&bt), true)?;
                let (a, ea) = self.write_effect(a)?;
                let (b, eb) = self.write_effect(b)?;
                Ok((self.change(Kind::If(c.tm, a, b)), ea.either(eb)))
            }
            WriteKind::Match(s, arms) => self.write_match(s, arms, w.span),
        }
    }

    fn rows_of(&mut self, p: &Proposed) -> Tm {
        if p.single {
            self.singleton(p.value, &p.row_ty)
        } else {
            p.value
        }
    }

    /// Resolves an insert's target to its relation and row type.
    fn target(&mut self, target: &ast::RelationRef) -> R<(RelId, Ty, usize)> {
        let rel = match self.lookup_value(&target.relation) {
            Ok(ValueDef::Relation(r)) => r,
            Ok(_) => {
                return Err(diag(
                    target.span,
                    format!("`{}` is not a relation", target.relation.name.text),
                ));
            }
            Err(e) => return Err(diag(target.span, e)),
        };
        let r = self.schema.relation(rel);
        let def = r.elem_def;
        let ctors = match &target.sel {
            Some(sel) => {
                let set = self
                    .schema
                    .relation_sel(rel, sel)
                    .map_err(|e| diag(target.span, e))?;
                self.schema.normalize_ctors(def, set)
            }
            None => None,
        };
        let n = match &ctors {
            Some(cs) => cs.len(),
            None => self.schema.ty(def).ctors().len(),
        };
        let row_ty = row_ty(&r.elem, ctors);
        Ok((rel, row_ty, n))
    }

    /// Checks an insert's rows, filling what the relation supplies.
    fn proposed(
        &mut self,
        target: &ast::RelationRef,
        from: bool,
        source: &Expr,
        span: Span,
    ) -> R<Proposed> {
        let (rel, row_ty, n) = self.target(target)?;
        let site = Site { rel, old: None };
        if !from {
            if n > 1 {
                return Err(diag(
                    span,
                    format!(
                        "a single row is inserted into one constructor of {}: name it, or insert constructor values with `from (...)`",
                        self.show(&row_ty)
                    ),
                ));
            }
            once(source, self.once)?;
            let row = self.new_value(source, &row_ty, &site)?;
            return Ok(Proposed {
                rel,
                row_ty,
                value: row.tm,
                single: true,
            });
        }
        once(source, self.once)?;
        let reject = std::mem::replace(&mut self.reject_ok, true);
        let r = (|| -> R<Tm> {
            let set_ty = Ty::Set(Box::new(row_ty.clone()));
            match &source.kind {
                ExprKind::Comprehension(c) => {
                    if c.order.is_some() {
                        return Err(diag(
                            source.span,
                            "an insert's rows are a set; `order by` orders only a result",
                        ));
                    }
                    let body = self.comprehension_body_with(c, &mut |this, e| {
                        this.new_value(e, &row_ty, &site)
                    })?;
                    Ok(self.finish(body, Some(&row_ty)).tm)
                }
                _ => Ok(self.check(source, &set_ty)?.tm),
            }
        })();
        self.reject_ok = reject;
        Ok(Proposed {
            rel,
            row_ty: row_ty.clone(),
            value: r?,
            single: false,
        })
    }

    /// Checks `e` as the new value of a row at `site`, where a constructor may omit the fields the row or the relation supplies.
    pub(super) fn new_value(&mut self, e: &Expr, expected: &Ty, site: &Site) -> R<Out> {
        match &e.kind {
            ExprKind::Ctor { path, ctor, record } => {
                let o = self.ctor(
                    path,
                    ctor.as_ref(),
                    record.as_ref(),
                    Some(expected),
                    e.span,
                    Some(site),
                )?;
                self.subsume(o, expected, e.span)
            }
            ExprKind::Record(r) => {
                let o = self.record(r, Some(expected), e.span, Some(site))?;
                self.subsume(o, expected, e.span)
            }
            ExprKind::Match(Some(s), arms) => {
                let heads: Vec<(&[Pattern], bool)> = arms
                    .iter()
                    .map(|a| (a.patterns.as_slice(), a.guard.is_some()))
                    .collect();
                let mut m = self.match_patterns(s, &heads, e.span)?;
                let mut joined: Option<Ty> = None;
                for (arm, ir) in arms.iter().zip(m.arms.iter_mut()) {
                    self.scope.push();
                    for (name, local) in &ir.locals {
                        self.scope.bind(name, local.clone());
                    }
                    let r = (|| -> R<(Option<Tm>, Out)> {
                        let guard = match &arm.guard {
                            Some(g) => {
                                let bt = self.bool_ty();
                                Some(self.check(g, &bt)?.tm)
                            }
                            None => None,
                        };
                        Ok((guard, self.new_value(&arm.body, expected, site)?))
                    })();
                    self.scope.pop();
                    let (guard, body) = r?;
                    joined = Some(match joined {
                        None => body.ty.clone(),
                        Some(j) => self.join(&j, &body.ty).unwrap_or_else(|| expected.clone()),
                    });
                    ir.guard = guard;
                    ir.body = Some(body);
                }
                let ty = joined.expect("a match has an arm");
                Ok(self.match_compile(m, ty))
            }
            _ => self.check(e, expected),
        }
    }

    /// Refuses a key field given in the new value of an existing row.
    pub(super) fn site_given(&self, site: &Site, fs: &[(ast::Ident, Expr)]) -> R<()> {
        if site.old.is_none() {
            return Ok(());
        }
        let r = self.schema.relation(site.rel);
        match fs.iter().find(|(n, _)| r.key.contains(&n.text)) {
            Some((n, _)) => Err(diag(
                n.span,
                format!(
                    "the new value keeps the row's key: `{}` is a key field of `{}` and is not given",
                    n.text, r.name
                ),
            )),
            None => Ok(()),
        }
    }

    /// The value of field `f` of constructor `c` that a constructor at `site` omits, if the row or the relation supplies one.
    pub(super) fn fill(
        &mut self,
        site: &Site,
        c: u32,
        f: &str,
        header: bool,
        fty: Ty,
    ) -> R<Option<Tm>> {
        let r = self.schema.relation(site.rel);
        let is_key = r.key.iter().any(|k| k == f);
        if let Some(old) = site.old
            && (header || is_key)
        {
            return Ok(Some(self.add(Kind::Field(old, f.to_string()), fty)));
        }
        let default = r
            .defaults
            .iter()
            .find(|d| d.field == f && d.ctor == Some(c))
            .or_else(|| r.defaults.iter().find(|d| d.field == f && d.ctor.is_none()))
            .map(|d| d.value.clone());
        let generated = r.generate.filter(|_| is_key && r.key.len() == 1);
        match (default, generated) {
            (Some(DefaultValue::Generate(g)), _) | (None, Some(g)) => {
                Ok(Some(self.add(Kind::Fresh(g, site.rel), fty)))
            }
            (Some(DefaultValue::Expr(e)), _) => Ok(Some(self.default_expr(site.rel, &e, &fty)?)),
            (None, None) => Ok(None),
        }
    }

    /// Checks a relation's default expression where it was declared.
    fn default_expr(&mut self, rel: RelId, e: &Expr, fty: &Ty) -> R<Tm> {
        let module = self.schema.relation(rel).module;
        let vis = std::mem::replace(&mut self.vis, Visible::module(module));
        let scope = std::mem::take(&mut self.scope);
        let ctx = std::mem::replace(&mut self.ctx, Context::Schema);
        let tparams = std::mem::take(&mut self.tparams);
        let reject = std::mem::replace(&mut self.reject_ok, false);
        let r = self.check(e, fty);
        self.vis = vis;
        self.scope = scope;
        self.ctx = ctx;
        self.tparams = tparams;
        self.reject_ok = reject;
        Ok(r?.tm)
    }

    /// `key (insert ...)`: the key of the row a nested single-row insert inserts beside its enclosing write.
    pub(super) fn nested_insert(&mut self, w: &Write) -> R<Out> {
        if self.hoist.is_empty() {
            return Err(diag(
                w.span,
                "a write appears only where a change set is expected: in a transact block, a script or ad hoc text",
            ));
        }
        let inner = ungroup(w);
        let WriteKind::Insert {
            target,
            from: false,
            source,
            upsert: None,
            rejects: None,
        } = &inner.kind
        else {
            return Err(diag(
                w.span,
                "`key (...)` of a write takes a single-row insert: `key (insert r { ... })`",
            ));
        };
        let p = self.proposed(target, false, source, inner.span)?;
        let var = self.terms.var("ξ", p.row_ty.clone());
        self.proposed.insert(var);
        self.hoist.last_mut().expect("a frame").push(Nested {
            var,
            rel: p.rel,
            row: p.value,
        });
        let tm = self.var_tm(var);
        self.key_of(
            Out {
                tm,
                ty: p.row_ty,
                rows: Some(p.rel),
            },
            w.span,
        )
    }

    /// Wraps an insert or upsert of rows from a single-generator comprehension so its failing records go to `j`.
    fn rejects(
        &mut self,
        j: &ast::QName,
        insert: Tm,
        source: &Expr,
        from: bool,
        span: Span,
    ) -> R<Tm> {
        let one_generator = match &source.kind {
            ExprKind::Comprehension(c) if from && c.group.is_none() => {
                let gens: usize = c
                    .clauses
                    .iter()
                    .map(|cl| match cl {
                        ast::Clause::From(gs) => gs.len(),
                        ast::Clause::Where(_) => 0,
                    })
                    .sum();
                gens == 1 && matches!(c.clauses.first(), Some(ast::Clause::From(_)))
            }
            _ => false,
        };
        let rows = match self.terms.kind(insert) {
            Kind::Insert(_, rows) | Kind::Upsert { rows, .. } => *rows,
            _ => unreachable!("rejects wraps an insert"),
        };
        let over_source = match self.terms.kind(rows) {
            Kind::For(_, src, _) => matches!(self.terms.kind(*src), Kind::Rel(..)),
            _ => false,
        };
        if !one_generator || !over_source {
            return Err(diag(
                span,
                "`rejects into` needs rows from a comprehension with a single generator over a source relation, so each row has one source record",
            ));
        }
        let into = match self.lookup_value(j) {
            Ok(ValueDef::Relation(r)) => r,
            Ok(_) => {
                return Err(diag(
                    j.span(),
                    format!("`{}` is not a relation", j.name.text),
                ));
            }
            Err(e) => return Err(diag(j.span(), e)),
        };
        if self.schema.relation(into).elem_def != self.schema.prelude_type("RejectedRow") {
            return Err(diag(
                j.span(),
                format!(
                    "`rejects into` names a relation of `RejectedRow`; `{}` holds {}",
                    j.name.text,
                    self.show(&self.schema.relation(into).elem)
                ),
            ));
        }
        Ok(self.change(Kind::Rejects { into, insert }))
    }

    /// Checks an upsert; returns its write, its binding's type or why `let` cannot bind it, its effect and its relation.
    fn upsert(
        &mut self,
        target: &ast::RelationRef,
        from: bool,
        source: &Expr,
        u: &ast::Upsert,
        span: Span,
    ) -> R<(Tm, R<Ty>, Effect, RelId)> {
        let p = self.proposed(target, from, source, span)?;
        let r = self.schema.relation(p.rel);
        let def = r.elem_def;
        let (on, scope) = match &u.on {
            ConflictTarget::Key => (Conflict::Key, None),
            ConflictTarget::Fields(es, sc) => {
                let want = match sc {
                    None => None,
                    Some(s) => {
                        let sel = match s {
                            CtorScope::One(c) => ast::CtorSel::One(c.clone()),
                            CtorScope::Many(cs) => ast::CtorSel::Many(cs.clone()),
                        };
                        let set = self
                            .schema
                            .relation_sel(p.rel, &sel)
                            .map_err(|e| diag(span, e))?;
                        self.schema.normalize_ctors(def, set)
                    }
                };
                let found = r.indexes.iter().position(|ix| {
                    ix.unique
                        && ix
                            .scope
                            .clone()
                            .and_then(|s| self.schema.normalize_ctors(def, s))
                            == want
                        && ix.fields.len() == es.len()
                        && ix.fields.iter().zip(es).all(|(a, b)| same_expr(a, b))
                });
                match found {
                    Some(i) => (Conflict::Unique(i as u32), want),
                    None => {
                        return Err(diag(
                            span,
                            format!(
                                "`unless conflict on` names no key or unique declared on `{}`",
                                r.name
                            ),
                        ));
                    }
                }
            }
        };
        let target_set = Self::ctors_of(&p.row_ty);
        if let Some(scope) = &scope {
            let within = match &target_set {
                Some(t) => t.iter().all(|c| scope.contains(c)),
                None => false,
            };
            if !within {
                return Err(diag(
                    span,
                    format!(
                        "the insert's constructors of {} lie outside the constructors the unique ranges over",
                        self.show(&p.row_ty)
                    ),
                ));
            }
        }
        let old_ty = row_ty(&r.elem, scope);
        let rows = self.rows_of(&p);
        self.scope.push();
        let old = self.bind("old", old_ty.clone(), Some(p.rel));
        self.snapshot.insert(old);
        let new = self.bind("new", p.row_ty.clone(), Some(p.rel));
        self.proposed.insert(new);
        let saved = std::mem::replace(&mut self.once, false);
        let r = self.write_effect(&u.otherwise);
        self.once = saved;
        self.scope.pop();
        let (otherwise, eff) = r?;
        let binding = self.upsert_binding(&eff, p.rel, &p.row_ty, &old_ty, span);
        let binding = binding.map(|t| if p.single { t } else { Ty::Set(Box::new(t)) });
        let tm = self.change(Kind::Upsert {
            rel: p.rel,
            rows,
            on,
            old,
            new,
            otherwise,
            single: p.single,
        });
        Ok((tm, binding, Effect::done().either(eff), p.rel))
    }

    /// The type of the row an upsert binds: the target's row type joined with the new values its else branch writes into the relation.
    fn upsert_binding(
        &self,
        eff: &Effect,
        rel: RelId,
        row_ty: &Ty,
        old_ty: &Ty,
        span: Span,
    ) -> R<Ty> {
        if self.cascade_closure(&eff.deletes).contains(&rel) {
            return Err(diag(
                span,
                "a `let` on an upsert binds exactly one row, and this else branch can delete it",
            ));
        }
        let mut joined = row_ty.clone();
        let mut keeps = false;
        for o in &eff.outcomes {
            let Outcome::Done(ws) = o else {
                continue;
            };
            let mine: Vec<&Ty> = ws
                .iter()
                .filter(|(r, _)| *r == rel)
                .map(|(_, t)| t)
                .collect();
            if mine.is_empty() {
                keeps = true;
            }
            for t in mine {
                joined = self.join(&joined, t).ok_or_else(|| {
                    diag(
                        span,
                        format!(
                            "the upsert's rows have no common type: {} and {}",
                            self.show(&joined),
                            self.show(t)
                        ),
                    )
                })?;
            }
        }
        if keeps {
            joined = self.join(&joined, old_ty).ok_or_else(|| {
                diag(
                    span,
                    format!(
                        "the upsert's rows have no common type: {} and {}",
                        self.show(&joined),
                        self.show(old_ty)
                    ),
                )
            })?;
        }
        Ok(joined)
    }

    /// The relations deleting from `rels` can delete rows of, through `cascade` actions included.
    fn cascade_closure(&self, rels: &[RelId]) -> BTreeSet<RelId> {
        let mut out: BTreeSet<RelId> = rels.iter().copied().collect();
        loop {
            let before = out.len();
            for (i, r) in self.schema.relations.iter().enumerate() {
                if r.elem_def.0 == u32::MAX {
                    continue;
                }
                let t = self.schema.ty(r.elem_def);
                let cascades = t
                    .header()
                    .iter()
                    .chain(t.ctors().iter().flat_map(|c| c.fields.iter()))
                    .any(|f| {
                        f.on_delete == OnDelete::Cascade
                            && matches!(f.ty, Ty::Key { rel, .. } if out.contains(&rel))
                    });
                if cascades {
                    out.insert(RelId(i as u32));
                }
            }
            if out.len() == before {
                return out;
            }
        }
    }

    /// Returns whether `t` reads a variable bound to rows a write proposes.
    fn mentions_proposed(&self, t: Tm) -> bool {
        match self.terms.kind(t) {
            Kind::Var(v) if self.proposed.contains(v) => true,
            _ => self
                .terms
                .children(t)
                .into_iter()
                .any(|c| self.mentions_proposed(c)),
        }
    }

    /// Returns whether `o` is a row of a relation read from the statement's snapshot.
    fn snapshot_value(&self, o: &Out) -> bool {
        o.rows.is_some() && matches!(o.ty, Ty::Data { .. }) && !self.mentions_proposed(o.tm)
    }

    /// The rows of `R` an `update ... in` or `delete ... in` ranges over.
    fn ranged(&mut self, source: &Expr, what: &str) -> R<(RelId, Out, Ty)> {
        once(source, self.once)?;
        let o = self.synth(source)?;
        if self.mentions_proposed(o.tm) {
            return Err(diag(
                source.span,
                format!(
                    "`{what} x in R` ranges over rows of the snapshot, and these are rows a write proposes"
                ),
            ));
        }
        let elem = self.elem_ty(&o.ty);
        match (o.rows, elem) {
            (Some(rel), Some(elem @ Ty::Data { .. })) => Ok((rel, o, elem)),
            _ => Err(diag(
                source.span,
                format!(
                    "`{what} x in R` ranges over rows of one relation variable, and {} holds none",
                    self.show(&o.ty)
                ),
            )),
        }
    }

    /// The snapshot row `binder` names, for `update x` and `delete x`.
    fn snapshot_row(&mut self, binder: &ast::Ident, what: &str) -> R<(RelId, Local)> {
        let local = self.scope.lookup(&binder.text).cloned();
        if local.is_none() && !self.params.contains_key(&binder.text) {
            return Err(diag(
                binder.span,
                format!("`{}` is not in scope", binder.text),
            ));
        }
        match local {
            Some(
                local @ Local {
                    rows: Some(rel), ..
                },
            ) if self.snapshot.contains(&local.var) && matches!(local.ty, Ty::Data { .. }) => {
                Ok((rel, local))
            }
            _ => Err(diag(
                binder.span,
                format!(
                    "`{what} {}` needs `{}` bound to a row of the snapshot, by a generator, an upsert's `old` or a match over one; `{}` is not a row of the snapshot",
                    binder.text, binder.text, binder.text
                ),
            )),
        }
    }

    /// Checks an update; returns its relation, old rows, new-value function and new values' type.
    fn update(
        &mut self,
        binder: &ast::Ident,
        source: Option<&Expr>,
        filter: Option<&Expr>,
        value: &Expr,
    ) -> R<(RelId, Tm, Tm, Ty)> {
        let (rel, rows, elem, x) = match source {
            Some(src) => {
                let (rel, o, elem) = self.ranged(src, "update")?;
                self.scope.push();
                let x = self.bind(&binder.text, elem.clone(), Some(rel));
                self.snapshot.insert(x);
                let rows = match filter {
                    Some(p) => {
                        let r = self.row_filter(x, o.tm, &elem, p);
                        if r.is_err() {
                            self.scope.pop();
                        }
                        r?
                    }
                    None => o.tm,
                };
                (rel, rows, elem, x)
            }
            None => {
                let (rel, local) = self.snapshot_row(binder, "update")?;
                let xt = self.var_tm(local.var);
                let mut rows = self.singleton(xt, &local.ty);
                self.scope.push();
                let x = self.bind(&binder.text, local.ty.clone(), Some(rel));
                self.snapshot.insert(x);
                if let Some(p) = filter {
                    let bt = self.bool_ty();
                    let p = self.write_expr(p, Some(&bt), true);
                    if p.is_err() {
                        self.scope.pop();
                    }
                    let set_ty = self.terms.ty(rows).clone();
                    let none = self.add(Kind::EmptySet, set_ty.clone());
                    rows = self.add(Kind::If(p?.tm, rows, none), set_ty);
                }
                (rel, rows, local.ty, x)
            }
        };
        let whole = self.schema.relation(rel).elem.clone();
        let old = self.var_tm(x);
        let site = Site {
            rel,
            old: Some(old),
        };
        let r = (|| -> R<Out> {
            once(value, source.is_none() && self.once)?;
            self.new_value(value, &whole, &site)
        })();
        self.scope.pop();
        let new = r?;
        let f = self.add(
            Kind::Lam(x, new.tm),
            Ty::Fn(Box::new(elem), Box::new(whole)),
        );
        Ok((rel, rows, f, new.ty))
    }

    /// `for x in rows do if p then {x} else {}`, with `x` in scope.
    fn row_filter(&mut self, x: Var, rows: Tm, elem: &Ty, p: &Expr) -> R<Tm> {
        forbid_nested(p)?;
        let bt = self.bool_ty();
        let p = self.check(p, &bt)?;
        let set_ty = Ty::Set(Box::new(elem.clone()));
        let xt = self.var_tm(x);
        let one = self.singleton(xt, elem);
        let none = self.add(Kind::EmptySet, set_ty.clone());
        let body = self.add(Kind::If(p.tm, one, none), set_ty.clone());
        Ok(self.add(Kind::For(x, rows, body), set_ty))
    }

    /// Checks a delete; returns its relation and the rows it names.
    fn delete(
        &mut self,
        binder: &ast::Ident,
        source: Option<&Expr>,
        filter: Option<&Expr>,
    ) -> R<(RelId, Tm)> {
        match source {
            Some(src) => {
                let (rel, o, elem) = self.ranged(src, "delete")?;
                let rows = match filter {
                    Some(p) => {
                        self.scope.push();
                        let x = self.bind(&binder.text, elem.clone(), Some(rel));
                        self.snapshot.insert(x);
                        let r = self.row_filter(x, o.tm, &elem, p);
                        self.scope.pop();
                        r?
                    }
                    None => o.tm,
                };
                Ok((rel, rows))
            }
            None => {
                let (rel, local) = self.snapshot_row(binder, "delete")?;
                let xt = self.var_tm(local.var);
                let mut rows = self.singleton(xt, &local.ty);
                if let Some(p) = filter {
                    let bt = self.bool_ty();
                    let p = self.write_expr(p, Some(&bt), true)?;
                    let set_ty = self.terms.ty(rows).clone();
                    let none = self.add(Kind::EmptySet, set_ty.clone());
                    rows = self.add(Kind::If(p.tm, rows, none), set_ty);
                }
                Ok((rel, rows))
            }
        }
    }

    fn write_match(&mut self, s: &Expr, arms: &[WriteArm], span: Span) -> R<(Tm, Effect)> {
        once(s, self.once)?;
        let heads: Vec<(&[Pattern], bool)> = arms
            .iter()
            .map(|a| (a.patterns.as_slice(), a.guard.is_some()))
            .collect();
        let mut m = self.match_patterns(s, &heads, span)?;
        let over_snapshot = self.snapshot_value(&m.scrutinee);
        let mut eff: Option<Effect> = None;
        for (arm, ir) in arms.iter().zip(m.arms.iter_mut()) {
            self.scope.push();
            for (name, local) in &ir.locals {
                if over_snapshot && local.rows.is_some() {
                    self.snapshot.insert(local.var);
                }
                self.scope.bind(name, local.clone());
            }
            let saved = self.once;
            if !ir.locals.is_empty() {
                self.once = false;
            }
            let r = (|| -> R<(Option<Tm>, Tm, Effect)> {
                let guard = match &arm.guard {
                    Some(g) => {
                        let bt = self.bool_ty();
                        Some(self.write_expr(g, Some(&bt), false)?.tm)
                    }
                    None => None,
                };
                let (tm, e) = self.write_effect(&arm.body)?;
                Ok((guard, tm, e))
            })();
            self.once = saved;
            self.scope.pop();
            let (guard, tm, e) = r?;
            eff = Some(match eff {
                None => e,
                Some(x) => x.either(e),
            });
            ir.guard = guard;
            ir.body = Some(Out {
                tm,
                ty: Ty::Change,
                rows: None,
            });
        }
        let out = self.match_compile(m, Ty::Change);
        Ok((out.tm, eff.expect("a match has an arm")))
    }

    /// Checks `let name = w` in a block, then the items after it.
    fn let_write(
        &mut self,
        name: &ast::Ident,
        w: &Write,
        rest: &[BlockItem],
        b: &TransactBlock,
        result: Option<&Ty>,
    ) -> R<Out> {
        let w = ungroup(w);
        self.hoist.push(Vec::new());
        let parts = self.let_parts(w);
        let nested = self.hoist.pop().expect("pushed");
        let (kind, ty, rel) = parts?;
        self.scope.push();
        let v = self.bind(&name.text, ty.clone(), Some(rel));
        self.proposed.insert(v);
        let k = self.items(rest, b, result);
        self.scope.pop();
        let k = k?;
        let body = match kind {
            LetKind::Insert { rel, value, single } => {
                let vt = self.var_tm(v);
                let rows = if single { self.singleton(vt, &ty) } else { vt };
                let ins = self.change(Kind::Insert(rel, rows));
                let blk = self.add(Kind::Block(ins, k.tm), k.ty.clone());
                self.add(Kind::Let(v, value, blk), k.ty.clone())
            }
            LetKind::Delete { rel, rows } => {
                let vt = self.var_tm(v);
                let del = self.change(Kind::Delete(rel, vt));
                let blk = self.add(Kind::Block(del, k.tm), k.ty.clone());
                self.add(Kind::Let(v, rows, blk), k.ty.clone())
            }
            LetKind::Bang(write) => self.add(Kind::LetBang(v, write, k.tm), k.ty.clone()),
        };
        let tm = self.block_nested(nested, body, &k.ty);
        Ok(Out {
            tm,
            ty: k.ty,
            rows: k.rows,
        })
    }

    /// Checks a write bound by `let`: how it binds, the bound type and relation.
    fn let_parts(&mut self, w: &Write) -> R<(LetKind, Ty, RelId)> {
        match &w.kind {
            WriteKind::Insert {
                target,
                from,
                source,
                upsert: None,
                rejects: None,
            } => {
                let p = self.proposed(target, *from, source, w.span)?;
                let ty = if p.single {
                    p.row_ty.clone()
                } else {
                    Ty::Set(Box::new(p.row_ty.clone()))
                };
                Ok((
                    LetKind::Insert {
                        rel: p.rel,
                        value: p.value,
                        single: p.single,
                    },
                    ty,
                    p.rel,
                ))
            }
            WriteKind::Insert {
                target,
                from,
                source,
                upsert: None,
                rejects: Some(j),
            } => {
                let p = self.proposed(target, *from, source, w.span)?;
                let rows = self.rows_of(&p);
                let ins = self.change(Kind::Insert(p.rel, rows));
                let tm = self.rejects(j, ins, source, *from, w.span)?;
                Ok((LetKind::Bang(tm), Ty::Set(Box::new(p.row_ty)), p.rel))
            }
            WriteKind::Insert {
                target,
                from,
                source,
                upsert: Some(u),
                rejects,
            } => {
                let (tm, ty, _, rel) = self.upsert(target, *from, source, u, w.span)?;
                let ty = ty?;
                let tm = match rejects {
                    Some(j) => self.rejects(j, tm, source, *from, w.span)?,
                    None => tm,
                };
                Ok((LetKind::Bang(tm), ty, rel))
            }
            WriteKind::Update {
                binder,
                source,
                filter,
                value,
            } => {
                let (rel, rows, f, new_ty) =
                    self.update(binder, source.as_ref(), filter.as_ref(), value)?;
                let tm = self.change(Kind::Replace(rel, rows, f));
                Ok((LetKind::Bang(tm), Ty::Set(Box::new(new_ty)), rel))
            }
            WriteKind::Delete {
                binder,
                source,
                filter,
            } => {
                let (rel, rows) = self.delete(binder, source.as_ref(), filter.as_ref())?;
                let elem = self
                    .elem_ty(self.terms.ty(rows))
                    .expect("deleted rows are a set");
                Ok((LetKind::Delete { rel, rows }, Ty::Set(Box::new(elem)), rel))
            }
            _ => Err(diag(
                w.span,
                "`let` binds the rows of an insert, an upsert, an update or a delete; `skip`, `fail`, a write sequence, `if` or `match` binds none",
            )),
        }
    }

    /// Checks a `transact` block whose result, when given, is `result`.
    pub(super) fn transact(&mut self, b: &TransactBlock, result: Option<&Ty>) -> R<Out> {
        let mut opts = TxnOptions::default();
        let mut durability = false;
        for o in &b.options {
            let twice = match o {
                TxnOption::Retry(n) => opts.retry.replace(*n).is_some(),
                TxnOption::ReadOnly { deferrable } => {
                    let twice = opts.read_only;
                    opts.read_only = true;
                    opts.deferrable = *deferrable;
                    twice
                }
                TxnOption::Durability { quorum } => {
                    let level = if *quorum {
                        Durability::Quorum
                    } else {
                        Durability::Local
                    };
                    durability = true;
                    opts.durability.replace(level).is_some()
                }
            };
            if twice {
                return Err(diag(b.span, "a `transact` option is given twice"));
            }
        }
        if opts.read_only && durability {
            return Err(diag(
                b.span,
                "a `read_only` block commits no writes, so it takes no `durability`",
            ));
        }
        let read_only = std::mem::replace(&mut self.read_only, opts.read_only);
        self.scope.push();
        let r = self.items(&b.items, b, result);
        self.scope.pop();
        self.read_only = read_only;
        let body = r?;
        let tm = self.add(Kind::Transact(body.tm, opts), body.ty.clone());
        Ok(Out {
            tm,
            ty: body.ty,
            rows: body.rows,
        })
    }

    fn no_writes(&self, span: Span) -> R<()> {
        if self.read_only {
            return Err(diag(span, "a `read_only` block may not write"));
        }
        Ok(())
    }

    /// Checks the items of a block from `items` on, then its `return`.
    fn items(&mut self, items: &[BlockItem], b: &TransactBlock, result: Option<&Ty>) -> R<Out> {
        let Some((first, rest)) = items.split_first() else {
            let Some(e) = &b.ret else {
                if let Some(t) = result
                    && *t != Ty::Unit
                {
                    return Err(diag(
                        b.span,
                        format!(
                            "the block returns nothing, and its result is {}",
                            self.show(t)
                        ),
                    ));
                }
                return Ok(self.out(Kind::Record(Vec::new()), Ty::Unit));
            };
            self.hoist.push(Vec::new());
            let r = self.write_expr(e, result, true);
            let nested = self.hoist.pop().expect("pushed");
            let o = r?;
            if !nested.is_empty() {
                self.no_writes(e.span)?;
            }
            let ty = result.cloned().unwrap_or_else(|| o.ty.clone());
            let tm = self.block_nested(nested, o.tm, &ty);
            return Ok(Out {
                tm,
                ty,
                rows: o.rows,
            });
        };
        match first {
            BlockItem::Write(w) => {
                self.no_writes(w.span)?;
                let tm = self.write(w)?;
                let k = self.items(rest, b, result)?;
                let tm = self.add(Kind::Block(tm, k.tm), k.ty.clone());
                Ok(Out { tm, ..k })
            }
            BlockItem::Let(name, LetValue::Write(w)) => {
                self.no_writes(w.span)?;
                self.let_write(name, w, rest, b, result)
            }
            BlockItem::Let(name, LetValue::Expr(e)) => {
                self.hoist.push(Vec::new());
                let r = self.write_expr(e, None, true);
                let nested = self.hoist.pop().expect("pushed");
                let o = r?;
                if !nested.is_empty() {
                    self.no_writes(e.span)?;
                }
                self.scope.push();
                let v = self.bind(&name.text, o.ty.clone(), o.rows);
                if self.snapshot_value(&o) {
                    self.snapshot.insert(v);
                }
                let k = self.items(rest, b, result);
                self.scope.pop();
                let k = k?;
                let tm = self.add(Kind::Let(v, o.tm, k.tm), k.ty.clone());
                let tm = self.block_nested(nested, tm, &k.ty);
                Ok(Out { tm, ..k })
            }
        }
    }

    /// Each relation's `on delete retag` new value for a referrer, a `Lam` over the referrer's row.
    pub(super) fn retag_action(
        &mut self,
        rel: RelId,
        c: u32,
        rec: Option<&RecordExpr>,
        span: Span,
    ) -> R<Tm> {
        let elem = self.schema.relation(rel).elem.clone();
        let Ty::Data { def, args, .. } = &elem else {
            unreachable!("a relation holds a declared sum")
        };
        self.scope.push();
        let x = self.bind("self", elem.clone(), Some(rel));
        let old = self.var_tm(x);
        let site = Site {
            rel,
            old: Some(old),
        };
        let empty = RecordExpr::Fields(Vec::new(), span);
        let r = self.build_ctor(
            expr::Target::Decl(*def, Some(args.clone())),
            c,
            Some(rec.unwrap_or(&empty)),
            span,
            Some(&site),
        );
        self.scope.pop();
        let new = r?;
        Ok(self.add(
            Kind::Lam(x, new.tm),
            Ty::Fn(Box::new(elem.clone()), Box::new(elem)),
        ))
    }
}

/// Checks the `on delete` actions of every declared type and elaborates each relation's `retag` action.
pub(super) fn delete_actions(
    schema: &Schema,
    terms: &mut Terms,
    diags: &mut Vec<FileDiag>,
) -> BTreeMap<RelId, Tm> {
    let mut out = BTreeMap::new();
    for (i, t) in schema.types.iter().enumerate() {
        let TypeDefKind::Sum { header, ctors, .. } = &t.kind else {
            continue;
        };
        let mut err = |span: Span, msg: String| {
            diags.push(FileDiag {
                file: t.file,
                diag: diag(span, msg),
            })
        };
        let mut first: Option<(u32, Payload, Option<RecordExpr>)> = None;
        let fields = header.iter().map(|f| (None, f)).chain(
            ctors
                .iter()
                .enumerate()
                .flat_map(|(c, k)| k.fields.iter().map(move |f| (Some(c as u32), f))),
        );
        let mut ok = true;
        for (owner, f) in fields {
            let OnDelete::Retag(c, rec) = &f.on_delete else {
                continue;
            };
            let target = &ctors[*c as usize].name;
            if owner.is_none() {
                err(
                    f.span,
                    format!(
                        "`on delete retag` is not allowed on header field `{}`, which every constructor has",
                        f.name
                    ),
                );
                ok = false;
                continue;
            }
            if t.field(*c, &f.name).is_some() {
                err(
                    f.span,
                    format!(
                        "`on delete retag {target}` needs a constructor without the field `{}`",
                        f.name
                    ),
                );
                ok = false;
                continue;
            }
            let Some(lits) = payload_lits(rec.as_ref()) else {
                err(
                    f.span,
                    "the payload of `on delete retag` is a record of literals".to_string(),
                );
                ok = false;
                continue;
            };
            match &first {
                None => first = Some((*c, lits, rec.clone())),
                Some((c0, l0, _)) if *c0 == *c && *l0 == lits => {}
                Some(_) => {
                    err(
                        f.span,
                        format!(
                            "the `on delete retag` actions of `{}` differ; every retag action of one type is the same action",
                            t.name
                        ),
                    );
                    ok = false;
                }
            }
        }
        let Some((c, _, rec)) = first.filter(|_| ok) else {
            continue;
        };
        for (ri, r) in schema.relations.iter().enumerate() {
            if r.elem_def != TypeId(i as u32) {
                continue;
            }
            let rel = RelId(ri as u32);
            let mut ch = Checker::new(schema, Visible::module(r.module), Context::Schema);
            ch.terms = std::mem::take(terms);
            let span = rec.as_ref().map_or(t.span, |r| match r {
                RecordExpr::Fields(_, s) | RecordExpr::With(_, _, s) | RecordExpr::Elided(s) => *s,
            });
            let res = ch.retag_action(rel, c, rec.as_ref(), span);
            *terms = std::mem::take(&mut ch.terms);
            match res {
                Ok(tm) => {
                    out.insert(rel, tm);
                }
                Err(d) => diags.push(FileDiag {
                    file: t.file,
                    diag: d,
                }),
            }
        }
    }
    out
}

/// A checked registered transaction: its term, a `Lam` over its parameters of a `Transact`.
#[derive(Debug)]
pub struct Txn {
    pub terms: Terms,
    pub term: Tm,
}

/// Type checks a registered transaction of a bundle that reads `module`, against `program`, the checked `schema`.
pub fn check_txn(
    schema: &Schema,
    program: &Program,
    module: ModuleId,
    t: &ast::TxnDecl,
) -> Result<Txn, Diag> {
    let mut c = Checker::new(schema, Visible::module(module), Context::Function);
    let term = c.txn(t)?;
    let txn = Txn {
        terms: c.terms,
        term,
    };
    crate::analysis::txn_diag(schema, program, &txn, t.span)?;
    Ok(txn)
}

impl Checker<'_> {
    fn txn(&mut self, t: &ast::TxnDecl) -> R<Tm> {
        if let Some(l) = &t.limit {
            self.check(l, &Ty::Base(Base::Duration))?;
        }
        let result = self.resolve_type(&t.result)?;
        if result.mentions_fn() {
            return Err(diag(
                t.result.span,
                "a registered transaction never returns a function",
            ));
        }
        self.scope.push();
        let mut vars = Vec::new();
        for p in &t.params {
            let pty = self.resolve_type(&p.ty)?;
            if pty.mentions_fn() {
                return Err(diag(
                    p.ty.span,
                    "a parameter of a registered transaction never has a function type",
                ));
            }
            vars.push(self.bind(&p.name.text, pty, None));
        }
        let body = self.transact(&t.body, Some(&result));
        self.scope.pop();
        let body = body?;
        Ok(self.lams(vars, body.tm, result))
    }
}
