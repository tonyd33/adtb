use super::*;
use crate::ast::{BinOp, Expr, ExprKind, Lit};
use crate::kernel::{FileFormat, Kind, Prim, Transition};

impl Checker<'_> {
    fn arity<'e>(&self, name: &str, es: &'e [Expr], n: usize, span: Span) -> R<&'e [Expr]> {
        if es.len() == n {
            Ok(es)
        } else {
            Err(diag(
                span,
                format!(
                    "`{name}` takes {n} argument{}, given {}",
                    if n == 1 { "" } else { "s" },
                    es.len()
                ),
            ))
        }
    }

    fn prim(&mut self, p: Prim, args: Vec<Tm>, ty: Ty) -> Out {
        self.out(Kind::Prim(p, args), ty)
    }

    fn named_ty(&self, name: &str) -> Ty {
        Ty::Data {
            def: self.schema.prelude_type(name),
            args: Vec::new(),
            ctors: None,
        }
    }

    fn base_arg(&mut self, e: &Expr, want: Base) -> R<Tm> {
        Ok(self.check(e, &Ty::Base(want))?.tm)
    }

    /// An argument that must be written as an integer literal, so its value is known when the call is checked.
    fn constant_int(&self, e: &Expr) -> R<u64> {
        match &e.kind {
            ExprKind::Lit(Lit::Int(d)) => {
                d.parse().map_err(|_| diag(e.span, "constant out of range"))
            }
            _ => Err(diag(
                e.span,
                "this argument is a constant: write an integer literal",
            )),
        }
    }

    /// The `Overflow` constructor written in place: true for `Reject`.
    fn constant_overflow(&self, e: &Expr) -> R<bool> {
        if let ExprKind::Ctor {
            path,
            ctor,
            record: None,
        } = &e.kind
        {
            let name = ctor.as_ref().unwrap_or(&path.name);
            match name.text.as_str() {
                "Clamp" => return Ok(false),
                "Reject" => return Ok(true),
                _ => {}
            }
        }
        Err(diag(
            e.span,
            "this argument is a constant: write `Clamp` or `Reject`",
        ))
    }

    fn constant_text(&self, e: &Expr) -> R<String> {
        match &e.kind {
            ExprKind::Lit(Lit::Text(t)) => Ok(t.clone()),
            _ => Err(diag(
                e.span,
                "this argument is a constant: write a text literal",
            )),
        }
    }

    fn is_float(ty: &Ty) -> bool {
        matches!(ty, Ty::Base(Base::F32 | Base::F64))
    }

    fn is_decimal(ty: &Ty) -> bool {
        matches!(ty, Ty::Base(Base::Decimal(_)))
    }

    fn is_integer(ty: &Ty) -> bool {
        matches!(ty, Ty::Base(Base::I8 | Base::I16 | Base::I32 | Base::I64))
    }

    fn wrong(&self, name: &str, tys: &[&Ty], span: Span) -> Diag {
        diag(
            span,
            format!(
                "`{name}` does not take {}",
                tys.iter()
                    .map(|t| self.show(t))
                    .collect::<Vec<_>>()
                    .join(" and ")
            ),
        )
    }

    pub(super) fn prelude_call(&mut self, name: &str, es: &[Expr], span: Span) -> R<Out> {
        if matches!(name, "count" | "sum" | "min" | "max" | "avg") {
            return Err(diag(
                span,
                format!("`{name}` is an aggregate: `{name}(e for x in q)`"),
            ));
        }
        let families = [
            Self::span_call,
            Self::text_call,
            Self::checked_call,
            Self::decimal_call,
            Self::convert_call,
            Self::string_call,
            Self::time_call,
            Self::change_call,
        ];
        for family in families {
            if let Some(out) = family(self, name, es, span)? {
                return Ok(out);
            }
        }
        Err(diag(span, format!("`{name}` is not a function")))
    }

    fn span_call(&mut self, name: &str, es: &[Expr], span: Span) -> R<Option<Out>> {
        let out = match name {
            "span" => {
                let [a, b] = self.arity(name, es, 2, span)? else {
                    unreachable!()
                };
                let (x, y) = self.operands(a, b, None)?;
                let t = self
                    .join(&x.ty, &y.ty)
                    .ok_or_else(|| self.wrong(name, &[&x.ty, &y.ty], span))?;
                if !self.ordered(&t) {
                    return Err(diag(
                        span,
                        format!("`Span a` needs an ordered `a`, found {}", self.show(&t)),
                    ));
                }
                let ty = Ty::Data {
                    def: self.schema.prelude_type("Span"),
                    args: vec![t],
                    ctors: None,
                };
                Ok(self.prim(Prim::Span, vec![x.tm, y.tm], ty))
            }
            "start" | "end" => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let o = self.synth(a)?;
                let t = self
                    .span_arg(&o.ty)
                    .ok_or_else(|| self.wrong(name, &[&o.ty], span))?;
                let p = if name == "start" {
                    Prim::Start
                } else {
                    Prim::End
                };
                Ok(self.prim(p, vec![o.tm], t))
            }
            "overlaps" | "covers" => {
                let [a, b] = self.arity(name, es, 2, span)? else {
                    unreachable!()
                };
                let x = self.synth(a)?;
                let t = self
                    .span_arg(&x.ty)
                    .ok_or_else(|| self.wrong(name, &[&x.ty], span))?;
                let y = if name == "covers" && !matches!(self.span_arg_expr(b), Some(true)) {
                    let probe = self.synth(b);
                    match probe {
                        Ok(o) if self.span_arg(&o.ty).is_some() => {
                            self.subsume(o, &x.ty, b.span)?
                        }
                        Ok(o) => self.subsume(o, &t, b.span)?,
                        Err(_) => self.check(b, &t)?,
                    }
                } else {
                    self.check(b, &x.ty)?
                };
                let p = if name == "covers" {
                    Prim::Covers
                } else {
                    Prim::Overlaps
                };
                let bt = self.bool_ty();
                Ok(self.prim(p, vec![x.tm, y.tm], bt))
            }
            _ => return Ok(None),
        };
        out.map(Some)
    }

    fn text_call(&mut self, name: &str, es: &[Expr], span: Span) -> R<Option<Out>> {
        let text = Ty::Base(Base::Text);
        let i64t = Ty::Base(Base::I64);
        let out = match name {
            "uuid" => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.check(a, &text)?;
                let ty = self.named_ty("Uuid");
                Ok(self.prim(Prim::ParseUuid, vec![x.tm], ty))
            }
            "text" => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.synth(a)?;
                let ok = match &x.ty {
                    Ty::Base(b) => {
                        b.is_num()
                            || matches!(
                                b,
                                Base::Instant
                                    | Base::Date
                                    | Base::Time
                                    | Base::DateTime
                                    | Base::ZonedDateTime
                                    | Base::Duration
                                    | Base::Period
                            )
                    }
                    Ty::Data { def, .. } => {
                        *def == self.schema.prelude_type("Uuid")
                            || matches!(
                                &self.schema.ty(*def).kind,
                                TypeDefKind::Newtype {
                                    base: Ty::Base(Base::Text),
                                    ..
                                }
                            )
                    }
                    _ => false,
                };
                if !ok {
                    return Err(self.wrong(name, &[&x.ty], span));
                }
                Ok(self.prim(Prim::ToText, vec![x.tm], text))
            }
            "bytes_len" => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.synth(a)?;
                if !matches!(x.ty, Ty::Base(Base::Text | Base::Bytes)) {
                    return Err(self.wrong(name, &[&x.ty], span));
                }
                Ok(self.prim(Prim::BytesLen, vec![x.tm], i64t))
            }
            "scalar_len" | "grapheme_len" => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::Text)?;
                let p = if name == "scalar_len" {
                    Prim::ScalarLen
                } else {
                    Prim::GraphemeLen
                };
                Ok(self.prim(p, vec![x], i64t))
            }
            "utf8" => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::Bytes)?;
                let ty = self.option_ty(text);
                Ok(self.prim(Prim::Utf8, vec![x], ty))
            }
            "bytes" => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::Text)?;
                Ok(self.prim(Prim::Bytes, vec![x], Ty::Base(Base::Bytes)))
            }
            _ => return Ok(None),
        };
        out.map(Some)
    }

    fn checked_call(&mut self, name: &str, es: &[Expr], span: Span) -> R<Option<Out>> {
        let out = match name {
            "checked_add" | "checked_sub" | "checked_mul" => {
                let [a, b] = self.arity(name, es, 2, span)? else {
                    unreachable!()
                };
                let op = match name {
                    "checked_add" => BinOp::Add,
                    "checked_sub" => BinOp::Sub,
                    _ => BinOp::Mul,
                };
                let (x, y) = self.operator_operands(op, a, b, None)?;
                let ty = match (&x.ty, &y.ty) {
                    (s, t) if Self::is_integer(s) && s == t => s.clone(),
                    (Ty::Base(Base::Decimal(s)), Ty::Base(Base::Decimal(t))) => {
                        if name == "checked_mul" {
                            if u32::from(*s) + u32::from(*t) > 38 {
                                return Err(diag(span, "the product's scale is above 38"));
                            }
                            Ty::Base(Base::Decimal(s + t))
                        } else {
                            Ty::Base(Base::Decimal((*s).max(*t)))
                        }
                    }
                    _ => return Err(self.wrong(name, &[&x.ty, &y.ty], span)),
                };
                let p = match name {
                    "checked_add" => Prim::CheckedAdd,
                    "checked_sub" => Prim::CheckedSub,
                    _ => Prim::CheckedMul,
                };
                let ty = self.option_ty(ty);
                Ok(self.prim(p, vec![x.tm, y.tm], ty))
            }
            "checked_div" | "checked_rem" => {
                let [a, b] = self.arity(name, es, 2, span)? else {
                    unreachable!()
                };
                let (x, y) = self.operands(a, b, None)?;
                if !(Self::is_integer(&x.ty) && x.ty == y.ty) {
                    return Err(self.wrong(name, &[&x.ty, &y.ty], span));
                }
                let p = if name == "checked_div" {
                    Prim::CheckedDiv
                } else {
                    Prim::CheckedRem
                };
                let ty = self.option_ty(x.ty.clone());
                Ok(self.prim(p, vec![x.tm, y.tm], ty))
            }
            "checked_neg" => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.synth(a)?;
                if !Self::is_integer(&x.ty) {
                    return Err(self.wrong(name, &[&x.ty], span));
                }
                let ty = self.option_ty(x.ty.clone());
                Ok(self.prim(Prim::CheckedNeg, vec![x.tm], ty))
            }
            _ => return Ok(None),
        };
        out.map(Some)
    }

    fn decimal_call(&mut self, name: &str, es: &[Expr], span: Span) -> R<Option<Out>> {
        let i64t = Ty::Base(Base::I64);
        let out = match name {
            "div" => {
                let [a, b, u, r] = self.arity(name, es, 4, span)? else {
                    unreachable!()
                };
                let x = self.decimal_operand(a)?;
                let y = self.decimal_operand(b)?;
                if !(Self::is_decimal(&x.ty) && Self::is_decimal(&y.ty)) {
                    return Err(self.wrong(name, &[&x.ty, &y.ty], span));
                }
                let scale = self.constant_int(u)?;
                if scale > 38 {
                    return Err(diag(u.span, "a decimal scale is at most 38"));
                }
                let ut = self.check(u, &i64t)?;
                let rt = self.check(r, &self.named_ty("Rounding"))?;
                Ok(self.prim(
                    Prim::DecimalDiv,
                    vec![x.tm, y.tm, ut.tm, rt.tm],
                    Ty::Base(Base::Decimal(scale as u8)),
                ))
            }
            "round" | "rescale" => {
                let n = if name == "round" { 3 } else { 2 };
                let args = self.arity(name, es, n, span)?;
                let x = self.synth(&args[0])?;
                let Ty::Base(Base::Decimal(s)) = x.ty else {
                    return Err(self.wrong(name, &[&x.ty], span));
                };
                let scale = self.constant_int(&args[1])?;
                if scale > 38 {
                    return Err(diag(args[1].span, "a decimal scale is at most 38"));
                }
                if name == "round" && scale >= u64::from(s) {
                    return Err(diag(
                        args[1].span,
                        format!(
                            "`round` takes a scale below the operand's {s}; use `rescale` to widen"
                        ),
                    ));
                }
                if name == "rescale" && scale < u64::from(s) {
                    return Err(diag(
                        args[1].span,
                        format!(
                            "`rescale` takes a scale of at least the operand's {s}; use `round` to narrow"
                        ),
                    ));
                }
                let mut tms = vec![x.tm, self.check(&args[1], &i64t)?.tm];
                if name == "round" {
                    let rt = self.check(&args[2], &self.named_ty("Rounding"))?;
                    tms.push(rt.tm);
                }
                let p = if name == "round" {
                    Prim::Round
                } else {
                    Prim::Rescale
                };
                Ok(self.prim(p, tms, Ty::Base(Base::Decimal(scale as u8))))
            }
            "abs" => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.synth(a)?;
                if !self.is_num(&x.ty) {
                    return Err(self.wrong(name, &[&x.ty], span));
                }
                let ty = x.ty.clone();
                Ok(self.prim(Prim::Abs, vec![x.tm], ty))
            }
            _ => return Ok(None),
        };
        out.map(Some)
    }

    fn convert_call(&mut self, name: &str, es: &[Expr], span: Span) -> R<Option<Out>> {
        let i64t = Ty::Base(Base::I64);
        let out = match name {
            "i8" | "i16" | "i32" | "i64" | "checked_i8" | "checked_i16" | "checked_i32"
            | "checked_i64" => {
                let checked = name.starts_with("checked_");
                let target = Base::from_name(&name.trim_start_matches("checked_").to_uppercase())
                    .expect("an integer type");
                let x = match es {
                    [a] | [a, _] => self.synth(a)?,
                    _ => return Err(self.arity(name, es, 1, span).expect_err("wrong arity")),
                };
                let mut tms = vec![x.tm];
                match es {
                    [_] if Self::is_integer(&x.ty) => {}
                    [_, r] if Self::is_decimal(&x.ty) || Self::is_float(&x.ty) => {
                        tms.push(self.check(r, &self.named_ty("Rounding"))?.tm);
                    }
                    [_] if Self::is_decimal(&x.ty) || Self::is_float(&x.ty) => {
                        return Err(diag(
                            span,
                            format!(
                                "`{name}` of {} takes a rounding: `{name}(x, HalfEven)`",
                                self.show(&x.ty)
                            ),
                        ));
                    }
                    _ => return Err(self.wrong(name, &[&x.ty], span)),
                }
                let ty = if checked {
                    self.option_ty(Ty::Base(target))
                } else {
                    Ty::Base(target)
                };
                Ok(self.prim(Prim::Convert { checked }, tms, ty))
            }
            "decimal" | "checked_decimal" => {
                let checked = name == "checked_decimal";
                let (x, s, r) = match es {
                    [x, s] => (x, s, None),
                    [x, s, r] => (x, s, Some(r)),
                    _ => return Err(self.arity(name, es, 2, span).expect_err("wrong arity")),
                };
                let xo = self.synth(x)?;
                let scale = self.constant_int(s)?;
                if scale > 38 {
                    return Err(diag(s.span, "a decimal scale is at most 38"));
                }
                let mut tms = vec![xo.tm, self.check(s, &i64t)?.tm];
                match r {
                    None if Self::is_integer(&xo.ty) => {}
                    Some(r) if Self::is_float(&xo.ty) => {
                        tms.push(self.check(r, &self.named_ty("Rounding"))?.tm);
                    }
                    _ => return Err(self.wrong(name, &[&xo.ty], span)),
                }
                let t = Ty::Base(Base::Decimal(scale as u8));
                let ty = if checked { self.option_ty(t) } else { t };
                Ok(self.prim(Prim::Convert { checked }, tms, ty))
            }
            "f32" | "f64" | "checked_f32" => {
                let checked = name == "checked_f32";
                let target = if name.ends_with("32") {
                    Base::F32
                } else {
                    Base::F64
                };
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.synth(a)?;
                let ok = Self::is_integer(&x.ty)
                    || Self::is_decimal(&x.ty)
                    || (Self::is_float(&x.ty) && x.ty != Ty::Base(target));
                if !ok {
                    return Err(self.wrong(name, &[&x.ty], span));
                }
                let ty = if checked {
                    self.option_ty(Ty::Base(target))
                } else {
                    Ty::Base(target)
                };
                Ok(self.prim(Prim::Convert { checked }, vec![x.tm], ty))
            }
            "unwrap" => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.synth(a)?;
                let base = match &x.ty {
                    Ty::Data { def, .. } => match &self.schema.ty(*def).kind {
                        TypeDefKind::Newtype { base, .. } => base.clone(),
                        _ => return Err(self.wrong(name, &[&x.ty], span)),
                    },
                    _ => return Err(self.wrong(name, &[&x.ty], span)),
                };
                Ok(self.prim(Prim::Unwrap, vec![x.tm], base))
            }
            "value" => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.synth(a)?;
                let Ty::Key { rel, .. } = x.ty else {
                    return Err(self.wrong(name, &[&x.ty], span));
                };
                let ty = self.key_value_ty(rel).map_err(|e| diag(span, e))?;
                Ok(self.prim(Prim::KeyValue, vec![x.tm], ty))
            }
            _ => return Ok(None),
        };
        out.map(Some)
    }

    fn string_call(&mut self, name: &str, es: &[Expr], span: Span) -> R<Option<Out>> {
        let text = Ty::Base(Base::Text);
        let out = match name {
            "starts_with" | "ends_with" | "contains" => {
                let [a, b] = self.arity(name, es, 2, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::Text)?;
                let y = self.base_arg(b, Base::Text)?;
                let p = match name {
                    "starts_with" => Prim::StartsWith,
                    "ends_with" => Prim::EndsWith,
                    _ => Prim::Contains,
                };
                let bt = self.bool_ty();
                Ok(self.prim(p, vec![x, y], bt))
            }
            "matches" => {
                let [a, b] = self.arity(name, es, 2, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::Text)?;
                let re = self.constant_text(b)?;
                if let Err(e) = regex_syntax::Parser::new().parse(&re) {
                    return Err(diag(
                        b.span,
                        format!("`{re}` is not a regular expression: {e}"),
                    ));
                }
                let y = self.base_arg(b, Base::Text)?;
                let bt = self.bool_ty();
                Ok(self.prim(Prim::Matches, vec![x, y], bt))
            }
            "lower" | "upper" | "fold" => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::Text)?;
                let p = match name {
                    "lower" => Prim::Lower,
                    "upper" => Prim::Upper,
                    _ => Prim::Fold,
                };
                Ok(self.prim(p, vec![x], text))
            }
            "collate" => {
                let [a, b] = self.arity(name, es, 2, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::Text)?;
                let c = self.constant_text(b)?;
                crate::collation::parse(&c).map_err(|m| diag(b.span, m))?;
                let y = self.base_arg(b, Base::Text)?;
                Ok(self.prim(Prim::Collate, vec![x, y], Ty::Base(Base::SortKey)))
            }
            _ => return Ok(None),
        };
        out.map(Some)
    }

    fn time_call(&mut self, name: &str, es: &[Expr], span: Span) -> R<Option<Out>> {
        let out = match name {
            "micros" | "millis" | "seconds" | "minutes" | "hours" | "days" | "weeks" | "months"
            | "years" => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::I64)?;
                let (p, ty) = match name {
                    "micros" => (Prim::Micros, Base::Duration),
                    "millis" => (Prim::Millis, Base::Duration),
                    "seconds" => (Prim::Seconds, Base::Duration),
                    "minutes" => (Prim::Minutes, Base::Duration),
                    "hours" => (Prim::Hours, Base::Duration),
                    "days" => (Prim::Days, Base::Period),
                    "weeks" => (Prim::Weeks, Base::Period),
                    "months" => (Prim::Months, Base::Period),
                    _ => (Prim::Years, Base::Period),
                };
                Ok(self.prim(p, vec![x], Ty::Base(ty)))
            }
            "add_period" => {
                let [a, b, o] = self.arity(name, es, 3, span)? else {
                    unreachable!()
                };
                let x = self.synth(a)?;
                if !matches!(
                    x.ty,
                    Ty::Base(Base::Date | Base::DateTime | Base::ZonedDateTime)
                ) {
                    return Err(self.wrong(name, &[&x.ty], span));
                }
                let p = self.base_arg(b, Base::Period)?;
                let reject = self.constant_overflow(o)?;
                let ot = self.check(o, &self.named_ty("Overflow"))?;
                let ty = if reject {
                    self.option_ty(x.ty.clone())
                } else {
                    x.ty.clone()
                };
                Ok(self.prim(Prim::AddPeriod, vec![x.tm, p, ot.tm], ty))
            }
            "zoned" => {
                let [a, b, o] = self.arity(name, es, 3, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::DateTime)?;
                let z = self.base_arg(b, Base::Text)?;
                let reject = self.constant_overflow(o)?;
                let ot = self.check(o, &self.named_ty("Overflow"))?;
                let t = Ty::Base(Base::ZonedDateTime);
                let ty = if reject { self.option_ty(t) } else { t };
                Ok(self.prim(Prim::Zoned, vec![x, z, ot.tm], ty))
            }
            "in_zone" => {
                let [a, b] = self.arity(name, es, 2, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::Instant)?;
                let z = self.base_arg(b, Base::Text)?;
                Ok(self.prim(Prim::InZone, vec![x, z], Ty::Base(Base::ZonedDateTime)))
            }
            "to_instant" => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::ZonedDateTime)?;
                Ok(self.prim(Prim::ToInstant, vec![x], Ty::Base(Base::Instant)))
            }
            _ => return Ok(None),
        };
        out.map(Some)
    }

    fn change_call(&mut self, name: &str, es: &[Expr], span: Span) -> R<Option<Out>> {
        let text = Ty::Base(Base::Text);
        let out = match name {
            "tag" => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.synth(a)?;
                let ty = match &x.ty {
                    Ty::Data { def, args, .. }
                        if matches!(self.schema.ty(*def).kind, TypeDefKind::Sum { .. }) =>
                    {
                        Ty::Tag(Box::new(Ty::Data {
                            def: *def,
                            args: args.clone(),
                            ctors: None,
                        }))
                    }
                    _ => {
                        return Err(diag(
                            span,
                            format!(
                                "`tag` takes a value of a declared sum, found {}",
                                self.show(&x.ty)
                            ),
                        ));
                    }
                };
                Ok(self.out(Kind::Tag(x.tm), ty))
            }
            "print" => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.synth(a)?;
                if x.ty.mentions_fn() {
                    return Err(diag(span, "`print` takes a value, not a function"));
                }
                Ok(self.prim(Prim::Print, vec![x.tm], text))
            }
            "inserted" | "deleted" | "prior" => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                if self.ctx != Context::Constraint {
                    return Err(diag(
                        span,
                        format!("`{name}` reads the change of a statement, only in a constraint"),
                    ));
                }
                let (rel, ctors) = self.relation_ref_expr(a)?;
                let base = self.rel_out(rel, ctors.clone());
                let t = match name {
                    "inserted" => Transition::Inserted,
                    "deleted" => Transition::Deleted,
                    _ => Transition::Prior,
                };
                let mut o = self.out(Kind::Transition(t, rel, ctors), base.ty);
                o.rows = Some(rel);
                Ok(o)
            }
            _ => return Ok(None),
        };
        out.map(Some)
    }

    /// `read_csv(path, T)` and the other file scans, each read at the declared row type `T`.
    pub(super) fn file_scan(
        &mut self,
        name: &str,
        format: FileFormat,
        es: &[Expr],
        pins: Option<&ast::Version>,
        span: Span,
    ) -> R<Out> {
        if self.ctx != Context::Adhoc && !self.reject_ok {
            return Err(diag(
                span,
                format!(
                    "`{name}` reads files only in ad hoc text and in the source query of an insert"
                ),
            ));
        }
        let [path, row] = self.arity(name, es, 2, span)? else {
            unreachable!()
        };
        let row = self.row_type_arg(name, row)?;
        let path = self.base_arg(path, Base::Text)?;
        let pins = pins.map(|v| v.number);
        Ok(self.prim(
            Prim::ReadFile { format, pins },
            vec![path],
            Ty::Set(Box::new(row)),
        ))
    }

    /// The row type a file scan names: a declared record or sum type with no parameters.
    fn row_type_arg(&self, name: &str, e: &Expr) -> R<Ty> {
        let refused = || {
            diag(
                e.span,
                format!("`{name}` takes a declared record or sum type as its row type"),
            )
        };
        let ExprKind::Ctor {
            path,
            ctor: None,
            record: None,
        } = &e.kind
        else {
            return Err(refused());
        };
        let def = match self.lookup_type(path) {
            Ok(TypeName::Decl(d)) => d,
            Ok(_) => return Err(refused()),
            Err(m) => return Err(diag(e.span, m)),
        };
        self.note(path.name.span, Ref::Type(def));
        let td = self.schema.ty(def);
        if matches!(td.kind, TypeDefKind::Newtype { .. }) || !td.params.is_empty() {
            return Err(refused());
        }
        Ok(Ty::Data {
            def,
            args: Vec::new(),
            ctors: None,
        })
    }

    /// `r`, `r.C`, `r.(C | D)` or `r.S` written as an expression.
    fn relation_ref_expr(&self, e: &Expr) -> R<(RelId, Option<CtorSet>)> {
        let (q, sel) = match &e.kind {
            ExprKind::Name(q) => (q, None),
            ExprKind::Select(x, sel) => match &x.kind {
                ExprKind::Name(q) => (q, Some(sel)),
                _ => return Err(diag(e.span, "expected a relation")),
            },
            _ => return Err(diag(e.span, "expected a relation")),
        };
        let Ok(ValueDef::Relation(rel)) = self.lookup_value(q) else {
            return Err(diag(e.span, format!("`{}` is not a relation", q.name.text)));
        };
        let ctors = match sel {
            None => None,
            Some(s) => {
                let set = self.relation_sel(rel, s).map_err(|m| diag(e.span, m))?;
                self.schema
                    .normalize_ctors(self.schema.relation(rel).elem_def, set)
            }
        };
        Ok((rel, ctors))
    }

    fn span_arg(&self, ty: &Ty) -> Option<Ty> {
        match ty {
            Ty::Data { def, args, .. } if *def == self.schema.prelude_type("Span") => {
                args.first().cloned()
            }
            _ => None,
        }
    }

    /// Whether an argument is visibly a span, before checking it.
    fn span_arg_expr(&self, e: &Expr) -> Option<bool> {
        match &e.kind {
            ExprKind::Call { callee, .. }
                if callee.module.is_none() && callee.name.text == "span" =>
            {
                Some(true)
            }
            _ => None,
        }
    }

    pub(super) fn type_call(
        &mut self,
        t: &ast::Type,
        func: &ast::Ident,
        args: &ast::CallArgs,
        span: Span,
    ) -> R<Out> {
        let ty = self.resolve_type(t)?;
        let ast::CallArgs::List(es) = args else {
            return Err(diag(span, "`T.parse` takes one text"));
        };
        match func.text.as_str() {
            "parse" => {
                let [a] = self.arity("parse", es, 1, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::Text)?;
                let ty = self.option_ty(ty);
                Ok(self.prim(Prim::Parse, vec![x], ty))
            }
            other => Err(diag(
                func.span,
                format!("the prelude defines no `T.{other}`"),
            )),
        }
    }
}
