use super::*;
use crate::ast::{BinOp, Expr, ExprKind, Lit, UnOp};
use crate::kernel::{Const, Kind, Prim};

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
        Base::I8 => (i128::from(i8::MIN), i128::from(i8::MAX)),
        Base::I16 => (i128::from(i16::MIN), i128::from(i16::MAX)),
        Base::I32 => (i128::from(i32::MIN), i128::from(i32::MAX)),
        Base::I64 => (i128::from(i64::MIN), i128::from(i64::MAX)),
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
                let rows = self.row_origin(&x).join(self.row_origin(&y));
                let o = self.out(Kind::Union(x.tm, y.tm), expected.clone());
                self.with_row_origin(o, rows)
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
        if subtype(self.schema, &out.ty, expected) {
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
            join(self.schema, a, b)
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
                pins,
                body,
            } => self.at(*instant, point, pins.as_ref(), body),
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
                    .map_err(|()| diag(e.span, "integer literal out of range"))?;
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
                    .map_err(|()| diag(e.span, "decimal literal out of range"))?;
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
                        Ok(self.out(Kind::Const(Const::Float(f64::from(v as f32))), ty))
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
                let Some(per) = ast::duration_unit(unit) else {
                    return Err(diag(e.span, format!("`{unit}` is not a duration unit")));
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
                    if self.ambiguous(&q.name.text) {
                        return Err(diag(q.span(), e));
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

    /// `tm`, over the variables of `binders`, for one record of their values, each bound afresh.
    pub(super) fn over_record(&mut self, binders: &[(String, Var)], row: Tm, tm: Tm) -> Tm {
        let ty = self.terms.ty(tm).clone();
        let fresh: BTreeMap<Var, Var> = binders
            .iter()
            .map(|(_, v)| (*v, self.terms.var_like(*v)))
            .collect();
        let mut out = self.terms.renamed(tm, &fresh, &mut self.frees);
        for (label, v) in binders.iter().rev() {
            let vty = self.terms.var_info(*v).ty.clone();
            let f = self.terms.transport_field(row, label.clone(), vty);
            out = self.add(Kind::Let(fresh[v], f, out), ty.clone());
        }
        out
    }

    /// The grouped value for one row of a group.
    pub(super) fn group_value(&mut self, g: &GroupInfo, row: Tm) -> Tm {
        let f = self.var_tm(g.value);
        self.add(Kind::App(f, row), g.value_ty.clone())
    }

    /// A group binder used as a set: the values of its grouped expression.
    fn group_as_set(&mut self, zeta: Var, g: &GroupInfo) -> Out {
        let z = self.var_tm(zeta);
        let rows_tm = self
            .terms
            .transport_field(z, "rows".into(), g.rows_ty.clone());
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

    fn union_pair(&mut self, a: &Expr, b: &Expr) -> R<(Out, Out)> {
        if empty_union(a) {
            let y = self.synth(b)?;
            let want = self.elem_ty(&y.ty).map(|t| Ty::Set(Box::new(t)));
            Ok((self.arm_against(a, want.as_ref())?, y))
        } else {
            let x = self.synth(a)?;
            let y = if empty_union(b) {
                let want = self.elem_ty(&x.ty).map(|t| Ty::Set(Box::new(t)));
                self.arm_against(b, want.as_ref())?
            } else {
                self.synth(b)?
            };
            Ok((x, y))
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
                let (x, y) = self.union_pair(a, b)?;
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
                let rows = self.row_origin(&x).join(self.row_origin(&y));
                let (x, y) = (self.widen(x, &j), self.widen(y, &j));
                let o = self.out(Kind::Union(x.tm, y.tm), j);
                Ok(self.with_row_origin(o, rows))
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
                    if u32::from(*s) + u32::from(*t) > 38 {
                        return Err(diag(
                            span,
                            format!(
                                "`*` of `Decimal {s}` and `Decimal {t}` has scale {} above 38; rescale an operand first",
                                u32::from(*s) + u32::from(*t)
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
                out.sort_unstable();
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
        let out = self.out(Kind::Restrict(o.tm, set), Ty::Set(Box::new(narrowed)));
        Ok(self.with_row_origin(out, self.row_origin(&o)))
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
        let ones: Vec<Tm> = outs
            .into_iter()
            .map(|o| {
                let o = self.widen(o, &elem);
                self.add(Kind::Singleton(o.tm), set_ty.clone())
            })
            .collect();
        let tm = self.balanced_union(&ones, &set_ty);
        Ok(Out {
            tm,
            ty: set_ty,
            rows: None,
        })
    }

    /// Returns the union of `sets` in order, nested to a depth logarithmic in their number.
    fn balanced_union(&mut self, sets: &[Tm], ty: &Ty) -> Tm {
        match sets {
            [] => self.add(Kind::EmptySet, ty.clone()),
            [one] => *one,
            _ => {
                let (left, right) = sets.split_at(sets.len() / 2);
                let a = self.balanced_union(left, ty);
                let b = self.balanced_union(right, ty);
                self.add(Kind::Union(a, b), ty.clone())
            }
        }
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
        let rows = self.row_origin(&x).join(self.row_origin(&y));
        let o = self.out(Kind::If(cond.tm, x.tm, y.tm), ty);
        Ok(self.with_row_origin(o, rows))
    }
}

pub(super) fn empty_union(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Set(es) => es.is_empty(),
        ExprKind::Binary(BinOp::Union, a, b) => empty_union(a) && empty_union(b),
        _ => false,
    }
}
