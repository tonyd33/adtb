use super::expr::{every_ctor, wants_expected};
use super::placed::Placed;
use super::*;
use crate::ast::{CallArgs, Expr, ExprKind};
use crate::kernel::{Agg, Prim};
use crate::resolve::Unbound;

/// What the callee of a call denotes.
pub(super) enum Callee {
    Local(Local),
    Declared(ValueDef),
    Prelude(PreludeFn),
    Unknown(Unbound),
}

impl Checker<'_> {
    /// Check arguments against parameter types over `n` type parameters of a callee. Each
    /// parameter takes, by its variance in `result`'s callee type, the join of the lower bounds
    /// its arguments give or the meet of the upper bounds they and `result`'s expected type give;
    /// every argument is then placed at its parameter's type under that solution.
    pub(super) fn solve_args(
        &mut self,
        callee: (&str, &[String]),
        params: &[Ty],
        es: &[&Expr],
        result: Option<(&Ty, &Ty)>,
        span: Span,
    ) -> R<(Vec<Placed>, Vec<Option<Ty>>)> {
        let mut mine = Vec::new();
        let solved = self.solve_args_with(callee, params, es, result, span, &mut mine);
        for e in mine {
            self.solved_parts.remove(&e);
        }
        solved
    }

    fn solve_args_with(
        &mut self,
        callee: (&str, &[String]),
        params: &[Ty],
        es: &[&Expr],
        result: Option<(&Ty, &Ty)>,
        span: Span,
        mine: &mut Vec<*const Expr>,
    ) -> R<(Vec<Placed>, Vec<Option<Ty>>)> {
        let n = callee.1.len();
        let mut bounds: Vec<Bounds> = vec![Bounds::default(); n];
        let mut literals: Vec<Bounds> = vec![Bounds::default(); n];
        let mut deferred: Vec<(&Expr, usize)> = Vec::new();
        if n > 0 {
            for (e, pty) in es.iter().zip(params) {
                if !mentions_any_param(pty, n) || matches!(e.kind, ExprKind::Lambda(..)) {
                    continue;
                }
                if wants_expected(e) {
                    self.literal_parts(e, pty, n, &mut literals, &mut deferred, mine)?;
                } else {
                    let o = self.synth(e)?;
                    bound(self.schema, pty, &o.ty, Side::Lower, false, &mut bounds);
                    self.keep_part(e, o, mine);
                }
            }
            let unbounded: Vec<bool> = (0..n)
                .map(|j| bounds[j].is_empty() && literals[j].is_empty())
                .collect();
            for (e, pty) in es.iter().zip(params) {
                let open = (0..n).any(|j| unbounded[j] && mentions(pty, j as u32));
                if open && !self.solved_parts.contains_key(&std::ptr::from_ref(*e)) {
                    let o = self.synth(e)?;
                    bound(self.schema, pty, &o.ty, Side::Lower, true, &mut bounds);
                    self.keep_part(e, o, mine);
                }
            }
            if let Some((res, exp)) = result {
                bound(self.schema, res, exp, Side::Upper, false, &mut bounds);
            }
        }
        let refused = |j: usize, why: String| {
            diag(
                span,
                format!(
                    "the type argument `{}` of `{}` has no solution: {why}",
                    callee.1[j], callee.0
                ),
            )
        };
        let mut lowers: Vec<Option<Ty>> = Vec::new();
        for (j, b) in bounds.iter().enumerate() {
            let mut lower: Option<Ty> = None;
            for t in &b.lower {
                lower = Some(match lower {
                    None => t.clone(),
                    Some(l) => self.join(&l, t, span)?.ok_or_else(|| {
                        refused(
                            j,
                            format!(
                                "no type is above both {} and {}",
                                self.show(&l),
                                self.show(t)
                            ),
                        )
                    })?,
                });
            }
            for t in &literals[j].lower {
                lower = match lower {
                    None => Some(t.clone()),
                    Some(l) => Some(self.join(&l, t, span)?.unwrap_or(l)),
                };
            }
            lowers.push(lower);
        }
        for (e, j) in deferred {
            let Some(lower) = lowers[j].clone() else {
                continue;
            };
            let whole = every_ctor(&lower);
            let Ty::Data { def, .. } = whole else {
                continue;
            };
            if self.ctor_named(e).is_none_or(|(d, _)| d != def) {
                continue;
            }
            let o = self.check(e, &whole)?;
            lowers[j] = Some(self.join(&lower, &o.ty, span)?.unwrap_or(whole));
            self.keep_part(e, o, mine);
        }
        let mut sol: Vec<Option<Ty>> = Vec::new();
        for (j, (b, name)) in bounds.iter().zip(callee.1).enumerate() {
            let refused = |why: String| refused(j, why);
            let lower = lowers[j].take();
            let mut upper: Option<Ty> = None;
            for t in &b.upper {
                let t = self.current(t);
                upper = Some(match upper {
                    None => t,
                    Some(u) => ty::meet(self.schema, &u, &t).ok_or_else(|| {
                        refused(format!(
                            "no type is below both {} and {}",
                            self.show(&u),
                            self.show(&t)
                        ))
                    })?,
                });
            }
            if let (Some(l), Some(u)) = (&lower, &upper)
                && !self.subtype(l, u)
            {
                return Err(refused(format!(
                    "its lower bound {} is not below its upper bound {}",
                    self.show(l),
                    self.show(u)
                )));
            }
            let variance = result.and_then(|(res, _)| variance(self.schema, res, j as u32));
            if let (
                Some(Variance {
                    co: true,
                    contra: true,
                }),
                Some(l),
                Some(u),
            ) = (variance, &lower, &upper)
                && !self.subtype(u, l)
            {
                return Err(diag(
                    span,
                    format!(
                        "the type argument `{name}` of `{}` is not fixed by its arguments",
                        callee.0
                    ),
                ));
            }
            let contra = variance
                == Some(Variance {
                    co: false,
                    contra: true,
                });
            sol.push(if contra {
                upper.or(lower)
            } else {
                lower.or(upper)
            });
        }
        let mut tms = Vec::new();
        for (e, pty) in es.iter().zip(params) {
            tms.push(self.check_placed(e, &pty.subst(&solved(&sol)))?);
        }
        Ok((tms, sol))
    }

    /// Bound each type parameter `pty` mentions by the parts of literal `e` that have a type
    /// without an expected one, and add to `deferred` each constructor at a bare parameter that
    /// has none.
    fn literal_parts<'e>(
        &mut self,
        e: &'e Expr,
        pty: &Ty,
        n: usize,
        literals: &mut [Bounds],
        deferred: &mut Vec<(&'e Expr, usize)>,
        mine: &mut Vec<*const Expr>,
    ) -> R<()> {
        if !mentions_any_param(pty, n) || matches!(e.kind, ExprKind::Lambda(..)) {
            return Ok(());
        }
        if !wants_expected(e) || self.synthesizes(e) {
            let o = self.synth(e)?;
            bound(self.schema, pty, &o.ty, Side::Lower, true, literals);
            self.keep_part(e, o, mine);
            return Ok(());
        }
        match (&e.kind, pty) {
            (ExprKind::Ctor { .. }, Ty::Param(j)) => deferred.push((e, *j as usize)),
            (ExprKind::Record(ast::RecordExpr::Fields(fs, _)), Ty::Record(ftys)) => {
                for (name, f) in fs {
                    if let Some((_, t)) = ftys.iter().find(|(m, _)| *m == name.text) {
                        self.literal_parts(f, t, n, literals, deferred, mine)?;
                    }
                }
            }
            (ExprKind::Set(xs), Ty::Set(t) | Ty::NonEmpty(t)) => {
                for x in xs {
                    self.literal_parts(x, t, n, literals, deferred, mine)?;
                }
            }
            (
                ExprKind::Ctor {
                    record: Some(ast::RecordExpr::Fields(fs, _)),
                    ..
                },
                Ty::Data { def, args, .. },
            ) => {
                let Some((d, c)) = self.ctor_named(e) else {
                    return Ok(());
                };
                if d != *def {
                    return Ok(());
                }
                let ftys = ty::ctor_fields(self.schema, d, args, c);
                for (name, f) in fs {
                    if let Some((_, t, _)) = ftys.iter().find(|(m, _, _)| *m == name.text) {
                        self.literal_parts(f, t, n, literals, deferred, mine)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Keep `o`, the elaboration of `e`, for the check that places `e`.
    fn keep_part(&mut self, e: &Expr, o: Out, mine: &mut Vec<*const Expr>) {
        let k = std::ptr::from_ref(e);
        self.solved_parts.insert(k, o);
        mine.push(k);
    }

    /// Returns whether literal `e` and each literal it holds has a type without an expected one.
    fn synthesizes(&self, e: &Expr) -> bool {
        let fields = |r: Option<&ast::RecordExpr>| match r {
            Some(ast::RecordExpr::Fields(fs, _)) => fs.iter().all(|(_, f)| self.synthesizes(f)),
            _ => true,
        };
        match &e.kind {
            ExprKind::Set(es) => !es.is_empty() && es.iter().all(|x| self.synthesizes(x)),
            ExprKind::Binary(ast::BinOp::Union, _, _) => !expr::empty_union(e),
            ExprKind::Ctor { record, .. } => self.ctor_synthesizes(e) && fields(record.as_ref()),
            ExprKind::Record(r) => fields(Some(r)),
            ExprKind::Lambda(..) => false,
            _ => true,
        }
    }

    pub(super) fn lambda(
        &mut self,
        ps: &ast::LambdaParams,
        body: &Expr,
        expected: Option<&Ty>,
        span: Span,
    ) -> R<Out> {
        self.scope.push();
        let r = self.lambda_in(ps, body, expected, span);
        self.scope.pop();
        r
    }

    fn lambda_in(
        &mut self,
        ps: &ast::LambdaParams,
        body: &Expr,
        expected: Option<&Ty>,
        span: Span,
    ) -> R<Out> {
        match ps {
            ast::LambdaParams::One(x) => {
                let Some(Ty::Fn(a, r)) = expected else {
                    return Err(diag(
                        span,
                        "the parameter type of `fn x -> ...` is not known here; write `fn (x: T) -> ...`",
                    ));
                };
                let v = self.bind(&x.text, (**a).clone(), None, Provenance::Built);
                let b = self.check(body, r)?;
                let ty = Ty::Fn(a.clone(), r.clone());
                let tm = self.b().lam(v, b.tm, ty);
                Ok(self.out(tm))
            }
            ast::LambdaParams::List(params) => {
                let mut vars = Vec::new();
                let mut want = expected.cloned();
                for p in params {
                    let t = self.resolve_type(&p.ty, FnTypes::Allowed)?;
                    if let Some(Ty::Fn(a, r)) = &want {
                        if !subtype(self.schema, a, &t) {
                            return Err(diag(
                                p.ty.span,
                                format!(
                                    "expected a parameter of {}, found {}",
                                    self.show(a),
                                    self.show(&t)
                                ),
                            ));
                        }
                        want = Some((**r).clone());
                    } else {
                        want = None;
                    }
                    vars.push(self.bind(&p.name.text, t, None, Provenance::Built));
                }
                let b = match &want {
                    Some(r) => self.check(body, r)?,
                    None => self.synth(body)?,
                };
                let tm = self.lams(vars, b.tm, b.ty);
                let ty = self.terms.ty(tm).clone();
                Ok(Out::of(tm, ty))
            }
        }
    }

    pub(super) fn call(
        &mut self,
        callee: &ast::QName,
        args: &CallArgs,
        pins: Option<&ast::Version>,
        expected: Option<&Ty>,
        span: Span,
    ) -> R<Out> {
        let text = callee.name.text.as_str();
        if let Some(v) = pins
            && !matches!(self.callee(callee), Callee::Prelude(PreludeFn::File(_)))
        {
            return Err(diag(v.span, "only a file scan takes `pins`"));
        }
        if text.starts_with(|c: char| c.is_ascii_uppercase()) {
            return self.newtype_call(callee, args, span);
        }
        let def = match self.callee(callee) {
            Callee::Local(local) => return self.apply_local(local, args, span),
            Callee::Declared(def) => def,
            Callee::Unknown(e) => return Err(e.at(callee.span())),
            Callee::Prelude(f) => return self.prelude_callee(f, args, pins, span),
        };
        self.record_value(callee, def);
        match def {
            ValueDef::Function(f) => self.call_fn(f, args, expected, span),
            ValueDef::Aggregate(a) => match args {
                CallArgs::Aggregate {
                    expr,
                    generators,
                    filter,
                } => {
                    self.scope.push();
                    let r =
                        self.aggregate_in(Agg::User(a), text, expr, generators, filter.as_deref());
                    self.scope.pop();
                    r
                }
                CallArgs::List(_) => Err(diag(
                    span,
                    format!("`{text}` is an aggregate: `{text}(e for x in q)`"),
                )),
            },
            ValueDef::Relation(_) => Err(diag(
                span,
                format!("`{text}` is a relation, not a function"),
            )),
            _ => Err(diag(span, format!("`{text}` is not a function"))),
        }
    }

    /// Returns what `callee` denotes: a local binder, else a declaration in scope, else the
    /// prelude's function of that name.
    pub(super) fn callee(&self, callee: &ast::QName) -> Callee {
        let text = callee.name.text.as_str();
        if callee.module.is_none()
            && let Some(local) = self.scope.lookup(text).cloned()
        {
            return Callee::Local(local);
        }
        match self.find_value(callee) {
            Ok(def) => Callee::Declared(def),
            Err(e) => match PreludeFn::named(text) {
                Some(f) if callee.module.is_none() => Callee::Prelude(f),
                _ => Callee::Unknown(e),
            },
        }
    }

    fn prelude_callee(
        &mut self,
        f: PreludeFn,
        args: &CallArgs,
        pins: Option<&ast::Version>,
        span: Span,
    ) -> R<Out> {
        match args {
            CallArgs::Aggregate {
                expr,
                generators,
                filter,
            } => self.aggregate(f, expr, generators, filter.as_deref(), span),
            CallArgs::List(es) => self.prelude_call(f, es, pins, span),
        }
    }

    fn newtype_call(&mut self, callee: &ast::QName, args: &CallArgs, span: Span) -> R<Out> {
        let def = match self.lookup_type(callee) {
            Ok(TypeName::Decl(d)) => d,
            Ok(_) => {
                return Err(diag(
                    callee.span(),
                    format!("`{}` is not a newtype", callee.name.text),
                ));
            }
            Err(e) => return Err(e.at(callee.span())),
        };
        let TypeDefKind::Newtype { base, .. } = &self.schema.ty(def).kind else {
            return Err(diag(
                callee.span(),
                format!(
                    "`{}` is not a newtype; build a constructor with `{{ ... }}`",
                    callee.name.text
                ),
            ));
        };
        let base = base.clone();
        let CallArgs::List(es) = args else {
            return Err(diag(span, "a newtype takes one value"));
        };
        let [e] = es.as_slice() else {
            return Err(diag(span, "a newtype takes one value"));
        };
        let o = self.check(e, &base)?;
        let ty = Ty::Data {
            def,
            args: Vec::new(),
            ctors: None,
        };
        self.prim(Prim::Wrap, vec![o.tm], Some(ty), span)
    }

    fn apply_local(&mut self, local: Local, args: &CallArgs, span: Span) -> R<Out> {
        let CallArgs::List(es) = args else {
            return Err(diag(
                span,
                "an aggregate form applies to an aggregate, not a function value",
            ));
        };
        let mut ty = local.ty.clone();
        let mut f = self.var_tm(local.var);
        if es.is_empty() {
            match ty {
                Ty::Fn(a, r) if *a == Ty::Unit => {
                    let unit = self.b().unit();
                    let unit = self.placed_as_is(unit);
                    f = self.b().app(f, unit);
                    ty = *r;
                }
                _ => {
                    return Err(diag(
                        span,
                        format!("{} is not a function of no arguments", self.show(&local.ty)),
                    ));
                }
            }
        }
        for e in es {
            let Ty::Fn(a, r) = ty else {
                return Err(diag(
                    e.span,
                    format!("too many arguments for {}", self.show(&local.ty)),
                ));
            };
            let x = self.check_placed(e, &a)?;
            f = self.b().app(f, x);
            ty = *r;
        }
        Ok(Out {
            tm: f,
            ty,
            rows: None,
            provenance: Provenance::Built,
        })
    }

    fn call_fn(&mut self, f: FnId, args: &CallArgs, expected: Option<&Ty>, span: Span) -> R<Out> {
        let def = self.schema.function(f).clone();
        let CallArgs::List(es) = args else {
            return Err(diag(
                span,
                format!("`{}` is a function, not an aggregate", def.name),
            ));
        };
        if es.len() != def.params.len() {
            return Err(diag(
                span,
                format!(
                    "`{}` takes {} arguments, given {}",
                    def.name,
                    def.params.len(),
                    es.len()
                ),
            ));
        }
        let params: Vec<Ty> = def.params.iter().map(|(_, t)| t.clone()).collect();
        let es: Vec<&Expr> = es.iter().collect();
        let names: Vec<String> = def.type_params.iter().map(|(n, _)| n.clone()).collect();
        let (tms, sol) = self.solve_args(
            (&def.name, &names),
            &params,
            &es,
            expected.map(|e| (&def.result, e)),
            span,
        )?;
        let mut type_args = Vec::new();
        for (j, (name, bounds)) in def.type_params.iter().enumerate() {
            let Some(t) = sol[j].clone() else {
                return Err(diag(
                    span,
                    format!(
                        "the type argument `{name}` of `{}` is not known here",
                        def.name
                    ),
                ));
            };
            if t.mentions_fn() {
                return Err(diag(span, FN_TYPE_ARGUMENT));
            }
            for b in bounds {
                let ok = match b {
                    Bound::Ord => self.ordered(&t, span)?,
                    Bound::Num => self.is_num(&t),
                    Bound::Eq | Bound::Hash => true,
                };
                if !ok {
                    return Err(diag(
                        span,
                        format!(
                            "{} does not satisfy the bound `{name}: {b:?}` of `{}`",
                            self.show(&t),
                            def.name
                        ),
                    ));
                }
            }
            type_args.push(t);
        }
        let result = def.result.subst(&type_args);
        let mut o = Out::of(self.b().call(f, type_args, tms), result);
        o.rows = self.fn_rows[f.0 as usize];
        if o.rows.is_some() {
            o.provenance = Provenance::Snapshot;
        }
        Ok(o)
    }
}

fn mentions_any_param(t: &Ty, n: usize) -> bool {
    (0..n as u32).any(|i| mentions(t, i))
}

fn solved(sol: &[Option<Ty>]) -> Vec<Ty> {
    sol.iter()
        .enumerate()
        .map(|(i, t)| t.clone().unwrap_or(Ty::Param(i as u32)))
        .collect()
}

pub(super) fn mentions(t: &Ty, i: u32) -> bool {
    match t {
        Ty::Param(j) => *j == i,
        Ty::Data { args, .. } => args.iter().any(|a| mentions(a, i)),
        Ty::Set(x) | Ty::NonEmpty(x) | Ty::Tag(x) => mentions(x, i),
        Ty::Group(a, b) | Ty::Fn(a, b) => mentions(a, i) || mentions(b, i),
        Ty::Record(fs) => fs.iter().any(|(_, x)| mentions(x, i)),
        Ty::Sum { ctors, .. } => ctors
            .iter()
            .any(|(_, fs)| fs.iter().any(|(_, x)| mentions(x, i))),
        _ => false,
    }
}

/// The types the arguments and the expected result give one type parameter: types it must be
/// above, and types it must be below.
#[derive(Clone, Default)]
struct Bounds {
    lower: Vec<Ty>,
    upper: Vec<Ty>,
}

impl Bounds {
    fn is_empty(&self) -> bool {
        self.lower.is_empty() && self.upper.is_empty()
    }
}

/// Whether an actual type stands below or above the pattern it is matched against.
#[derive(Clone, Copy, PartialEq)]
enum Side {
    Lower,
    Upper,
}

impl Side {
    fn flipped(self) -> Side {
        match self {
            Side::Lower => Side::Upper,
            Side::Upper => Side::Lower,
        }
    }
}

/// Add to `bounds` what matching `actual` on `side` of `pattern` gives each type parameter.
/// `literal` takes the fields of records and sums as covariant.
fn bound(
    schema: &Schema,
    pattern: &Ty,
    actual: &Ty,
    side: Side,
    literal: bool,
    bounds: &mut [Bounds],
) {
    let both = |p: &Ty, a: &Ty, bounds: &mut [Bounds]| {
        if literal {
            bound(schema, p, a, side, literal, bounds);
        } else {
            bound(schema, p, a, Side::Lower, literal, bounds);
            bound(schema, p, a, Side::Upper, literal, bounds);
        }
    };
    match (pattern, actual) {
        (Ty::Param(i), t) => {
            let Some(b) = bounds.get_mut(*i as usize) else {
                return;
            };
            match side {
                Side::Lower => b.lower.push(t.clone()),
                Side::Upper => b.upper.push(t.clone()),
            }
        }
        (
            Ty::Data {
                def: d1, args: a1, ..
            },
            Ty::Data {
                def: d2, args: a2, ..
            },
        ) if d1 == d2 => {
            for (i, (p, a)) in a1.iter().zip(a2).enumerate() {
                if ty::occurs_in_field(schema, *d1, i as u32) {
                    bound(schema, p, a, side, literal, bounds);
                }
            }
        }
        (Ty::Set(p) | Ty::NonEmpty(p), Ty::Set(a) | Ty::NonEmpty(a)) => {
            bound(schema, p, a, side, literal, bounds);
        }
        (Ty::Tag(p), Ty::Tag(a)) => {
            bound(schema, p, a, Side::Lower, false, bounds);
            bound(schema, p, a, Side::Upper, false, bounds);
        }
        (Ty::Fn(p1, r1), Ty::Fn(p2, r2)) => {
            bound(schema, p1, p2, side.flipped(), false, bounds);
            bound(schema, r1, r2, side, false, bounds);
        }
        (Ty::Group(k1, v1), Ty::Group(k2, v2)) => {
            bound(schema, k1, k2, side, literal, bounds);
            bound(schema, v1, v2, side, literal, bounds);
        }
        (Ty::Record(fs), Ty::Record(gs)) => {
            for (n, p) in fs {
                if let Some((_, a)) = gs.iter().find(|(m, _)| m == n) {
                    both(p, a, bounds);
                }
            }
        }
        (Ty::Sum { ctors: cs, .. }, Ty::Sum { ctors: ds, .. }) => {
            for (c, fs) in cs {
                if let Some((_, gs)) = ds.iter().find(|(d, _)| d == c) {
                    for (n, p) in fs {
                        if let Some((_, a)) = gs.iter().find(|(m, _)| m == n) {
                            both(p, a, bounds);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// Where a type parameter occurs in a type: in a covariant position, a contravariant one, or
/// both.
#[derive(Clone, Copy, Default, PartialEq)]
struct Variance {
    co: bool,
    contra: bool,
}

/// Returns where type parameter `i` occurs in `ty`, `None` when it does not.
fn variance(schema: &Schema, ty: &Ty, i: u32) -> Option<Variance> {
    fn walk(schema: &Schema, ty: &Ty, i: u32, co: bool, out: &mut Variance) {
        let both = |t: &Ty, out: &mut Variance| {
            walk(schema, t, i, true, out);
            walk(schema, t, i, false, out);
        };
        match ty {
            Ty::Param(j) if *j == i => {
                if co {
                    out.co = true;
                } else {
                    out.contra = true;
                }
            }
            Ty::Data { def, args, .. } => {
                for (k, a) in args.iter().enumerate() {
                    if ty::occurs_in_field(schema, *def, k as u32) {
                        walk(schema, a, i, co, out);
                    }
                }
            }
            Ty::Set(t) | Ty::NonEmpty(t) => walk(schema, t, i, co, out),
            Ty::Group(k, t) => {
                walk(schema, k, i, co, out);
                walk(schema, t, i, co, out);
            }
            Ty::Fn(p, r) => {
                walk(schema, p, i, !co, out);
                walk(schema, r, i, co, out);
            }
            Ty::Tag(t) => both(t, out),
            Ty::Record(fs) => fs.iter().for_each(|(_, t)| both(t, out)),
            Ty::Sum { ctors, .. } => ctors
                .iter()
                .for_each(|(_, fs)| fs.iter().for_each(|(_, t)| both(t, out))),
            _ => {}
        }
    }
    let mut out = Variance::default();
    walk(schema, ty, i, true, &mut out);
    (out != Variance::default()).then_some(out)
}
