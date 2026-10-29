use super::expr::{empty_union, wants_expected};
use super::*;
use crate::ast::{BinOp, Expr, ExprKind};
use crate::kernel::{Agg, Kind};

/// A clause of a comprehension, elaborated: a generator or a filter.
enum Frame {
    For(Var, Tm),
    Where(Tm),
}

/// A comprehension elaborated up to its `select`, with the scope of the `select` still open.
pub(super) struct Body {
    frames: Vec<Frame>,
    pub(super) select: Out,
    /// Variables bound by the comprehension's generators, which no ordering key may read except through the selected value.
    binders: Vec<Var>,
}

impl Checker<'_> {
    pub(super) fn at(
        &mut self,
        instant: bool,
        point: &Expr,
        pins: Option<&ast::Version>,
        body: &Expr,
    ) -> R<Out> {
        let pty = if instant {
            Ty::Base(Base::Instant)
        } else {
            Ty::Data {
                def: self.schema.prelude_type("Position"),
                args: Vec::new(),
                ctors: None,
            }
        };
        let p = self.check(point, &pty)?;
        let b = self.synth(body)?;
        if self.elem_ty(&b.ty).is_none() {
            return Err(diag(
                body.span,
                format!("an `at` block reads a set, found {}", self.show(&b.ty)),
            ));
        }
        let ty = b.ty.clone();
        Ok(self.out(
            Kind::At {
                instant,
                point: p.tm,
                pins: pins.map(|v| v.number),
                body: b.tm,
            },
            ty,
        ))
    }

    /// A generator's source, as a set and the type of its elements.
    fn generator_source(&mut self, e: &Expr) -> R<(Out, Ty)> {
        let o = self.synth(e)?;
        match self.elem_ty(&o.ty) {
            Some(t) => Ok((o, t)),
            None => Err(diag(
                e.span,
                format!("a generator ranges over a set, found {}", self.show(&o.ty)),
            )),
        }
    }

    fn clauses(
        &mut self,
        clauses: &[ast::Clause],
        frames: &mut Vec<Frame>,
        binders: &mut Vec<Var>,
    ) -> R<()> {
        for cl in clauses {
            match cl {
                ast::Clause::From(gens) => {
                    for g in gens {
                        let (src, elem) = self.generator_source(&g.source)?;
                        let v = self.bind_origin(&g.binder.text, elem, self.row_origin(&src));
                        frames.push(Frame::For(v, src.tm));
                        binders.push(v);
                    }
                }
                ast::Clause::Where(p) => {
                    let bt = self.bool_ty();
                    let o = self.check(p, &bt)?;
                    frames.push(Frame::Where(o.tm));
                }
            }
        }
        Ok(())
    }

    /// Wraps `inner`, a set, in the comprehension's generators and filters.
    fn assemble(&mut self, frames: Vec<Frame>, mut inner: Tm, set_ty: &Ty) -> Tm {
        for f in frames.into_iter().rev() {
            inner = match f {
                Frame::For(v, src) => self.add(Kind::For(v, src, inner), set_ty.clone()),
                Frame::Where(p) => {
                    let none = self.add(Kind::EmptySet, set_ty.clone());
                    self.add(Kind::If(p, inner, none), set_ty.clone())
                }
            };
        }
        inner
    }

    /// The record of `binders` a group or aggregate ranges over, labelled by name.
    fn binder_record(&mut self, binders: &[Var]) -> (Vec<(String, Var)>, Tm, Ty) {
        let mut labels: Vec<(String, Var)> = Vec::new();
        for &v in binders {
            let mut name = self.terms.var_info(v).name.clone();
            while labels.iter().any(|(l, _)| *l == name) {
                name.push('\'');
            }
            labels.push((name, v));
        }
        let mut fields: Vec<(String, Tm)> = Vec::new();
        let mut tys: Vec<(String, Ty)> = Vec::new();
        for (l, v) in &labels {
            let t = self.var_tm(*v);
            fields.push((l.clone(), t));
            tys.push((l.clone(), self.terms.var_info(*v).ty.clone()));
        }
        fields.sort_by(|a, b| a.0.cmp(&b.0));
        tys.sort_by(|a, b| a.0.cmp(&b.0));
        let ty = Ty::Record(tys);
        let tm = self.add(Kind::Record(fields), ty.clone());
        (labels, tm, ty)
    }

    /// Elaborates a comprehension's clauses and `select`, leaving the scope of the `select` open.
    pub(super) fn comprehension_body(
        &mut self,
        c: &ast::Comprehension,
        elem: Option<&Ty>,
    ) -> R<Body> {
        self.comprehension_body_with(c, &mut |this, e| match elem {
            Some(t) => this.check(e, t),
            None => this.synth(e),
        })
    }

    /// Elaborates a comprehension's clauses, then its `select` by `select`, leaving the scope of the `select` open.
    pub(super) fn comprehension_body_with(
        &mut self,
        c: &ast::Comprehension,
        select: &mut dyn FnMut(&mut Self, &Expr) -> R<Out>,
    ) -> R<Body> {
        self.scope.push();
        let mut frames = Vec::new();
        let mut binders = Vec::new();
        self.clauses(&c.clauses, &mut frames, &mut binders)?;
        if let Some((g, after)) = &c.group {
            let value = self.synth(&g.value)?;
            let key = self.synth(&g.by)?;
            let (labels, rec, rec_ty) = self.binder_record(&binders);
            let b_ty = Ty::Set(Box::new(rec_ty.clone()));
            let one = self.add(Kind::Singleton(rec), b_ty.clone());
            let b_tm = self.assemble(std::mem::take(&mut frames), one, &b_ty);
            let xi = self.fresh(rec_ty.clone());
            let xt = self.var_tm(xi);
            let by = self.over_record(&labels, xt, key.tm);
            let rows_ty = Ty::NonEmpty(Box::new(rec_ty));
            let zeta_ty = Ty::Record(vec![
                ("key".into(), key.ty),
                ("rows".into(), rows_ty.clone()),
            ]);
            let group_tm = self.add(
                Kind::Group {
                    var: xi,
                    source: b_tm,
                    by,
                },
                Ty::Set(Box::new(zeta_ty.clone())),
            );
            self.scope.pop();
            self.scope.push();
            let zeta = self.terms.var(&g.into.text, zeta_ty.clone());
            self.scope.bind(
                &g.into.text,
                Local {
                    var: zeta,
                    ty: zeta_ty,
                    rows: None,
                    group: Some(Rc::new(GroupInfo {
                        binders: labels,
                        value: value.tm,
                        value_ty: value.ty,
                        value_rows: value.rows,
                        rows_ty,
                    })),
                },
            );
            frames.push(Frame::For(zeta, group_tm));
            binders = vec![zeta];
            self.clauses(after, &mut frames, &mut binders)?;
        }
        let select = select(self, &c.select)?;
        Ok(Body {
            frames,
            select,
            binders,
        })
    }

    /// Closes a comprehension body as a set of its selected values.
    pub(super) fn finish(&mut self, body: Body, elem: Option<&Ty>) -> Out {
        let ty = elem.cloned().unwrap_or_else(|| body.select.ty.clone());
        let set_ty = Ty::Set(Box::new(ty));
        let one = self.add(Kind::Singleton(body.select.tm), set_ty.clone());
        let key_preserving = match self.terms.kind(body.select.tm) {
            Kind::Var(v) => body.binders.contains(v),
            _ => false,
        };
        let empty = body
            .frames
            .iter()
            .any(|frame| matches!(frame, Frame::For(_, source) if self.empty_row_term(*source)));
        let rows = if empty {
            RowOrigin::Empty
        } else if key_preserving {
            self.row_origin(&body.select)
        } else {
            RowOrigin::Unknown
        };
        let tm = self.assemble(body.frames, one, &set_ty);
        self.scope.pop();
        self.with_row_origin(
            Out {
                tm,
                ty: set_ty,
                rows: None,
            },
            rows,
        )
    }

    pub(super) fn comprehension(
        &mut self,
        c: &ast::Comprehension,
        elem: Option<&Ty>,
        span: Span,
    ) -> R<Out> {
        if c.order.is_some() {
            return Err(diag(
                span,
                "`order by` is allowed only on the outermost comprehension of a registered query or an ad hoc statement",
            ));
        }
        let body = self.comprehension_body(c, elem)?;
        Ok(self.finish(body, elem))
    }

    pub(super) fn exists(&mut self, gens: &[ast::Generator], filter: Option<&Expr>) -> R<Out> {
        self.scope.push();
        let mut frames = Vec::new();
        let mut binders = Vec::new();
        let r = (|| -> R<()> {
            self.clauses(
                &[ast::Clause::From(gens.to_vec())],
                &mut frames,
                &mut binders,
            )?;
            if let Some(p) = filter {
                self.clauses(&[ast::Clause::Where(p.clone())], &mut frames, &mut binders)?;
            }
            Ok(())
        })();
        self.scope.pop();
        r?;
        let set_ty = Ty::Set(Box::new(Ty::Unit));
        let unit = self.add(Kind::Record(Vec::new()), Ty::Unit);
        let one = self.add(Kind::Singleton(unit), set_ty.clone());
        let tm = self.assemble(frames, one, &set_ty);
        let bt = self.bool_ty();
        Ok(self.out(Kind::Exists(tm), bt))
    }

    /// Rewrites ordering keys written in the scope of a comprehension's `select` as functions of the selected value `x`.
    fn keys_over(&mut self, body: &Body, keys: &[ast::OrderKey]) -> R<(Var, Vec<(Tm, bool)>)> {
        let mut out = Vec::new();
        for k in keys {
            let o = self.synth(&k.expr)?;
            if !self.ordered(&o.ty) {
                return Err(diag(
                    k.expr.span,
                    format!(
                        "an ordering key has an ordered type, found {}",
                        self.show(&o.ty)
                    ),
                ));
            }
            out.push((o.tm, k.desc, k.expr.span));
        }
        let x = match self.terms.kind(body.select.tm) {
            Kind::Var(v) => *v,
            _ => self.fresh(body.select.ty.clone()),
        };
        let mut keys_out = Vec::new();
        for (tm, desc, span) in out {
            let tm = self.over_selected(tm, body.select.tm, x);
            if self.mentions_any(tm, &body.binders, x) {
                return Err(diag(
                    span,
                    "an ordering key reads only the selected value, or fields the selected value holds unchanged",
                ));
            }
            keys_out.push((tm, desc));
        }
        Ok((x, keys_out))
    }

    /// Replaces each subterm of `t` equal to the selected value, or to a field of it, by `x` or its field.
    fn over_selected(&mut self, t: Tm, select: Tm, x: Var) -> Tm {
        if self.same_term(t, select) {
            return self.var_tm(x);
        }
        let fields: Vec<(String, Tm)> = match self.terms.kind(select) {
            Kind::Record(fs) | Kind::Ctor { fields: fs, .. } => fs.clone(),
            _ => Vec::new(),
        };
        for (label, ft) in &fields {
            if self.same_term(t, *ft) {
                let xt = self.var_tm(x);
                let ty = self.terms.ty(t).clone();
                return self.add(Kind::Field(xt, label.clone()), ty);
            }
        }
        let node = self.terms.node(t).clone();
        let kind = match node.kind {
            Kind::Field(inner, f) => Kind::Field(self.over_selected(inner, select, x), f),
            Kind::Key(inner) => Kind::Key(self.over_selected(inner, select, x)),
            Kind::Prim(p, args) => Kind::Prim(
                p,
                args.into_iter()
                    .map(|a| self.over_selected(a, select, x))
                    .collect(),
            ),
            Kind::Deref { key, reference } => Kind::Deref {
                key: self.over_selected(key, select, x),
                reference,
            },
            Kind::Tag(inner) => Kind::Tag(self.over_selected(inner, select, x)),
            _ => return t,
        };
        self.add(kind, node.ty)
    }

    fn same_term(&self, a: Tm, b: Tm) -> bool {
        if a == b {
            return true;
        }
        let (na, nb) = (self.terms.node(a), self.terms.node(b));
        if na.ty != nb.ty {
            return false;
        }
        match (&na.kind, &nb.kind) {
            (Kind::Var(x), Kind::Var(y)) => x == y,
            (Kind::Field(x, f), Kind::Field(y, g)) => f == g && self.same_term(*x, *y),
            (Kind::Key(x), Kind::Key(y))
            | (Kind::Tag(x), Kind::Tag(y))
            | (Kind::Deref { key: x, .. }, Kind::Deref { key: y, .. }) => self.same_term(*x, *y),
            (Kind::Const(x), Kind::Const(y)) => x == y,
            (Kind::Prim(p, xs), Kind::Prim(q, ys)) => {
                p == q
                    && xs.len() == ys.len()
                    && xs.iter().zip(ys).all(|(x, y)| self.same_term(*x, *y))
            }
            _ => false,
        }
    }

    /// Returns whether `t` reads any of `vars` other than `except`.
    fn mentions_any(&self, t: Tm, vars: &[Var], except: Var) -> bool {
        if let Kind::Var(v) = self.terms.kind(t) {
            return *v != except && vars.contains(v);
        }
        self.terms
            .children(t)
            .into_iter()
            .any(|c| self.mentions_any(c, vars, except))
    }

    /// The outermost query of a registered query or ad hoc statement, where `order by` is allowed.
    pub(super) fn top_query(&mut self, e: &Expr, expected: Option<&Ty>) -> R<Out> {
        let ExprKind::Comprehension(c) = &e.kind else {
            return match expected {
                Some(t) => self.check(e, t),
                None => self.synth(e),
            };
        };
        let Some(order) = &c.order else {
            return match expected {
                Some(t) => self.check(e, t),
                None => self.synth(e),
            };
        };
        let elem = expected.and_then(|t| self.elem_ty(t));
        let body = self.comprehension_body(c, elem.as_ref())?;
        let (x, keys) = match self.keys_over(&body, &order.keys) {
            Ok(r) => r,
            Err(d) => {
                self.scope.pop();
                return Err(d);
            }
        };
        let elem_ty = body.select.ty.clone();
        let rows = body.select.rows;
        let source = self.finish(body, elem.as_ref());
        let key_tms: Vec<Tm> = keys.iter().map(|(k, _)| *k).collect();
        if (order.take.is_some() || order.after.is_some())
            && !self.determinate(x, &elem_ty, rows, &key_tms)
        {
            return Err(diag(
                e.span,
                "`take` and `after` need a determinate order: include the row's key, or a field a key or unique covers",
            ));
        }
        let take = match &order.take {
            Some(t) => Some(self.check(t, &Ty::Base(Base::I64))?.tm),
            None => None,
        };
        let after = match &order.after {
            Some(a) => {
                let kty = if keys.len() == 1 {
                    self.terms.ty(keys[0].0).clone()
                } else {
                    let o = self.synth(a)?;
                    let o = match self.after_want(&o.ty, &keys) {
                        Some(want) if wants_expected(a) => self.check(a, &want)?,
                        _ => o,
                    };
                    return self.after_record(o, &keys, a.span, x, source, take);
                };
                let want = self.option_ty(kty);
                Some(self.check(a, &want)?.tm)
            }
            None => None,
        };
        let ty = source.ty.clone();
        let o = self.out(
            Kind::Present {
                var: x,
                source: source.tm,
                keys,
                take,
                after,
            },
            ty,
        );
        Ok(self.with_row_origin(o, self.row_origin(&source)))
    }

    /// Returns the type `after` expects of a cursor synthesized at `ty`: an `Option` of a record
    /// with its field names, each typed as the ordering key it stands for.
    fn after_want(&self, ty: &Ty, keys: &[(Tm, bool)]) -> Option<Ty> {
        let Ty::Data { def, args, .. } = ty else {
            return None;
        };
        let [Ty::Record(fs)] = args.as_slice() else {
            return None;
        };
        if *def != self.schema.prelude_type("Option") || fs.len() != keys.len() {
            return None;
        }
        let fields = fs
            .iter()
            .zip(keys)
            .map(|((name, _), (k, _))| (name.clone(), self.terms.ty(*k).clone()))
            .collect();
        Some(self.option_ty(Ty::Record(fields)))
    }

    /// `after c` for a compound ordering: `c` is an `Option` of a record whose fields, in order, have the keys' types.
    fn after_record(
        &mut self,
        o: Out,
        keys: &[(Tm, bool)],
        span: Span,
        x: Var,
        source: Out,
        take: Option<Tm>,
    ) -> R<Out> {
        let fields = match &o.ty {
            Ty::Data { def, args, .. } if *def == self.schema.prelude_type("Option") => {
                match args.as_slice() {
                    [Ty::Record(fs)] => fs.clone(),
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        };
        if fields.len() != keys.len() {
            return Err(diag(
                span,
                format!(
                    "`after` takes an `Option` of a record with {} fields, one per ordering key",
                    keys.len()
                ),
            ));
        }
        for ((name, fty), (k, _)) in fields.iter().zip(keys) {
            let kty = self.terms.ty(*k).clone();
            if !subtype(self.schema, fty, &kty) {
                return Err(diag(
                    span,
                    format!(
                        "field `{name}` of the `after` record is compared with an ordering key of {}, found {}",
                        self.show(&kty),
                        self.show(fty)
                    ),
                ));
            }
        }
        let ty = source.ty.clone();
        let out = self.out(
            Kind::Present {
                var: x,
                source: source.tm,
                keys: keys.to_vec(),
                take,
                after: Some(o.tm),
            },
            ty,
        );
        Ok(self.with_row_origin(out, self.row_origin(&source)))
    }

    pub(super) fn top(
        &mut self,
        count: &Expr,
        keys: &[ast::OrderKey],
        c: &ast::Comprehension,
        elem: Option<&Ty>,
        span: Span,
    ) -> R<Out> {
        let n = self.check(count, &Ty::Base(Base::I64))?;
        if c.order.is_some() {
            return Err(diag(span, "`order by` is not allowed inside `top`"));
        }
        let b = self.comprehension_body(c, elem)?;
        let (x, ks) = match self.keys_over(&b, keys) {
            Ok(r) => r,
            Err(d) => {
                self.scope.pop();
                return Err(d);
            }
        };
        let elem_ty = b.select.ty.clone();
        let rows = b.select.rows;
        let source = self.finish(b, elem);
        let key_tms: Vec<Tm> = ks.iter().map(|(k, _)| *k).collect();
        if !self.determinate(x, &elem_ty, rows, &key_tms) {
            return Err(diag(
                span,
                "`top` needs a determinate order: include the row's key, or a field a key or unique covers",
            ));
        }
        let ty = source.ty.clone();
        let o = self.out(
            Kind::Top {
                count: n.tm,
                keys: ks,
                var: x,
                source: source.tm,
            },
            ty,
        );
        Ok(self.with_row_origin(o, self.row_origin(&source)))
    }

    pub(super) fn fix(
        &mut self,
        binds: &[(ast::Ident, Expr)],
        body: &Expr,
        expected: Option<&Ty>,
    ) -> R<Out> {
        let origins = self.fix_origins(binds, &BTreeMap::new())?;
        let names: Vec<String> = binds.iter().map(|(n, _)| n.text.clone()).collect();
        let mut tys: Vec<Option<Ty>> = vec![None; binds.len()];
        let mut from_select = false;
        loop {
            let mut changed = false;
            for (i, (_, e)) in binds.iter().enumerate() {
                let mut operands = Vec::new();
                union_operands(e, &mut operands);
                for op in operands {
                    if empty_union(op) {
                        continue;
                    }
                    let unknown: Vec<&str> = names
                        .iter()
                        .zip(&tys)
                        .filter(|(_, t)| t.is_none())
                        .map(|(n, _)| n.as_str())
                        .collect();
                    let reads_unknown = mentions_names(op, &unknown);
                    if reads_unknown && !from_select {
                        continue;
                    }
                    self.scope.push();
                    for ((n, t), origin) in names.iter().zip(&tys).zip(&origins) {
                        if let Some(t) = t {
                            self.bind_origin(n, Ty::Set(Box::new(t.clone())), *origin);
                        }
                    }
                    let el = if reads_unknown {
                        self.select_bound(op, &unknown)
                    } else {
                        self.synth(op).and_then(|o| {
                            self.elem_ty(&o.ty).map(Some).ok_or_else(|| {
                                diag(
                                    op.span,
                                    format!("a `fix` binder is a set, found {}", self.show(&o.ty)),
                                )
                            })
                        })
                    };
                    self.scope.pop();
                    let Some(el) = el? else {
                        continue;
                    };
                    let next = match &tys[i] {
                        None => el,
                        Some(t) => self.join(t, &el).ok_or_else(|| {
                            diag(
                                op.span,
                                format!(
                                    "the operands of `union` have no common type: {} and {}",
                                    self.show(t),
                                    self.show(&el)
                                ),
                            )
                        })?,
                    };
                    if tys[i]
                        .as_ref()
                        .is_none_or(|t| !subtype(self.schema, &next, t))
                    {
                        tys[i] = Some(next);
                        changed = true;
                    }
                }
            }
            if !changed {
                if from_select || tys.iter().all(Option::is_some) {
                    break;
                }
                from_select = true;
            }
        }
        if let Some((n, _)) = binds
            .iter()
            .zip(&tys)
            .find(|(_, t)| t.is_none())
            .map(|(b, _)| b)
        {
            return Err(diag(
                n.span,
                format!(
                    "the type of `{}` is not known: give it a `union` operand whose `select` does not read the `fix` binders",
                    n.text
                ),
            ));
        }
        self.scope.push();
        let mut vars = Vec::new();
        for ((n, t), origin) in names.iter().zip(&tys).zip(&origins) {
            vars.push(self.bind_origin(n, Ty::Set(Box::new(t.clone().expect("solved"))), *origin));
        }
        let mut bound = Vec::new();
        let r = (|| -> R<Out> {
            for (i, (_, e)) in binds.iter().enumerate() {
                let want = Ty::Set(Box::new(tys[i].clone().expect("solved")));
                let o = self.check(e, &want)?;
                bound.push((vars[i], o.tm));
            }
            match expected {
                Some(t) => self.check(body, t),
                None => self.synth(body),
            }
        })();
        self.scope.pop();
        let b = r?;
        let ty = b.ty.clone();
        let o = self.out(Kind::Fix(bound, b.tm), ty);
        Ok(self.with_row_origin(o, self.row_origin(&b)))
    }

    /// Returns the type of the `select` of comprehension `op` when it reads none of `unknown`, its
    /// generators over sources that read none of them bound and its other clauses left out.
    fn select_bound(&mut self, op: &Expr, unknown: &[&str]) -> R<Option<Ty>> {
        let ExprKind::Comprehension(c) = &op.kind else {
            return Ok(None);
        };
        if c.group.is_some() {
            return Ok(None);
        }
        let mut unknown = unknown.to_vec();
        let mut kept = Vec::new();
        for clause in &c.clauses {
            let ast::Clause::From(gens) = clause else {
                continue;
            };
            for g in gens {
                if mentions_names(&g.source, &unknown) {
                    unknown.push(&g.binder.text);
                } else {
                    kept.push(g.clone());
                }
            }
        }
        if mentions_names(&c.select, &unknown) {
            return Ok(None);
        }
        if kept.is_empty() {
            return self.synth(&c.select).map(|o| Some(o.ty));
        }
        let reduced = ast::Comprehension {
            clauses: vec![ast::Clause::From(kept)],
            group: None,
            select: c.select.clone(),
            order: None,
        };
        let o = self.comprehension(&reduced, None, op.span)?;
        Ok(self.elem_ty(&o.ty))
    }

    pub(super) fn aggregate(
        &mut self,
        name: &str,
        value: &Expr,
        gens: &[ast::Generator],
        filter: Option<&Expr>,
        span: Span,
    ) -> R<Out> {
        let op = match name {
            "count" => Agg::Count,
            "sum" => Agg::Sum,
            "min" => Agg::Min,
            "max" => Agg::Max,
            "avg" => Agg::Avg,
            _ => return Err(diag(span, format!("`{name}` is not an aggregate"))),
        };
        self.scope.push();
        let r = self.aggregate_in(op, name, value, gens, filter);
        self.scope.pop();
        r
    }

    fn group_local(&self, e: &Expr) -> Option<Local> {
        let ExprKind::Name(q) = &e.kind else {
            return None;
        };
        if q.module.is_some() {
            return None;
        }
        self.scope
            .lookup(&q.name.text)
            .filter(|l| l.group.is_some())
            .cloned()
    }

    /// `agg α (x in Q . e)` for one generator over a set; otherwise `agg α (ξ in B . e)` with `B` the records of the generators' bindings, a generator over a group ranging over its rows.
    pub(super) fn aggregate_in(
        &mut self,
        op: Agg,
        name: &str,
        value: &Expr,
        gens: &[ast::Generator],
        filter: Option<&Expr>,
    ) -> R<Out> {
        let single = gens.len() == 1 && self.group_local(&gens[0].source).is_none();
        let (var, source, nonempty, record_binders) = if single {
            let g = &gens[0];
            let (src, elem) = self.generator_source(&g.source)?;
            let v = self.bind(&g.binder.text, elem.clone(), src.rows);
            let source = match filter {
                None => src.tm,
                Some(p) => {
                    let bt = self.bool_ty();
                    let pt = self.check(p, &bt)?;
                    let set_ty = Ty::Set(Box::new(elem));
                    let xt = self.var_tm(v);
                    let one = self.add(Kind::Singleton(xt), set_ty.clone());
                    let none = self.add(Kind::EmptySet, set_ty.clone());
                    let body = self.add(Kind::If(pt.tm, one, none), set_ty.clone());
                    self.add(Kind::For(v, src.tm, body), set_ty)
                }
            };
            (v, source, false, Vec::new())
        } else {
            let nonempty = gens.len() == 1 && filter.is_none();
            let set_of = |t: Ty| {
                if nonempty {
                    Ty::NonEmpty(Box::new(t))
                } else {
                    Ty::Set(Box::new(t))
                }
            };
            let mut frames = Vec::new();
            let mut binders = Vec::new();
            for g in gens {
                if let Some(local) = self.group_local(&g.source) {
                    let info = local.group.clone().expect("a group");
                    let row_ty = self.elem_ty(&info.rows_ty).expect("rows is a set");
                    let z = self.var_tm(local.var);
                    let rows_tm =
                        self.terms
                            .transport_field(z, "rows".into(), info.rows_ty.clone());
                    let xi = self.fresh(row_ty);
                    frames.push(Frame::For(xi, rows_tm));
                    binders.push(xi);
                    let xt = self.var_tm(xi);
                    let gv = self.group_value(&info, xt);
                    let one = self.add(Kind::Singleton(gv), set_of(info.value_ty.clone()));
                    let y = self.bind(&g.binder.text, info.value_ty.clone(), info.value_rows);
                    frames.push(Frame::For(y, one));
                    binders.push(y);
                } else {
                    let (src, elem) = self.generator_source(&g.source)?;
                    let v = self.bind(&g.binder.text, elem, src.rows);
                    frames.push(Frame::For(v, src.tm));
                    binders.push(v);
                }
            }
            if let Some(p) = filter {
                let bt = self.bool_ty();
                let pt = self.check(p, &bt)?;
                frames.push(Frame::Where(pt.tm));
            }
            let (labels, rec, rec_ty) = self.binder_record(&binders);
            let set_ty = set_of(rec_ty.clone());
            let one = self.add(Kind::Singleton(rec), set_ty.clone());
            let source = self.assemble(frames, one, &set_ty);
            let xi = self.fresh(rec_ty);
            (xi, source, nonempty, labels)
        };
        let v = match op {
            Agg::User(a) => {
                let def = &self.schema.aggregates[a as usize];
                let [(_, input)] = def.params.as_slice() else {
                    return Err(diag(
                        value.span,
                        format!("`{name}` is refused: an aggregate takes exactly one parameter"),
                    ));
                };
                let input = input.clone();
                self.check(value, &input)?
            }
            _ => self.synth(value)?,
        };
        let ty = match op {
            Agg::Count => Ty::Base(Base::I64),
            Agg::Sum => {
                if !self.is_num(&v.ty) {
                    return Err(diag(
                        value.span,
                        format!("`sum` adds numbers, found {}", self.show(&v.ty)),
                    ));
                }
                v.ty.clone()
            }
            Agg::Min | Agg::Max => {
                if !self.ordered(&v.ty) {
                    return Err(diag(
                        value.span,
                        format!("`{name}` needs an ordered type, found {}", self.show(&v.ty)),
                    ));
                }
                if nonempty {
                    v.ty.clone()
                } else {
                    self.option_ty(v.ty.clone())
                }
            }
            Agg::Avg => {
                if !matches!(v.ty, Ty::Base(Base::F32 | Base::F64)) {
                    return Err(diag(
                        value.span,
                        format!(
                            "`avg` averages `F32` or `F64`, found {}; convert first",
                            self.show(&v.ty)
                        ),
                    ));
                }
                if nonempty {
                    v.ty.clone()
                } else {
                    self.option_ty(v.ty.clone())
                }
            }
            Agg::User(a) => {
                let result = self.schema.aggregates[a as usize].result.clone();
                if nonempty {
                    result
                } else {
                    self.option_ty(result)
                }
            }
        };
        let body = if single {
            v.tm
        } else {
            let xt = self.var_tm(var);
            self.over_record(&record_binders, xt, v.tm)
        };
        Ok(self.out(
            Kind::Agg {
                op,
                var,
                source,
                body,
            },
            ty,
        ))
    }
}

/// Flattens `a union b union ...` into its operands.
fn union_operands<'e>(e: &'e Expr, out: &mut Vec<&'e Expr>) {
    match &e.kind {
        ExprKind::Binary(BinOp::Union, a, b) => {
            union_operands(a, out);
            union_operands(b, out);
        }
        _ => out.push(e),
    }
}

/// Returns whether `e` names any of `names` as an unqualified value.
fn mentions_names(e: &Expr, names: &[&str]) -> bool {
    let mut found = false;
    crate::visit::each_expr(e, &mut |x| {
        if let ExprKind::Name(q) = &x.kind
            && q.module.is_none()
            && names.contains(&q.name.text.as_str())
        {
            found = true;
        }
    });
    found
}
