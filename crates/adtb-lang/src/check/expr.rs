use super::write::Site;
use super::*;
use crate::ast::{BinOp, CallArgs, Expr, ExprKind, Lit, RecordExpr, UnOp};
use crate::kernel::{Agg, Const, FileFormat, Kind, Prim};

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

pub(super) fn is_literal(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Lit(Lit::Int(_) | Lit::Decimal(_) | Lit::Float(_)) => true,
        ExprKind::Unary(UnOp::Neg, x) => is_literal(x),
        _ => false,
    }
}

/// Forms that take their type from what is expected of them.
pub(super) fn wants_expected(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Lit(Lit::Int(_) | Lit::Decimal(_) | Lit::Float(_))
        | ExprKind::Ctor { .. }
        | ExprKind::Record(_)
        | ExprKind::Set(_)
        | ExprKind::Lambda(..) => true,
        ExprKind::Unary(UnOp::Neg, x) => is_literal(x),
        _ => false,
    }
}

fn every_ctor(ty: &Ty) -> Ty {
    match ty {
        Ty::Data { def, args, .. } => Ty::Data {
            def: *def,
            args: args.clone(),
            ctors: None,
        },
        Ty::Sum { ctors, .. } => Ty::Sum {
            ctors: ctors.clone(),
            sel: None,
        },
        t => t.clone(),
    }
}

/// The fractional digits of a numeric literal, absent for a float literal.
fn literal_scale(e: &Expr) -> Option<u8> {
    match &e.kind {
        ExprKind::Lit(Lit::Int(_)) => Some(0),
        ExprKind::Lit(Lit::Decimal(t)) => t
            .split_once('.')
            .and_then(|(_, f)| u8::try_from(f.len()).ok()),
        ExprKind::Unary(UnOp::Neg, x) => literal_scale(x),
        _ => None,
    }
}

fn literal_rank(e: &Expr) -> u8 {
    match &e.kind {
        ExprKind::Lit(Lit::Decimal(_)) => 1,
        ExprKind::Lit(Lit::Float(_)) => 2,
        ExprKind::Unary(UnOp::Neg, x) => literal_rank(x),
        _ => 0,
    }
}

const SCALES: [i128; 39] = {
    let mut t = [1i128; 39];
    let mut i = 1;
    while i < 39 {
        t[i] = t[i - 1] * 10;
        i += 1;
    }
    t
};

fn int_range(b: Base) -> Option<(i128, i128)> {
    Some(match b {
        Base::I8 => (i8::MIN as i128, i8::MAX as i128),
        Base::I16 => (i16::MIN as i128, i16::MAX as i128),
        Base::I32 => (i32::MIN as i128, i32::MAX as i128),
        Base::I64 => (i64::MIN as i128, i64::MAX as i128),
        _ => return None,
    })
}

fn is_int(ty: &Ty) -> bool {
    matches!(ty, Ty::Base(b) if int_range(*b).is_some())
}

impl Checker<'_> {
    /// Checks `e` against `expected`.
    pub fn check(&mut self, e: &Expr, expected: &Ty) -> R<Out> {
        let out = match &e.kind {
            ExprKind::Lit(Lit::Int(_) | Lit::Decimal(_) | Lit::Float(_)) => {
                self.literal(e, Some(expected))?
            }
            ExprKind::Unary(UnOp::Neg, x) if is_literal(x) => self.literal(e, Some(expected))?,
            ExprKind::Record(r) => self.record(r, Some(expected), e.span, None)?,
            ExprKind::Ctor { path, ctor, record } => self.ctor(
                path,
                ctor.as_ref(),
                record.as_ref(),
                Some(expected),
                e.span,
                None,
            )?,
            ExprKind::Match(Some(s), arms) => self.match_expr(s, arms, Some(expected), e.span)?,
            ExprKind::If(c, a, b) => self.if_expr(c, a, b, Some(expected))?,
            ExprKind::Lambda(ps, body) => self.lambda(ps, body, Some(expected), e.span)?,
            ExprKind::Comprehension(c) => {
                let elem = self.elem_ty(expected);
                self.comprehension(c, elem.as_ref(), e.span)?
            }
            ExprKind::Binary(BinOp::Union, a, b) if self.elem_ty(expected).is_some() => {
                let x = self.check(a, expected)?;
                let y = self.check(b, expected)?;
                let rows = x.rows.filter(|r| y.rows == Some(*r));
                let mut o = self.out(Kind::Union(x.tm, y.tm), expected.clone());
                o.rows = rows;
                o
            }
            ExprKind::Set(es) => self.set_literal(es, Some(expected), e.span)?,
            ExprKind::Call { callee, args, pins } => {
                self.call(callee, args, pins.as_ref(), Some(expected), e.span)?
            }
            ExprKind::Fix(binds, body) => self.fix(binds, body, Some(expected))?,
            ExprKind::Top { count, keys, body } => {
                self.top(count, keys, body, self.elem_ty(expected).as_ref(), e.span)?
            }
            ExprKind::Reject(x) if self.reject_ok => {
                let o = self.check(x, &Ty::Base(Base::Text))?;
                self.out(Kind::Reject(o.tm), expected.clone())
            }
            _ => self.synth(e)?,
        };
        self.subsume(out, expected, e.span)
    }

    pub(super) fn subsume(&self, out: Out, expected: &Ty, span: Span) -> R<Out> {
        if ty::subtype(self.schema, &out.ty, expected) {
            Ok(out)
        } else {
            Err(diag(
                span,
                format!(
                    "expected {}, found {}",
                    self.show(expected),
                    self.show(&out.ty)
                ),
            ))
        }
    }

    pub(super) fn join(&self, a: &Ty, b: &Ty) -> Option<Ty> {
        if self.adhoc() {
            ty::join_adhoc(self.schema, a, b)
        } else {
            ty::join(self.schema, a, b)
        }
    }

    /// Synthesizes the type of `e`.
    pub fn synth(&mut self, e: &Expr) -> R<Out> {
        match &e.kind {
            ExprKind::Lit(_) => self.literal(e, None),
            ExprKind::Name(q) => self.name(q),
            ExprKind::Call { callee, args, pins } => {
                self.call(callee, args, pins.as_ref(), None, e.span)
            }
            ExprKind::TypeCall { ty, func, args } => self.type_call(ty, func, args, e.span),
            ExprKind::Ctor { path, ctor, record } => {
                self.ctor(path, ctor.as_ref(), record.as_ref(), None, e.span, None)
            }
            ExprKind::Record(r) => self.record(r, None, e.span, None),
            ExprKind::Set(es) => self.set_literal(es, None, e.span),
            ExprKind::Unit => Ok(self.out(Kind::Record(Vec::new()), Ty::Unit)),
            ExprKind::Unary(op, x) => self.unary(*op, x, e),
            ExprKind::Binary(op, a, b) => self.binary(*op, a, b, None, e.span),
            ExprKind::Field(x, f) => self.field(x, f),
            ExprKind::Select(x, sel) => self.select(x, sel, e.span),
            ExprKind::Backlink(x, rel, f) => self.backlink(x, rel, f),
            ExprKind::KeyCall(x, args) => self.key_call(x, args, e.span),
            ExprKind::Comprehension(c) => self.comprehension(c, None, e.span),
            ExprKind::Match(Some(s), arms) => self.match_expr(s, arms, None, e.span),
            ExprKind::Match(None, _) => Err(diag(
                e.span,
                "a match without a scrutinee appears only in a schema operator's `by match`",
            )),
            ExprKind::Fix(binds, body) => self.fix(binds, body, None),
            ExprKind::Lambda(ps, body) => self.lambda(ps, body, None, e.span),
            ExprKind::Exists(gens, filter) => self.exists(gens, filter.as_deref()),
            ExprKind::Top { count, keys, body } => self.top(count, keys, body, None, e.span),
            ExprKind::If(c, a, b) => self.if_expr(c, a, b, None),
            ExprKind::At {
                instant,
                point,
                body,
                ..
            } => self.at(*instant, point, body),
            ExprKind::Reject(_) if self.reject_ok => Err(diag(
                e.span,
                "`reject` takes the type expected of it, and none is known here",
            )),
            ExprKind::Reject(_) => Err(diag(
                e.span,
                "`reject` is allowed only in the source query of an insert",
            )),
            ExprKind::Write(w) => Err(diag(
                w.span,
                "a write appears only where a change set is expected: in a transact block, a script or ad hoc text",
            )),
            ExprKind::Elided => Err(diag(e.span, "an elided expression cannot be checked")),
        }
    }

    pub(super) fn literal(&mut self, e: &Expr, expected: Option<&Ty>) -> R<Out> {
        let (neg, lit) = match &e.kind {
            ExprKind::Lit(l) => (false, l),
            ExprKind::Unary(UnOp::Neg, x) => match &x.kind {
                ExprKind::Lit(l) => (true, l),
                _ => {
                    let inner = self.literal(x, expected)?;
                    let ty = inner.ty.clone();
                    return Ok(self.out(Kind::Prim(Prim::Neg, vec![inner.tm]), ty));
                }
            },
            _ => unreachable!("literal() is called on literals"),
        };
        let expected = expected.filter(|t| {
            self.is_num(t) || !matches!(lit, Lit::Int(_) | Lit::Decimal(_) | Lit::Float(_))
        });
        match lit {
            Lit::Int(digits) => {
                let ty = expected.cloned().unwrap_or(Ty::Base(Base::I64));
                let value: i128 = digits
                    .trim_start_matches('0')
                    .parse::<i128>()
                    .or_else(|_| {
                        if digits.chars().all(|c| c == '0') {
                            Ok(0)
                        } else {
                            Err(())
                        }
                    })
                    .map_err(|_| diag(e.span, "integer literal out of range"))?;
                let value = if neg { -value } else { value };
                self.number(value, 0, &ty, e.span)
            }
            Lit::Decimal(text) => {
                let (whole, frac) = text.split_once('.').expect("a decimal has a point");
                let scale = frac.len();
                if scale > 38 {
                    return Err(diag(e.span, "a decimal has at most 38 fractional digits"));
                }
                let ty = expected
                    .cloned()
                    .unwrap_or(Ty::Base(Base::Decimal(scale as u8)));
                if is_int(&ty) {
                    return Err(diag(
                        e.span,
                        format!(
                            "a fractional literal does not check against {}",
                            self.show(&ty)
                        ),
                    ));
                }
                let digits = format!("{whole}{frac}");
                let value: i128 = digits
                    .trim_start_matches('0')
                    .parse::<i128>()
                    .or_else(|_| {
                        if digits.chars().all(|c| c == '0') {
                            Ok(0)
                        } else {
                            Err(())
                        }
                    })
                    .map_err(|_| diag(e.span, "decimal literal out of range"))?;
                let value = if neg { -value } else { value };
                self.number(value, scale as u8, &ty, e.span)
            }
            Lit::Float(text) => {
                let ty = expected.cloned().unwrap_or(Ty::Base(Base::F64));
                let v: f64 = text
                    .parse()
                    .map_err(|_| diag(e.span, "malformed float literal"))?;
                let v = if neg { -v } else { v };
                match ty {
                    Ty::Base(Base::F64) if v.is_finite() => {
                        Ok(self.out(Kind::Const(Const::Float(v)), ty))
                    }
                    Ty::Base(Base::F32) if (v as f32).is_finite() => {
                        Ok(self.out(Kind::Const(Const::Float(v as f32 as f64)), ty))
                    }
                    Ty::Base(Base::F32 | Base::F64) => Err(diag(
                        e.span,
                        format!("literal out of range of {}", self.show(&ty)),
                    )),
                    _ => Err(diag(
                        e.span,
                        format!("a float literal does not check against {}", self.show(&ty)),
                    )),
                }
            }
            Lit::Text(t) => Ok(self.out(Kind::Const(Const::Text(t.clone())), Ty::Base(Base::Text))),
            Lit::Duration { amount, unit } => {
                let per: i64 = match unit.as_str() {
                    "us" => 1,
                    "ms" => 1_000,
                    "s" => 1_000_000,
                    "min" => 60_000_000,
                    "h" => 3_600_000_000,
                    "d" => 86_400_000_000,
                    _ => return Err(diag(e.span, format!("`{unit}` is not a duration unit"))),
                };
                let micros = i64::try_from(*amount)
                    .ok()
                    .and_then(|a| a.checked_mul(per))
                    .ok_or_else(|| diag(e.span, "duration out of range"))?;
                let micros = if neg { -micros } else { micros };
                Ok(self.out(
                    Kind::Const(Const::Duration(micros)),
                    Ty::Base(Base::Duration),
                ))
            }
            Lit::Bool(b) => {
                let tm = self.bool_tm(*b);
                Ok(Out {
                    tm,
                    ty: self.bool_ty(),
                    rows: None,
                })
            }
        }
    }

    /// A numeric literal `value * 10^-scale` at `ty`.
    fn number(&mut self, value: i128, scale: u8, ty: &Ty, span: Span) -> R<Out> {
        let range_err = || diag(span, format!("literal out of range of {}", self.show(ty)));
        match ty {
            Ty::Base(b) if int_range(*b).is_some() => {
                let (lo, hi) = int_range(*b).expect("an integer type");
                if scale != 0 {
                    return Err(diag(
                        span,
                        format!(
                            "a fractional literal does not check against {}",
                            self.show(ty)
                        ),
                    ));
                }
                if value < lo || value > hi {
                    return Err(range_err());
                }
                Ok(self.out(Kind::Const(Const::Int(value)), ty.clone()))
            }
            Ty::Base(Base::Decimal(s)) => {
                if scale > *s {
                    return Err(diag(
                        span,
                        format!(
                            "a literal with {scale} fractional digits does not check against {}",
                            self.show(ty)
                        ),
                    ));
                }
                let unscaled = value
                    .checked_mul(SCALES[(*s - scale) as usize])
                    .filter(|v| v.abs() < SCALES[38])
                    .ok_or_else(range_err)?;
                Ok(self.out(Kind::Const(Const::Decimal(unscaled)), ty.clone()))
            }
            Ty::Base(Base::F32 | Base::F64) => {
                let v = value as f64 / SCALES[scale as usize] as f64;
                Ok(self.out(Kind::Const(Const::Float(v)), ty.clone()))
            }
            Ty::Param(_) if scale == 0 => Ok(self.out(Kind::Const(Const::Int(value)), ty.clone())),
            _ => Err(diag(
                span,
                format!("a numeric literal does not check against {}", self.show(ty)),
            )),
        }
    }

    fn name(&mut self, q: &ast::QName) -> R<Out> {
        if q.module.is_none() {
            let text = q.name.text.as_str();
            if let Some(local) = self.scope.lookup(text).cloned() {
                if let Some(g) = &local.group {
                    return Ok(self.group_as_set(local.var, g));
                }
                if let Some((def, ctors, field)) = self.row_fields.get(&local.var).cloned() {
                    self.note_field(q.name.span, def, ctors.as_deref(), &field, false);
                }
                let tm = self.var_tm(local.var);
                return Ok(Out {
                    tm,
                    ty: local.ty,
                    rows: local.rows,
                });
            }
            if text == "caller" {
                if !matches!(self.ctx, Context::Function | Context::Adhoc) {
                    return Err(diag(q.name.span, "`caller` is not in scope here"));
                }
                let ty = Ty::Data {
                    def: self.schema.principal,
                    args: Vec::new(),
                    ctors: None,
                };
                return Ok(self.out(Kind::Caller, ty));
            }
        }
        match self.lookup_value(q) {
            Ok(ValueDef::Relation(r)) => Ok(self.rel_out(r, None)),
            Ok(ValueDef::Function(f)) => self.fn_value(f, q.span()),
            Ok(_) => Err(diag(
                q.span(),
                format!(
                    "`{}` is not a value; it names a constraint, check or aggregate",
                    q.name.text
                ),
            )),
            Err(e) => {
                if self.adhoc() && q.module.is_none() {
                    if let Some((v, ty)) = self.params.get(&q.name.text).cloned() {
                        let tm = self.add(Kind::Var(v), ty.clone());
                        return Ok(Out { tm, ty, rows: None });
                    }
                    return Err(diag(
                        q.span(),
                        format!(
                            "`{}` is a parameter of this text, but the call supplies no value for it",
                            q.name.text
                        ),
                    ));
                }
                Err(diag(q.span(), e))
            }
        }
    }

    /// A function named without a call: a lambda over its parameters.
    fn fn_value(&mut self, f: FnId, span: Span) -> R<Out> {
        let def = self.schema.function(f);
        if !def.type_params.is_empty() {
            return Err(diag(
                span,
                format!(
                    "generic function `{}` is passed only by a lambda that calls it",
                    def.name
                ),
            ));
        }
        let mut vars = Vec::new();
        let mut args = Vec::new();
        for (name, ty) in &def.params {
            let v = self.terms.var(name, ty.clone());
            vars.push(v);
            args.push(self.var_tm(v));
        }
        let call = self.add(
            Kind::Call {
                func: f,
                type_args: Vec::new(),
                args,
            },
            def.result.clone(),
        );
        let result = def.result.clone();
        let tm = self.lams(vars, call, result);
        let ty = self.terms.ty(tm).clone();
        Ok(Out { tm, ty, rows: None })
    }

    /// `tm`, over the variables of `binders`, for one record of their values.
    pub(super) fn over_record(&mut self, binders: &[(String, Var)], row: Tm, tm: Tm) -> Tm {
        let ty = self.terms.ty(tm).clone();
        let mut out = tm;
        for (label, v) in binders.iter().rev() {
            let vty = self.terms.var_info(*v).ty.clone();
            let f = self.add(Kind::Field(row, label.clone()), vty);
            out = self.add(Kind::Let(*v, f, out), ty.clone());
        }
        out
    }

    /// The grouped value for one row of a group.
    pub(super) fn group_value(&mut self, g: &GroupInfo, row: Tm) -> Tm {
        self.over_record(&g.binders.clone(), row, g.value)
    }

    /// A group binder used as a set: the values of its grouped expression.
    fn group_as_set(&mut self, zeta: Var, g: &GroupInfo) -> Out {
        let z = self.var_tm(zeta);
        let rows_tm = self.add(Kind::Field(z, "rows".into()), g.rows_ty.clone());
        let row_elem = self.elem_ty(&g.rows_ty).expect("rows is a set");
        let xi = self.fresh(row_elem);
        let xt = self.var_tm(xi);
        let value = self.group_value(g, xt);
        let set_ty = Ty::NonEmpty(Box::new(g.value_ty.clone()));
        let one = self.add(Kind::Singleton(value), set_ty.clone());
        let tm = self.add(Kind::For(xi, rows_tm, one), set_ty.clone());
        Out {
            tm,
            ty: set_ty,
            rows: g.value_rows,
        }
    }

    fn unary(&mut self, op: UnOp, x: &Expr, e: &Expr) -> R<Out> {
        match op {
            UnOp::Neg if is_literal(x) => self.literal(e, None),
            UnOp::Neg => {
                let o = self.synth(x)?;
                let ok = match &o.ty {
                    Ty::Base(b) => b.is_num() || matches!(b, Base::Duration | Base::Period),
                    t => self.is_num(t),
                };
                if !ok {
                    return Err(diag(
                        e.span,
                        format!("`-` does not apply to {}", self.show(&o.ty)),
                    ));
                }
                Ok(self.out(Kind::Prim(Prim::Neg, vec![o.tm]), o.ty))
            }
            UnOp::Not => {
                let b = self.bool_ty();
                let o = self.check(x, &b)?;
                let f = self.bool_tm(false);
                let t = self.bool_tm(true);
                Ok(self.out(Kind::If(o.tm, f, t), b))
            }
            UnOp::Key => {
                if let ExprKind::Write(w) = &x.kind {
                    return self.nested_insert(w);
                }
                let o = self.synth(x)?;
                self.key_of(o, x.span)
            }
            UnOp::Try => {
                let o = self.synth(x)?;
                let ty = Ty::Data {
                    def: self.schema.prelude_type("Result"),
                    args: vec![
                        o.ty,
                        Ty::Data {
                            def: self.schema.prelude_type("Fault"),
                            args: Vec::new(),
                            ctors: None,
                        },
                    ],
                    ctors: None,
                };
                Ok(self.out(Kind::Try(o.tm), ty))
            }
        }
    }

    pub(super) fn key_of(&mut self, o: Out, span: Span) -> R<Out> {
        let ctors = Self::ctors_of(&o.ty);
        if let Some(rel) = o.rows
            && matches!(o.ty, Ty::Data { .. })
        {
            return Ok(self.out(Kind::Key(o.tm), Ty::Key { rel, ctors }));
        }
        if let Ty::Data { def, .. } = &o.ty
            && let Some((rel, field)) = self.self_key(*def)
        {
            return Ok(self.out(Kind::Field(o.tm, field), Ty::Key { rel, ctors }));
        }
        Err(diag(
            span,
            format!(
                "`key` needs a row whose relation is known here, found a value of {}",
                self.show(&o.ty)
            ),
        ))
    }

    /// Checks two operands that must meet at one type, a literal or constructor taking the other's type with every constructor of its sum.
    pub(super) fn operands(&mut self, a: &Expr, b: &Expr, expected: Option<&Ty>) -> R<(Out, Out)> {
        match (wants_expected(a), wants_expected(b)) {
            (true, false) => {
                let y = self.synth(b)?;
                let x = self.against(a, &y.ty)?;
                Ok((x, y))
            }
            (false, true) => {
                let x = self.synth(a)?;
                let y = self.against(b, &x.ty)?;
                Ok((x, y))
            }
            (true, true) => match expected {
                Some(t) => Ok((self.check(a, t)?, self.check(b, t)?)),
                None => {
                    let x = self.synth(a)?;
                    let y = self.against(b, &x.ty)?;
                    Ok((x, y))
                }
            },
            (false, false) => Ok((self.synth(a)?, self.synth(b)?)),
        }
    }

    /// Checks `e` against the whole sum of `other`; in ad hoc text, synthesizes it beside an anonymous sum, which the two operands join.
    fn against(&mut self, e: &Expr, other: &Ty) -> R<Out> {
        match other {
            Ty::Sum { .. } if self.adhoc() => self.synth(e),
            t => self.check(e, &every_ctor(t)),
        }
    }

    fn binary(
        &mut self,
        op: BinOp,
        a: &Expr,
        b: &Expr,
        expected: Option<&Ty>,
        span: Span,
    ) -> R<Out> {
        match op {
            BinOp::And | BinOp::Or => {
                let bt = self.bool_ty();
                let x = self.check(a, &bt)?;
                let y = self.check(b, &bt)?;
                let kind = if op == BinOp::And {
                    let f = self.bool_tm(false);
                    Kind::If(x.tm, y.tm, f)
                } else {
                    let t = self.bool_tm(true);
                    Kind::If(x.tm, t, y.tm)
                };
                Ok(self.out(kind, bt))
            }
            BinOp::Union => {
                let x = self.synth(a)?;
                let y = self.synth(b)?;
                let j = match (&x.ty, &y.ty) {
                    (Ty::Set(_) | Ty::NonEmpty(_), Ty::Set(_) | Ty::NonEmpty(_)) => {
                        self.join(&x.ty, &y.ty)
                    }
                    _ => {
                        return Err(diag(span, "`union` combines two sets"));
                    }
                }
                .ok_or_else(|| {
                    diag(
                        span,
                        format!(
                            "the operands of `union` have no common type: {} and {}",
                            self.show(&x.ty),
                            self.show(&y.ty)
                        ),
                    )
                })?;
                let rows = x.rows.filter(|r| y.rows == Some(*r));
                let (x, y) = (self.widen(x, &j), self.widen(y, &j));
                let mut o = self.out(Kind::Union(x.tm, y.tm), j);
                o.rows = rows;
                Ok(o)
            }
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let (x, y) = self.operator_operands(op, a, b, None)?;
                let decimals = matches!(
                    (&x.ty, &y.ty),
                    (Ty::Base(Base::Decimal(_)), Ty::Base(Base::Decimal(_)))
                );
                let j = if decimals {
                    Some(x.ty.clone())
                } else {
                    self.join(&x.ty, &y.ty)
                }
                .ok_or_else(|| {
                    diag(
                        span,
                        format!(
                            "{} and {} cannot be compared",
                            self.show(&x.ty),
                            self.show(&y.ty)
                        ),
                    )
                })?;
                if j.mentions_fn() {
                    return Err(diag(span, "`==` never applies to a function"));
                }
                let (x, y) = (self.widen(x, &j), self.widen(y, &j));
                let ordering = !matches!(op, BinOp::Eq | BinOp::Ne);
                if ordering && !self.ordered(&j) {
                    return Err(diag(span, format!("{} has no order", self.show(&j))));
                }
                let prim = match op {
                    BinOp::Eq => Prim::Eq,
                    BinOp::Ne => Prim::Ne,
                    BinOp::Lt => Prim::Lt,
                    BinOp::Le => Prim::Le,
                    BinOp::Gt => Prim::Gt,
                    _ => Prim::Ge,
                };
                let bt = self.bool_ty();
                Ok(self.out(Kind::Prim(prim, vec![x.tm, y.tm]), bt))
            }
            BinOp::Concat => {
                let (x, y) = self.operands(a, b, expected)?;
                match (&x.ty, &y.ty) {
                    (Ty::Base(Base::Text), Ty::Base(Base::Text))
                    | (Ty::Base(Base::Bytes), Ty::Base(Base::Bytes)) => {
                        let ty = x.ty.clone();
                        Ok(self.out(Kind::Prim(Prim::Concat, vec![x.tm, y.tm]), ty))
                    }
                    _ => Err(diag(
                        span,
                        format!(
                            "`++` concatenates two texts or two byte strings, found {} and {}",
                            self.show(&x.ty),
                            self.show(&y.ty)
                        ),
                    )),
                }
            }
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => {
                let (x, y) = self.operator_operands(op, a, b, expected)?;
                let ty = self.arith_result(op, &x.ty, &y.ty, span)?;
                let prim = match op {
                    BinOp::Add => Prim::Add,
                    BinOp::Sub => Prim::Sub,
                    BinOp::Mul => Prim::Mul,
                    BinOp::Div => Prim::Div,
                    _ => Prim::Rem,
                };
                Ok(self.out(Kind::Prim(prim, vec![x.tm, y.tm]), ty))
            }
        }
    }

    /// Operands of `op`: a literal checks against the other operand, except that against a decimal it keeps its own scale and against a time length it is an `I64` count.
    pub(super) fn operator_operands(
        &mut self,
        op: BinOp,
        a: &Expr,
        b: &Expr,
        expected: Option<&Ty>,
    ) -> R<(Out, Out)> {
        match (is_literal(a), is_literal(b)) {
            (true, false) => {
                let y = self.synth(b)?;
                let x = self.literal_operand(op, a, &y.ty)?;
                Ok((x, y))
            }
            (false, true) => {
                let x = self.synth(a)?;
                let y = self.literal_operand(op, b, &x.ty)?;
                Ok((x, y))
            }
            (true, true) => {
                if let Some(t) = expected.filter(|t| self.is_num(t)) {
                    return Ok((self.literal(a, Some(t))?, self.literal(b, Some(t))?));
                }
                if literal_rank(b) > literal_rank(a) {
                    let y = self.literal(b, None)?;
                    let x = self.literal_operand(op, a, &y.ty)?;
                    Ok((x, y))
                } else {
                    let x = self.literal(a, None)?;
                    let y = self.literal_operand(op, b, &x.ty)?;
                    Ok((x, y))
                }
            }
            (false, false) => self.operands(a, b, expected),
        }
    }

    fn literal_operand(&mut self, op: BinOp, lit: &Expr, other: &Ty) -> R<Out> {
        let t = match other {
            Ty::Base(Base::Decimal(_)) => literal_scale(lit).map(|k| Ty::Base(Base::Decimal(k))),
            Ty::Base(Base::Duration | Base::Period) if matches!(op, BinOp::Mul | BinOp::Div) => {
                Some(Ty::Base(Base::I64))
            }
            t => Some(t.clone()),
        };
        self.literal(lit, t.as_ref().or(Some(other)))
    }

    /// An operand that must be a decimal, a literal taking its own scale.
    pub(super) fn decimal_operand(&mut self, e: &Expr) -> R<Out> {
        match literal_scale(e) {
            Some(k) if is_literal(e) => self.literal(e, Some(&Ty::Base(Base::Decimal(k)))),
            _ => self.synth(e),
        }
    }

    fn arith_result(&self, op: BinOp, a: &Ty, b: &Ty, span: Span) -> R<Ty> {
        use Base::*;
        let none = || {
            diag(
                span,
                format!(
                    "`{}` does not apply to {} and {}",
                    match op {
                        BinOp::Add => "+",
                        BinOp::Sub => "-",
                        BinOp::Mul => "*",
                        BinOp::Div => "/",
                        _ => "%",
                    },
                    self.show(a),
                    self.show(b)
                ),
            )
        };
        let r = match (a, b) {
            (Ty::Base(x), Ty::Base(y)) if is_int(a) && x == y => *x,
            (Ty::Base(Decimal(s)), Ty::Base(Decimal(t))) => match op {
                BinOp::Add | BinOp::Sub => Decimal((*s).max(*t)),
                BinOp::Mul => {
                    if *s as u32 + *t as u32 > 38 {
                        return Err(diag(
                            span,
                            format!(
                                "`*` of `Decimal {s}` and `Decimal {t}` has scale {} above 38; rescale an operand first",
                                *s as u32 + *t as u32
                            ),
                        ));
                    }
                    Decimal(s + t)
                }
                _ => {
                    return Err(diag(
                        span,
                        "decimals have no `/` and no `%`; use `div` with a scale and a rounding",
                    ));
                }
            },
            (Ty::Base(x @ (F32 | F64)), Ty::Base(y)) if x == y && op != BinOp::Rem => *x,
            (Ty::Base(Duration), Ty::Base(Duration)) if matches!(op, BinOp::Add | BinOp::Sub) => {
                Duration
            }
            (Ty::Base(Duration), Ty::Base(I64)) if matches!(op, BinOp::Mul | BinOp::Div) => {
                Duration
            }
            (Ty::Base(Period), Ty::Base(Period)) if matches!(op, BinOp::Add | BinOp::Sub) => Period,
            (Ty::Base(Period), Ty::Base(I64)) if op == BinOp::Mul => Period,
            (Ty::Base(t @ (Instant | DateTime | ZonedDateTime)), Ty::Base(Duration))
                if matches!(op, BinOp::Add | BinOp::Sub) =>
            {
                *t
            }
            (Ty::Base(Instant), Ty::Base(Instant))
            | (Ty::Base(DateTime), Ty::Base(DateTime))
            | (Ty::Base(Date), Ty::Base(Date))
                if op == BinOp::Sub =>
            {
                Duration
            }
            (Ty::Param(i), Ty::Param(j)) if i == j && self.param_has(*i, Bound::Num) => {
                if matches!(op, BinOp::Add | BinOp::Sub) {
                    return Ok(a.clone());
                }
                return Err(diag(
                    span,
                    "a generic body applies only `+` and `-` to a `Num` type; `*` and division are defined per concrete type",
                ));
            }
            _ => return Err(none()),
        };
        Ok(Ty::Base(r))
    }

    fn field(&mut self, x: &Expr, f: &ast::Ident) -> R<Out> {
        if let ExprKind::Name(q) = &x.kind
            && q.module.is_none()
            && let Some(local) = self.scope.lookup(&q.name.text).cloned()
            && local.group.is_some()
        {
            if f.text != "key" {
                return Err(diag(
                    f.span,
                    format!(
                        "a group binder has only `key`; `{}` is used as a set of its rows",
                        q.name.text
                    ),
                ));
            }
            let z = self.var_tm(local.var);
            let kty = self
                .fields_of(&self.terms.var_info(local.var).ty.clone())
                .into_iter()
                .find(|(n, _)| n == "key")
                .map(|(_, t)| t)
                .expect("a group row has a key");
            return Ok(self.out(Kind::Field(z, "key".into()), kty));
        }
        let o = self.synth(x)?;
        self.field_of(o, f)
    }

    /// `o.f`, following a reference.
    pub(super) fn field_of(&mut self, o: Out, f: &ast::Ident) -> R<Out> {
        let o = match &o.ty {
            Ty::Key { rel, ctors } => self.deref(o.clone(), *rel, ctors.clone()),
            _ => o,
        };
        if let Some((_, fty)) = self
            .fields_of(&o.ty)
            .into_iter()
            .find(|(n, _)| *n == f.text)
        {
            if let Ty::Data { def, ctors, .. } = &o.ty {
                self.note_field(f.span, *def, ctors.as_deref(), &f.text, false);
            }
            return Ok(self.out(Kind::Field(o.tm, f.text.clone()), fty));
        }
        if let Some(rel) = o.rows
            && let Some(c) = self
                .schema
                .relation(rel)
                .computed
                .iter()
                .find(|c| c.name == f.text)
                .cloned()
        {
            let saved = std::mem::replace(
                &mut self.vis,
                Visible::module(self.schema.relation(rel).module),
            );
            self.scope.push();
            let v = self.bind("self", o.ty.clone(), Some(rel));
            let body = self.check(&c.expr, &c.ty);
            self.scope.pop();
            self.vis = saved;
            let body = body?;
            return Ok(self.out(Kind::Let(v, o.tm, body.tm), c.ty));
        }
        let hint = match &o.ty {
            Ty::Data { def, .. } if *def == self.schema.prelude_type("Option") => {
                "; a path stops at an `Option`: match it".to_string()
            }
            Ty::Data {
                def, ctors: None, ..
            } if self.schema.ty(*def).ctors().len() > 1 => {
                "; only header fields are reached on the whole sum: match on the constructor"
                    .to_string()
            }
            _ => String::new(),
        };
        Err(diag(
            f.span,
            format!("{} has no field `{}`{hint}", self.show(&o.ty), f.text),
        ))
    }

    pub(super) fn deref(&mut self, key: Out, rel: RelId, ctors: Option<CtorSet>) -> Out {
        let reference = matches!(
            self.terms.kind(key.tm),
            Kind::Field(base, _) if matches!(self.terms.ty(*base), Ty::Data { .. })
        );
        let r = self.schema.relation(rel);
        let ty = match &r.elem {
            Ty::Data { def, args, .. } => Ty::Data {
                def: *def,
                args: args.clone(),
                ctors,
            },
            t => t.clone(),
        };
        let tm = self.add(
            Kind::Deref {
                key: key.tm,
                reference,
            },
            ty.clone(),
        );
        Out {
            tm,
            ty,
            rows: Some(rel),
        }
    }

    fn relation_name(&self, x: &Expr) -> Option<RelId> {
        let ExprKind::Name(q) = &x.kind else {
            return None;
        };
        if q.module.is_none() && self.scope.lookup(&q.name.text).is_some() {
            return None;
        }
        match self.lookup_value(q) {
            Ok(ValueDef::Relation(r)) => Some(r),
            _ => None,
        }
    }

    fn select(&mut self, x: &Expr, sel: &ast::CtorSel, span: Span) -> R<Out> {
        if let Some(rel) = self.relation_name(x) {
            let set = self.relation_sel(rel, sel).map_err(|e| diag(span, e))?;
            let def = self.schema.relation(rel).elem_def;
            return Ok(self.rel_out(rel, self.schema.normalize_ctors(def, set)));
        }
        let o = self.synth(x)?;
        let Some(elem) = self.elem_ty(&o.ty) else {
            return Err(diag(
                span,
                "`.C` selects constructors of a set; a single value is examined by `match`",
            ));
        };
        let set = match &elem {
            Ty::Data { def, ctors, .. } => {
                self.note_sel(*def, o.rows, sel);
                let s = self.schema.type_sel(*def, sel).map_err(|e| diag(span, e))?;
                if let Some(have) = ctors
                    && let Some(c) = s.iter().find(|c| !have.contains(c))
                {
                    return Err(diag(
                        span,
                        format!(
                            "`{}` is not among the constructors of {}",
                            self.schema.ty(*def).ctors()[*c as usize].name,
                            self.show(&elem)
                        ),
                    ));
                }
                s
            }
            Ty::Sum { ctors, sel: have } => {
                let names: Vec<&ast::Ident> = match sel {
                    ast::CtorSel::One(c) => vec![c],
                    ast::CtorSel::Many(cs) => cs.iter().collect(),
                };
                let mut out = Vec::new();
                for n in names {
                    let i = ctors
                        .iter()
                        .position(|(c, _)| *c == n.text)
                        .filter(|i| have.as_ref().is_none_or(|h| h.contains(&(*i as u32))))
                        .ok_or_else(|| {
                            diag(
                                n.span,
                                format!(
                                    "`{}` is not among the constructors of {}",
                                    n.text,
                                    self.show(&elem)
                                ),
                            )
                        })?;
                    out.push(i as u32);
                }
                out.sort();
                out.dedup();
                out
            }
            _ => {
                return Err(diag(
                    span,
                    format!("{} has no constructors to select", self.show(&elem)),
                ));
            }
        };
        let narrowed = ty::restrict(self.schema, &elem, set.clone());
        let mut out = self.out(Kind::Restrict(o.tm, set), Ty::Set(Box::new(narrowed)));
        out.rows = o.rows;
        Ok(out)
    }

    fn backlink(&mut self, x: &Expr, rref: &ast::RelationRef, f: &ast::Ident) -> R<Out> {
        let rel = match self.lookup_value(&rref.relation) {
            Ok(ValueDef::Relation(r)) => r,
            Ok(_) => {
                return Err(diag(
                    rref.span,
                    format!("`{}` is not a relation", rref.relation.name.text),
                ));
            }
            Err(e) => return Err(diag(rref.span, e)),
        };
        let r = self.schema.relation(rel);
        let def = self.schema.ty(r.elem_def);
        let set: CtorSet = match &rref.sel {
            Some(sel) => self
                .relation_sel(rel, sel)
                .map_err(|e| diag(rref.span, e))?,
            None => (0..def.ctors().len() as u32).collect(),
        };
        let with_field: CtorSet = set
            .iter()
            .copied()
            .filter(|&c| def.field(c, &f.text).is_some())
            .collect();
        let Some(&first) = with_field.first() else {
            return Err(diag(
                f.span,
                format!("no row of `{}` has a field `{}`", r.name, f.text),
            ));
        };
        self.note_field(f.span, r.elem_def, Some(&with_field), &f.text, false);
        let fty = def
            .field(first, &f.text)
            .expect("filtered")
            .ty
            .subst(r.elem_args());
        if with_field.iter().any(|&c| {
            def.field(c, &f.text)
                .expect("filtered")
                .ty
                .subst(r.elem_args())
                != fty
        }) {
            return Err(diag(
                f.span,
                format!(
                    "field `{}` has different types across the constructors of `{}`",
                    f.text, r.name
                ),
            ));
        }
        let (target, optional) = match &fty {
            Ty::Key { rel: t, .. } => (*t, false),
            Ty::Data { def: o, args, .. } if *o == self.schema.prelude_type("Option") => {
                match args.as_slice() {
                    [Ty::Key { rel: t, .. }] => (*t, true),
                    _ => {
                        return Err(diag(
                            f.span,
                            format!("`{}` is not a reference field", f.text),
                        ));
                    }
                }
            }
            _ => {
                return Err(diag(
                    f.span,
                    format!("`{}` is not a reference field", f.text),
                ));
            }
        };
        let xo = self.synth(x)?;
        let kappa = match &xo.ty {
            Ty::Key { rel: k, .. } if *k == target => xo,
            Ty::Key { rel: k, .. } => {
                return Err(diag(
                    x.span,
                    format!(
                        "a key of `{}` never matches `{}`, a reference into `{}`",
                        self.schema.relation(*k).name,
                        f.text,
                        self.schema.relation(target).name
                    ),
                ));
            }
            _ => {
                let k = self.key_of(xo, x.span)?;
                match &k.ty {
                    Ty::Key { rel: kr, .. } if *kr == target => k,
                    _ => {
                        return Err(diag(
                            x.span,
                            format!(
                                "`{}` references rows of `{}`",
                                f.text,
                                self.schema.relation(target).name
                            ),
                        ));
                    }
                }
            }
        };
        let ctors = self.schema.normalize_ctors(r.elem_def, with_field);
        let source = self.rel_out(rel, ctors);
        let elem = self.elem_ty(&source.ty).expect("a relation is a set");
        let xi = self.fresh(elem.clone());
        let xt = self.var_tm(xi);
        let field = self.add(Kind::Field(xt, f.text.clone()), fty.clone());
        let rhs = if optional {
            self.add(
                Kind::Ctor {
                    ctor: 1,
                    fields: vec![("value".into(), kappa.tm)],
                },
                fty,
            )
        } else {
            kappa.tm
        };
        let bt = self.bool_ty();
        let test = self.add(Kind::Prim(Prim::Eq, vec![field, rhs]), bt);
        let set_ty = Ty::Set(Box::new(elem));
        let xt = self.var_tm(xi);
        let one = self.add(Kind::Singleton(xt), set_ty.clone());
        let none = self.add(Kind::EmptySet, set_ty.clone());
        let body = self.add(Kind::If(test, one, none), set_ty.clone());
        let mut out = self.out(Kind::For(xi, source.tm, body), set_ty);
        out.rows = Some(rel);
        Ok(out)
    }

    fn key_call(&mut self, x: &Expr, args: &ast::KeyArgs, span: Span) -> R<Out> {
        let Some(rel) = self.relation_name(x) else {
            return Err(diag(x.span, "`.key(...)` follows a relation name"));
        };
        let vty = self.key_value_ty(rel).map_err(|e| diag(span, e))?;
        let value = match (args, &vty) {
            (ast::KeyArgs::Value(_), Ty::Record(_)) => {
                return Err(diag(
                    span,
                    format!(
                        "`{}` has a composite key; name its fields: `{}.key({})`",
                        self.schema.relation(rel).name,
                        self.schema.relation(rel).name,
                        self.schema
                            .relation(rel)
                            .key
                            .iter()
                            .map(|k| format!("{k}: ..."))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            }
            (ast::KeyArgs::Value(v), t) => self.check(v, t)?,
            (ast::KeyArgs::Named(ns), Ty::Record(fs)) => {
                let mut out = Vec::new();
                for (name, fty) in fs {
                    let (_, e) = ns
                        .iter()
                        .find(|(n, _)| n.text == *name)
                        .ok_or_else(|| diag(span, format!("key field `{name}` is missing")))?;
                    out.push((name.clone(), self.check(e, fty)?.tm));
                }
                if let Some((n, _)) = ns
                    .iter()
                    .find(|(n, _)| !fs.iter().any(|(f, _)| *f == n.text))
                {
                    return Err(diag(n.span, format!("`{}` is not a key field", n.text)));
                }
                self.out(Kind::Record(out), vty.clone())
            }
            (ast::KeyArgs::Named(ns), t) => match ns.as_slice() {
                [(n, e)] if self.schema.relation(rel).key == [n.text.clone()] => {
                    self.check(e, t)?
                }
                _ => {
                    return Err(diag(
                        span,
                        "this key has one field; give its value: `r.key(v)`",
                    ));
                }
            },
        };
        let ty = Ty::Key { rel, ctors: None };
        Ok(self.out(Kind::KeyOf(rel, value.tm), ty))
    }

    pub(super) fn record(
        &mut self,
        r: &RecordExpr,
        expected: Option<&Ty>,
        span: Span,
        site: Option<&Site>,
    ) -> R<Out> {
        if let Some(t @ Ty::Data { def, ctors, .. }) = expected
            && matches!(self.schema.ty(*def).kind, TypeDefKind::Sum { .. })
        {
            let single = match ctors {
                Some(cs) if cs.len() == 1 => Some(cs[0]),
                None if self.schema.ty(*def).ctors().len() == 1 => Some(0),
                _ => None,
            };
            if let Some(c) = single {
                let Ty::Data { def, args, .. } = t else {
                    unreachable!()
                };
                return self.build_ctor(
                    Target::Decl(*def, Some(args.clone())),
                    c,
                    Some(r),
                    span,
                    site,
                );
            }
        }
        match r {
            RecordExpr::Fields(fs, _) => {
                let want: Option<Vec<(String, Ty)>> = match expected {
                    Some(Ty::Record(ws)) => Some(ws.clone()),
                    _ => None,
                };
                let mut out: Vec<(String, Tm)> = Vec::new();
                let mut tys: Vec<(String, Ty)> = Vec::new();
                for (name, e) in fs {
                    if out.iter().any(|(n, _)| *n == name.text) {
                        return Err(diag(
                            name.span,
                            format!("field `{}` is given twice", name.text),
                        ));
                    }
                    let o = match want
                        .as_ref()
                        .and_then(|ws| ws.iter().find(|(n, _)| *n == name.text))
                    {
                        Some((_, t)) => {
                            let o = self.check(e, t)?;
                            Out { ty: t.clone(), ..o }
                        }
                        None => self.synth(e)?,
                    };
                    out.push((name.text.clone(), o.tm));
                    tys.push((name.text.clone(), o.ty));
                }
                out.sort_by(|a, b| a.0.cmp(&b.0));
                tys.sort_by(|a, b| a.0.cmp(&b.0));
                Ok(self.out(Kind::Record(out), Ty::Record(tys)))
            }
            RecordExpr::With(base, fs, _) => {
                let b = self.synth(base)?;
                let Ty::Record(bfs) = &b.ty else {
                    return Err(diag(
                        span,
                        format!(
                            "`{{ x with ... }}` copies an anonymous record; for {} name the constructor: `C {{ x with ... }}`",
                            self.show(&b.ty)
                        ),
                    ));
                };
                let mut out = Vec::new();
                let mut tys = Vec::new();
                for (name, fty) in bfs.clone() {
                    match fs.iter().find(|(n, _)| n.text == name) {
                        Some((_, e)) => {
                            let o = self.check(e, &fty)?;
                            out.push((name.clone(), o.tm));
                        }
                        None => {
                            let ft = self.add(Kind::Field(b.tm, name.clone()), fty.clone());
                            out.push((name.clone(), ft));
                        }
                    }
                    tys.push((name, fty));
                }
                if let Some((n, _)) = fs
                    .iter()
                    .find(|(n, _)| !bfs.iter().any(|(f, _)| *f == n.text))
                {
                    return Err(diag(
                        n.span,
                        format!("the record has no field `{}`", n.text),
                    ));
                }
                Ok(self.out(Kind::Record(out), Ty::Record(tys)))
            }
            RecordExpr::Elided(s) => Err(diag(*s, "an elided record cannot be checked")),
        }
    }

    /// Resolves a constructor expression against the expected type, else by its name in scope, then builds it.
    pub(super) fn ctor(
        &mut self,
        path: &ast::QName,
        ctor: Option<&ast::Ident>,
        record: Option<&RecordExpr>,
        expected: Option<&Ty>,
        span: Span,
        site: Option<&Site>,
    ) -> R<Out> {
        let name = match ctor {
            Some(c) => &c.text,
            None => &path.name.text,
        };
        let known_args = |def: TypeId| match expected {
            Some(Ty::Data { def: d, args, .. }) if *d == def => Some(args.clone()),
            _ => None,
        };
        let (target, c) = if ctor.is_some() {
            let def = match self.lookup_type(path) {
                Ok(TypeName::Decl(d)) => d,
                Ok(_) => {
                    return Err(diag(
                        path.span(),
                        format!("`{}` has no constructors", path.name.text),
                    ));
                }
                Err(e) => return Err(diag(path.span(), e)),
            };
            let c = self.schema.ty(def).ctor_index(name).ok_or_else(|| {
                diag(
                    span,
                    format!(
                        "`{name}` is not a constructor of `{}`",
                        self.schema.ty(def).name
                    ),
                )
            })?;
            (Target::Decl(def, known_args(def)), c)
        } else {
            let from_expected = match expected {
                Some(Ty::Data { def, args, .. }) => self
                    .schema
                    .ty(*def)
                    .ctor_index(name)
                    .map(|c| (Target::Decl(*def, Some(args.clone())), c)),
                Some(Ty::Sum { ctors, sel }) => ctors
                    .iter()
                    .position(|(c, _)| c == name)
                    .filter(|i| sel.as_ref().is_none_or(|s| s.contains(&(*i as u32))))
                    .map(|i| {
                        let row = Ty::Sum {
                            ctors: ctors.clone(),
                            sel: (ctors.len() != 1).then(|| vec![i as u32]),
                        };
                        (Target::Known(row), i as u32)
                    }),
                Some(t @ Ty::Tag(inner)) => match inner.as_ref() {
                    Ty::Data { def, .. } => self
                        .schema
                        .ty(*def)
                        .ctor_index(name)
                        .map(|c| (Target::Known(t.clone()), c)),
                    _ => None,
                },
                _ => None,
            };
            match from_expected {
                Some(r) if path.module.is_none() => r,
                _ => {
                    let found: Vec<(TypeId, u32)> = match &path.module {
                        Some(_) => match self.lookup_type(path) {
                            Ok(TypeName::Decl(d)) => self
                                .schema
                                .ty(d)
                                .ctor_index(name)
                                .map(|c| vec![(d, c)])
                                .unwrap_or_default(),
                            _ => Vec::new(),
                        },
                        None => self.ctors_named(name),
                    };
                    match found.as_slice() {
                        [(def, c)] => (Target::Decl(*def, known_args(*def)), *c),
                        [] if self.adhoc() && path.module.is_none() => {
                            return self.anon_ctor(name, record, span);
                        }
                        [] => {
                            return Err(diag(
                                path.span(),
                                match expected {
                                    Some(t) => {
                                        format!("`{name}` is not a constructor of {}", self.show(t))
                                    }
                                    None => format!(
                                        "no constructor `{name}` in scope, and no expected type declares it"
                                    ),
                                },
                            ));
                        }
                        many => {
                            return Err(diag(
                                path.span(),
                                format!(
                                    "`{name}` is a constructor of {}; qualify it as `T.{name}`",
                                    many.iter()
                                        .map(|(d, _)| format!("`{}`", self.schema.ty(*d).name))
                                        .collect::<Vec<_>>()
                                        .join(" and ")
                                ),
                            ));
                        }
                    }
                }
            }
        };
        if let Target::Decl(def, _) = &target {
            self.note(ctor.map_or(path.name.span, |c| c.span), Ref::Ctor(*def, c));
            let t = self.schema.ty(*def);
            if *def == self.schema.prelude_type("Span") {
                return Err(diag(span, "a span is built only by `span(start, end)`"));
            }
            if matches!(t.kind, TypeDefKind::Newtype { .. }) {
                return Err(diag(
                    span,
                    format!("newtype `{}` is built by a call: `{}(v)`", t.name, t.name),
                ));
            }
        }
        self.build_ctor(target, c, record, span, site)
    }

    fn anon_ctor(&mut self, name: &str, record: Option<&RecordExpr>, span: Span) -> R<Out> {
        let fields = match record {
            None => Vec::new(),
            Some(RecordExpr::Fields(fs, _)) => fs.clone(),
            Some(_) => {
                return Err(diag(
                    span,
                    format!("`{name} {{ x with ... }}` needs a declared constructor"),
                ));
            }
        };
        let mut tms = Vec::new();
        let mut tys = Vec::new();
        for (n, e) in &fields {
            let o = self.synth(e)?;
            tms.push((n.text.clone(), o.tm));
            tys.push((n.text.clone(), o.ty));
        }
        tms.sort_by(|a, b| a.0.cmp(&b.0));
        tys.sort_by(|a, b| a.0.cmp(&b.0));
        let ty = Ty::Sum {
            ctors: vec![(name.to_string(), tys)],
            sel: None,
        };
        Ok(self.out(
            Kind::Ctor {
                ctor: 0,
                fields: tms,
            },
            ty,
        ))
    }

    /// Checks arguments against parameter types over `n` type parameters of a callee, solving those parameters from the synthesized arguments, then from `result` against `expected`.
    pub(super) fn solve_args(
        &mut self,
        params: &[Ty],
        es: &[&Expr],
        n: usize,
        result: Option<(&Ty, &Ty)>,
    ) -> R<(Vec<Tm>, Vec<Option<Ty>>)> {
        let mut sol: Vec<Option<Ty>> = vec![None; n];
        let mut outs: Vec<Option<Out>> = vec![None; es.len()];
        if n > 0 {
            for (i, (e, pty)) in es.iter().zip(params).enumerate() {
                if !wants_expected(e) && mentions_any_param(pty, n) {
                    let o = self.synth(e)?;
                    unify(self, pty, &o.ty, &mut sol);
                    outs[i] = Some(o);
                }
            }
            if let Some((res, exp)) = result
                && sol.iter().any(Option::is_none)
            {
                let mut from_result = vec![None; n];
                unify(self, res, exp, &mut from_result);
                for (s, r) in sol.iter_mut().zip(from_result) {
                    if s.is_none() {
                        *s = r;
                    }
                }
            }
        }
        let mut tms = Vec::new();
        for (i, (e, pty)) in es.iter().zip(params).enumerate() {
            let o = match outs[i].take() {
                Some(o) => {
                    let want = pty.subst(&solved(&sol));
                    self.subsume(o, &want, e.span)?
                }
                None if (0..n).any(|j| sol[j].is_none() && mentions(pty, j as u32)) => {
                    let o = self.synth(e)?;
                    unify(self, pty, &o.ty, &mut sol);
                    o
                }
                None => {
                    let want = pty.subst(&solved(&sol));
                    self.check(e, &want)?
                }
            };
            tms.push(o.tm);
        }
        Ok((tms, sol))
    }

    /// Builds constructor `c` of `target` from `record`, giving every field.
    pub(super) fn build_ctor(
        &mut self,
        target: Target,
        c: u32,
        record: Option<&RecordExpr>,
        span: Span,
        site: Option<&Site>,
    ) -> R<Out> {
        let (def, args) = match target {
            Target::Known(ty) => {
                let fields: Vec<(String, Ty)> = ty::fields_of_ctor(self.schema, &ty, c)
                    .into_iter()
                    .map(|(n, t, _)| (n, t))
                    .collect();
                let given = self.record_fields(&fields, record, span, &ty, c)?;
                return Ok(self.out(
                    Kind::Ctor {
                        ctor: c,
                        fields: given,
                    },
                    ty,
                ));
            }
            Target::Decl(def, args) => (def, args),
        };
        let site = site.filter(|s| self.schema.relation(s.rel).elem_def == def);
        let t = self.schema.ty(def).clone();
        let n = t.params.len();
        let raw: Vec<(String, Ty, bool)> = ty::ctor_fields(
            self.schema,
            def,
            &(0..n as u32).map(Ty::Param).collect::<Vec<_>>(),
            c,
        );
        let ctor_name = t.ctors()[c as usize].name.clone();
        let mut named: Vec<(String, &Expr)> = Vec::new();
        let mut base: Option<Out> = None;
        match record {
            None => {
                if let Some((f, _, _)) = raw.first() {
                    return Err(diag(
                        span,
                        format!(
                            "`{ctor_name}` has fields, starting with `{f}`; give them: `{ctor_name} {{ ... }}`"
                        ),
                    ));
                }
            }
            Some(RecordExpr::Elided(s)) => {
                return Err(diag(*s, "an elided record cannot be checked"));
            }
            Some(RecordExpr::Fields(fs, _)) | Some(RecordExpr::With(_, fs, _)) => {
                for (name, e) in fs {
                    if named.iter().any(|(n, _)| *n == name.text) {
                        return Err(diag(
                            name.span,
                            format!("field `{}` is given twice", name.text),
                        ));
                    }
                    if !raw.iter().any(|(n, _, _)| *n == name.text) {
                        return Err(diag(
                            name.span,
                            format!("`{ctor_name}` has no field `{}`", name.text),
                        ));
                    }
                    self.note_field(name.span, def, Some(&[c]), &name.text, e.span == name.span);
                    named.push((name.text.clone(), e));
                }
                if let Some(s) = site {
                    self.site_given(s, fs)?;
                }
                if let Some(RecordExpr::With(b, _, _)) = record {
                    let o = self.synth(b)?;
                    if !matches!(&o.ty, Ty::Data { def: d, .. } if *d == def) {
                        return Err(diag(
                            b.span,
                            format!(
                                "`{ctor_name} {{ x with ... }}` copies from a value of `{}`, found {}",
                                t.name,
                                self.show(&o.ty)
                            ),
                        ));
                    }
                    base = Some(o);
                }
            }
        }
        let args = args.or_else(|| match &base {
            Some(Out {
                ty: Ty::Data { args, .. },
                ..
            }) => Some(args.clone()),
            _ => None,
        });
        let same_ctor = base.as_ref().is_some_and(|b| match &b.ty {
            Ty::Data { ctors: Some(s), .. } => *s == [c],
            Ty::Data { ctors: None, .. } => t.ctors().len() == 1,
            _ => false,
        });
        let (names, (params, es)): (Vec<String>, (Vec<Ty>, Vec<&Expr>)) = raw
            .iter()
            .filter_map(|(f, fty, _)| {
                named
                    .iter()
                    .find(|(n, _)| n == f)
                    .map(|(_, e)| (f.clone(), (fty.clone(), *e)))
            })
            .unzip();
        let (tms, sol) = match &args {
            Some(a) => {
                let params: Vec<Ty> = params.iter().map(|p| p.subst(a)).collect();
                let (tms, _) = self.solve_args(&params, &es, 0, None)?;
                (tms, a.iter().cloned().map(Some).collect())
            }
            None => self.solve_args(&params, &es, n, None)?,
        };
        let mut args_out = Vec::new();
        for (i, a) in sol.into_iter().enumerate() {
            match a {
                Some(a) => args_out.push(a),
                None => {
                    return Err(diag(
                        span,
                        format!(
                            "the type argument `{}` of `{}` is not known here; give the expected type",
                            t.params[i], t.name
                        ),
                    ));
                }
            }
        }
        let mut given: Vec<(String, Tm)> = names.into_iter().zip(tms).collect();
        for (f, fty, header) in &raw {
            if given.iter().any(|(g, _)| g == f) {
                continue;
            }
            let fty = fty.subst(&args_out);
            match &base {
                Some(b) if same_ctor || *header => {
                    let ft = self.add(Kind::Field(b.tm, f.clone()), fty);
                    given.push((f.clone(), ft));
                }
                None if *header && self.carry.is_some_and(|(_, d)| d == def) => {
                    let (row, _) = self.carry.expect("a carried row");
                    let rt = self.var_tm(row);
                    let ft = self.add(Kind::Field(rt, f.clone()), fty);
                    given.push((f.clone(), ft));
                }
                _ => match site {
                    Some(s) => match self.fill(s, c, f, *header, fty)? {
                        Some(ft) => given.push((f.clone(), ft)),
                        None => {
                            return Err(diag(
                                span,
                                format!(
                                    "field `{f}` of `{ctor_name}` is missing, and `{}` has no default for it",
                                    self.schema.relation(s.rel).name
                                ),
                            ));
                        }
                    },
                    None => {
                        return Err(diag(
                            span,
                            format!("field `{f}` of `{ctor_name}` is missing"),
                        ));
                    }
                },
            }
        }
        given.sort_by(|a, b| a.0.cmp(&b.0));
        let ty = Ty::Data {
            def,
            args: args_out,
            ctors: self.schema.normalize_ctors(def, vec![c]),
        };
        Ok(self.out(
            Kind::Ctor {
                ctor: c,
                fields: given,
            },
            ty,
        ))
    }

    /// The fields of a constructor of an anonymous sum or tag, each checked against its type.
    fn record_fields(
        &mut self,
        fields: &[(String, Ty)],
        record: Option<&RecordExpr>,
        span: Span,
        ty: &Ty,
        c: u32,
    ) -> R<Vec<(String, Tm)>> {
        let name = ty::ctor_names(self.schema, ty)
            .and_then(|ns| ns.into_iter().find(|(i, _)| *i == c).map(|(_, n)| n))
            .unwrap_or_default();
        let fs = match record {
            None => Vec::new(),
            Some(RecordExpr::Fields(fs, _)) => fs.clone(),
            Some(RecordExpr::With(..)) => {
                return Err(diag(
                    span,
                    format!("`{name} {{ x with ... }}` needs a declared constructor"),
                ));
            }
            Some(RecordExpr::Elided(s)) => {
                return Err(diag(*s, "an elided record cannot be checked"));
            }
        };
        let mut out = Vec::new();
        for (n, e) in &fs {
            let Some((_, fty)) = fields.iter().find(|(f, _)| *f == n.text) else {
                return Err(diag(n.span, format!("`{name}` has no field `{}`", n.text)));
            };
            out.push((n.text.clone(), self.check(e, fty)?.tm));
        }
        if let Some((f, _)) = fields
            .iter()
            .find(|(f, _)| !out.iter().any(|(g, _)| g == f))
        {
            return Err(diag(span, format!("field `{f}` of `{name}` is missing")));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    fn set_literal(&mut self, es: &[Expr], expected: Option<&Ty>, span: Span) -> R<Out> {
        let elem = expected.and_then(|t| self.elem_ty(t));
        let mut outs = Vec::new();
        for e in es {
            outs.push(match &elem {
                Some(t) => self.check(e, t)?,
                None => self.synth(e)?,
            });
        }
        let elem = match elem {
            Some(t) => t,
            None => {
                let mut it = outs.iter();
                let Some(first) = it.next() else {
                    return Err(diag(span, "the type of `[]` is not known here"));
                };
                let mut j = first.ty.clone();
                for o in it {
                    j = self
                        .join(&j, &o.ty)
                        .ok_or_else(|| diag(span, "the elements of a set have no common type"))?;
                }
                j
            }
        };
        let set_ty = Ty::Set(Box::new(elem.clone()));
        let mut tm = self.add(Kind::EmptySet, set_ty.clone());
        for o in outs.into_iter().rev() {
            let o = self.widen(o, &elem);
            let one = self.add(Kind::Singleton(o.tm), set_ty.clone());
            tm = self.add(Kind::Union(one, tm), set_ty.clone());
        }
        Ok(Out {
            tm,
            ty: set_ty,
            rows: None,
        })
    }

    pub(super) fn if_expr(
        &mut self,
        c: &Expr,
        a: &Expr,
        b: &Expr,
        expected: Option<&Ty>,
    ) -> R<Out> {
        let bt = self.bool_ty();
        let cond = self.check(c, &bt)?;
        let (x, y, ty) = match expected {
            Some(t) => (self.check(a, t)?, self.check(b, t)?, t.clone()),
            None => {
                let (x, y) = if self.deferred(a) && !self.deferred(b) {
                    let y = self.arm_against(b, None)?;
                    (self.arm_against(a, Some(&y.ty))?, y)
                } else {
                    let x = self.arm_against(a, None)?;
                    let y = self.arm_against(b, Some(&x.ty))?;
                    (x, y)
                };
                let j = self.join(&x.ty, &y.ty).ok_or_else(|| {
                    diag(
                        b.span,
                        format!(
                            "the branches of `if` have no common type: {} and {}",
                            self.show(&x.ty),
                            self.show(&y.ty)
                        ),
                    )
                })?;
                (self.widen(x, &j), self.widen(y, &j), j)
            }
        };
        let rows = x.rows.filter(|r| y.rows == Some(*r));
        let mut o = self.out(Kind::If(cond.tm, x.tm, y.tm), ty);
        o.rows = rows;
        Ok(o)
    }

    fn lambda(
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
                let v = self.bind(&x.text, (**a).clone(), None);
                let b = self.check(body, r)?;
                let ty = Ty::Fn(a.clone(), r.clone());
                Ok(self.out(Kind::Lam(v, b.tm), ty))
            }
            ast::LambdaParams::List(params) => {
                let mut vars = Vec::new();
                let mut want = expected.cloned();
                for p in params {
                    let t = self.resolve_type(&p.ty)?;
                    if let Some(Ty::Fn(a, r)) = &want {
                        if !ty::subtype(self.schema, a, &t) {
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
                    vars.push(self.bind(&p.name.text, t, None));
                }
                let b = match &want {
                    Some(r) => self.check(body, r)?,
                    None => self.synth(body)?,
                };
                let tm = self.lams(vars, b.tm, b.ty);
                let ty = self.terms.ty(tm).clone();
                Ok(Out { tm, ty, rows: None })
            }
        }
    }

    fn at(&mut self, instant: bool, point: &Expr, body: &Expr) -> R<Out> {
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
        let ty = b.ty.clone();
        Ok(self.out(
            Kind::At {
                instant,
                point: p.tm,
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
                        let v = self.bind(&g.binder.text, elem, src.rows);
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
                ("key".into(), key.ty.clone()),
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
        let rows = body.select.rows.filter(|_| key_preserving);
        let tm = self.assemble(body.frames, one, &set_ty);
        self.scope.pop();
        Out {
            tm,
            ty: set_ty,
            rows,
        }
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

    fn exists(&mut self, gens: &[ast::Generator], filter: Option<&Expr>) -> R<Out> {
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
            (Kind::Key(x), Kind::Key(y)) | (Kind::Tag(x), Kind::Tag(y)) => self.same_term(*x, *y),
            (Kind::Deref { key: x, .. }, Kind::Deref { key: y, .. }) => self.same_term(*x, *y),
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
                    return self.after_record(o, &keys, a.span, x, source, take);
                };
                let want = self.option_ty(kty);
                Some(self.check(a, &want)?.tm)
            }
            None => None,
        };
        let ty = source.ty.clone();
        let mut o = self.out(
            Kind::Present {
                var: x,
                source: source.tm,
                keys,
                take,
                after,
            },
            ty,
        );
        o.rows = source.rows;
        Ok(o)
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
            if !ty::subtype(self.schema, fty, &kty) {
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
        let mut out = self.out(
            Kind::Present {
                var: x,
                source: source.tm,
                keys: keys.to_vec(),
                take,
                after: Some(o.tm),
            },
            ty,
        );
        out.rows = source.rows;
        Ok(out)
    }

    fn top(
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
        let mut o = self.out(
            Kind::Top {
                count: n.tm,
                keys: ks,
                var: x,
                source: source.tm,
            },
            ty,
        );
        o.rows = source.rows;
        Ok(o)
    }

    fn fix(&mut self, binds: &[(ast::Ident, Expr)], body: &Expr, expected: Option<&Ty>) -> R<Out> {
        let names: Vec<String> = binds.iter().map(|(n, _)| n.text.clone()).collect();
        let mut tys: Vec<Option<Ty>> = vec![None; binds.len()];
        let mut from_select = false;
        loop {
            let mut changed = false;
            for (i, (_, e)) in binds.iter().enumerate() {
                let mut operands = Vec::new();
                union_operands(e, &mut operands);
                for op in operands {
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
                    for (n, t) in names.iter().zip(&tys) {
                        if let Some(t) = t {
                            self.bind(n, Ty::Set(Box::new(t.clone())), None);
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
                        .is_none_or(|t| !ty::subtype(self.schema, &next, t))
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
        for (n, t) in names.iter().zip(&tys) {
            vars.push(self.bind(n, Ty::Set(Box::new(t.clone().expect("solved"))), None));
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
        let mut o = self.out(Kind::Fix(bound, b.tm), ty);
        o.rows = b.rows;
        Ok(o)
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

    pub(super) fn call(
        &mut self,
        callee: &ast::QName,
        args: &CallArgs,
        pins: Option<&ast::Version>,
        expected: Option<&Ty>,
        span: Span,
    ) -> R<Out> {
        let text = callee.name.text.as_str();
        let format = match text {
            "read_csv" => Some(FileFormat::Csv),
            "read_ndjson" => Some(FileFormat::Ndjson),
            "read_parquet" => Some(FileFormat::Parquet),
            "read_arrow" => Some(FileFormat::Arrow),
            _ => None,
        };
        if let Some(v) = pins
            && (format.is_none() || callee.module.is_some())
        {
            return Err(diag(v.span, "only a file scan takes `pins`"));
        }
        if text.starts_with(|c: char| c.is_ascii_uppercase()) {
            return self.newtype_call(callee, args, span);
        }
        if callee.module.is_none()
            && let Some(local) = self.scope.lookup(text).cloned()
        {
            return self.apply_local(local, args, span);
        }
        match self.lookup_value(callee) {
            Ok(ValueDef::Function(f)) => return self.call_fn(f, args, expected, span),
            Ok(ValueDef::Aggregate(a)) => {
                return match args {
                    CallArgs::Aggregate {
                        expr,
                        generators,
                        filter,
                    } => {
                        self.scope.push();
                        let r = self.aggregate_in(
                            Agg::User(a),
                            text,
                            expr,
                            generators,
                            filter.as_deref(),
                        );
                        self.scope.pop();
                        r
                    }
                    CallArgs::List(_) => Err(diag(
                        span,
                        format!("`{text}` is an aggregate: `{text}(e for x in q)`"),
                    )),
                };
            }
            Ok(ValueDef::Relation(_)) => {
                return Err(diag(
                    span,
                    format!("`{text}` is a relation, not a function"),
                ));
            }
            Ok(_) => return Err(diag(span, format!("`{text}` is not a function"))),
            Err(e) => {
                if callee.module.is_some() || !PRELUDE_FUNCTIONS.contains(&text) {
                    return Err(diag(callee.span(), e));
                }
            }
        }
        match (args, format) {
            (
                CallArgs::Aggregate {
                    expr,
                    generators,
                    filter,
                },
                _,
            ) => self.aggregate(text, expr, generators, filter.as_deref(), span),
            (CallArgs::List(es), Some(format)) => self.file_scan(text, format, es, pins, span),
            (CallArgs::List(es), None) => self.prelude_call(text, es, span),
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
            Err(e) => return Err(diag(callee.span(), e)),
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
        Ok(self.out(Kind::Prim(Prim::Wrap, vec![o.tm]), ty))
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
                    let unit = self.add(Kind::Record(Vec::new()), Ty::Unit);
                    f = self.add(Kind::App(f, unit), (*r).clone());
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
            let x = self.check(e, &a)?.tm;
            f = self.add(Kind::App(f, x), (*r).clone());
            ty = *r;
        }
        Ok(Out {
            tm: f,
            ty,
            rows: None,
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
        let n = def.type_params.len();
        let (tms, sol) = self.solve_args(&params, &es, n, expected.map(|e| (&def.result, e)))?;
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
                return Err(diag(
                    span,
                    "a type argument is a value type, never a function type",
                ));
            }
            for b in bounds {
                let ok = match b {
                    Bound::Ord => self.ordered(&t),
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
        let mut o = self.out(
            Kind::Call {
                func: f,
                type_args,
                args: tms,
            },
            result,
        );
        o.rows = self.fn_rows[f.0 as usize];
        Ok(o)
    }

    fn aggregate(
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
    fn aggregate_in(
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
                    let rows_tm = self.add(Kind::Field(z, "rows".into()), info.rows_ty.clone());
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

/// Where a constructor expression resolved: a declared type with its arguments when known, or an anonymous sum or tag type.
pub(super) enum Target {
    Decl(TypeId, Option<Vec<Ty>>),
    Known(Ty),
}

fn mentions_any_param(t: &Ty, n: usize) -> bool {
    (0..n as u32).any(|i| mentions(t, i))
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

/// Collects lower bounds for the callee's type parameters in `pattern` from `actual`.
fn unify(c: &Checker, pattern: &Ty, actual: &Ty, sol: &mut [Option<Ty>]) {
    match (pattern, actual) {
        (Ty::Param(i), t) => {
            let i = *i as usize;
            if i < sol.len() {
                sol[i] = match &sol[i] {
                    None => Some(t.clone()),
                    Some(s) => Some(c.join(s, t).unwrap_or_else(|| s.clone())),
                };
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
            for (p, a) in a1.iter().zip(a2) {
                unify(c, p, a, sol);
            }
        }
        (Ty::Set(p) | Ty::NonEmpty(p), Ty::Set(a) | Ty::NonEmpty(a)) | (Ty::Tag(p), Ty::Tag(a)) => {
            unify(c, p, a, sol)
        }
        (Ty::Fn(p1, r1), Ty::Fn(p2, r2)) => {
            unify(c, p1, p2, sol);
            unify(c, r1, r2, sol);
        }
        (Ty::Group(k1, v1), Ty::Group(k2, v2)) => {
            unify(c, k1, k2, sol);
            unify(c, v1, v2, sol);
        }
        (Ty::Record(fs), Ty::Record(gs)) => {
            for (n, p) in fs {
                if let Some((_, a)) = gs.iter().find(|(m, _)| m == n) {
                    unify(c, p, a, sol);
                }
            }
        }
        _ => {}
    }
}
