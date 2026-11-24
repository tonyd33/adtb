use super::call::Callee;
use super::*;
use crate::ast::{BinOp, Expr, ExprKind, Lit};
use crate::kernel::{FileFormat, Prim, StatedVersion, Transition};

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

    fn named_ty(def: TypeId) -> Ty {
        Ty::Data {
            def,
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

    /// Check that `e` is an `Overflow` constructor written in place.
    fn constant_overflow(&self, e: &Expr) -> R<()> {
        if let ExprKind::Ctor {
            path,
            ctor,
            record: None,
        } = &e.kind
        {
            let name = ctor.as_ref().unwrap_or(&path.name);
            if matches!(name.text.as_str(), "Clamp" | "Reject") {
                return Ok(());
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

    pub(super) fn prelude_call(
        &mut self,
        f: PreludeFn,
        es: &[Expr],
        pins: Option<&ast::Version>,
        span: Span,
    ) -> R<Out> {
        match f {
            PreludeFn::Span(g) => self.span_call(g, es, span),
            PreludeFn::Text(g) => self.text_call(g, es, span),
            PreludeFn::Checked(g) => self.checked_call(g, es, span),
            PreludeFn::Decimal(g) => self.decimal_call(g, es, span),
            PreludeFn::Convert(g) => self.convert_call(g, es, span),
            PreludeFn::String(g) => self.string_call(g, es, span),
            PreludeFn::Time(g) => self.time_call(g, es, span),
            PreludeFn::Change(g) => self.change_call(g, es, span),
            PreludeFn::Aggregate(_) => {
                let name = f.name();
                Err(diag(
                    span,
                    format!("`{name}` is an aggregate: `{name}(e for x in q)`"),
                ))
            }
            PreludeFn::File(g) => self.file_scan(g, es, pins, span),
        }
    }

    fn span_call(&mut self, f: SpanFn, es: &[Expr], span: Span) -> R<Out> {
        let name = f.name();
        match f {
            SpanFn::Span => {
                let [a, b] = self.arity(name, es, 2, span)? else {
                    unreachable!()
                };
                let (x, y) = self.operands(a, b, None)?;
                let t = self
                    .join(&x.ty, &y.ty, span)?
                    .ok_or_else(|| self.wrong(name, &[&x.ty, &y.ty], span))?;
                if !self.ordered(&t, span)? {
                    return Err(diag(
                        span,
                        format!("`Span a` needs an ordered `a`, found {}", self.show(&t)),
                    ));
                }
                self.prim(Prim::Span, vec![x.tm, y.tm], None, span)
            }
            SpanFn::Start | SpanFn::End => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let o = self.synth(a)?;
                if self.span_arg(&o.ty).is_none() {
                    return Err(self.wrong(name, &[&o.ty], span));
                }
                let p = if f == SpanFn::Start {
                    Prim::Start
                } else {
                    Prim::End
                };
                self.prim(p, vec![o.tm], None, span)
            }
            SpanFn::Overlaps | SpanFn::Covers => {
                let [a, b] = self.arity(name, es, 2, span)? else {
                    unreachable!()
                };
                let x = self.synth(a)?;
                let t = self
                    .span_arg(&x.ty)
                    .ok_or_else(|| self.wrong(name, &[&x.ty], span))?;
                let y = if f == SpanFn::Covers && !self.calls_span(b) {
                    let o = self.synth(b)?;
                    if self.span_arg(&o.ty).is_some() {
                        self.subsume(o, &x.ty, b.span)?
                    } else {
                        self.subsume(o, &t, b.span)?
                    }
                } else {
                    self.check(b, &x.ty)?
                };
                let p = if f == SpanFn::Covers {
                    Prim::Covers
                } else {
                    Prim::Overlaps
                };
                self.prim(p, vec![x.tm, y.tm], None, span)
            }
        }
    }

    fn text_call(&mut self, f: TextFn, es: &[Expr], span: Span) -> R<Out> {
        let name = f.name();
        let text = Ty::Base(Base::Text);
        match f {
            TextFn::Uuid => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.check(a, &text)?;
                self.prim(Prim::ParseUuid, vec![x.tm], None, span)
            }
            TextFn::Text => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.synth(a)?;
                self.prim(Prim::ToText, vec![x.tm], None, span)
                    .map_err(|_| self.wrong(name, &[&x.ty], span))
            }
            TextFn::BytesLen => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.synth(a)?;
                self.prim(Prim::BytesLen, vec![x.tm], None, span)
                    .map_err(|_| self.wrong(name, &[&x.ty], span))
            }
            TextFn::ScalarLen | TextFn::GraphemeLen => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::Text)?;
                let p = if f == TextFn::ScalarLen {
                    Prim::ScalarLen
                } else {
                    Prim::GraphemeLen
                };
                self.prim(p, vec![x], None, span)
            }
            TextFn::Utf8 => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::Bytes)?;
                self.prim(Prim::Utf8, vec![x], None, span)
            }
            TextFn::Bytes => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::Text)?;
                self.prim(Prim::Bytes, vec![x], None, span)
            }
        }
    }

    fn checked_call(&mut self, f: CheckedFn, es: &[Expr], span: Span) -> R<Out> {
        let name = f.name();
        match f {
            CheckedFn::Add | CheckedFn::Sub | CheckedFn::Mul => {
                let [a, b] = self.arity(name, es, 2, span)? else {
                    unreachable!()
                };
                let op = match f {
                    CheckedFn::Add => BinOp::Add,
                    CheckedFn::Sub => BinOp::Sub,
                    _ => BinOp::Mul,
                };
                let (x, y) = self.operator_operands(op, a, b, None)?;
                if let (Ty::Base(Base::Decimal(s)), Ty::Base(Base::Decimal(t))) = (&x.ty, &y.ty)
                    && f == CheckedFn::Mul
                    && u32::from(*s) + u32::from(*t) > 38
                {
                    return Err(diag(span, "the product's scale is above 38"));
                }
                let p = match f {
                    CheckedFn::Add => Prim::CheckedAdd,
                    CheckedFn::Sub => Prim::CheckedSub,
                    _ => Prim::CheckedMul,
                };
                self.prim(p, vec![x.tm, y.tm], None, span)
                    .map_err(|_| self.wrong(name, &[&x.ty, &y.ty], span))
            }
            CheckedFn::Div | CheckedFn::Rem => {
                let [a, b] = self.arity(name, es, 2, span)? else {
                    unreachable!()
                };
                let (x, y) = self.operands(a, b, None)?;
                let p = if f == CheckedFn::Div {
                    Prim::CheckedDiv
                } else {
                    Prim::CheckedRem
                };
                self.prim(p, vec![x.tm, y.tm], None, span)
                    .map_err(|_| self.wrong(name, &[&x.ty, &y.ty], span))
            }
            CheckedFn::Neg => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.synth(a)?;
                self.prim(Prim::CheckedNeg, vec![x.tm], None, span)
                    .map_err(|_| self.wrong(name, &[&x.ty], span))
            }
        }
    }

    fn decimal_call(&mut self, f: DecimalFn, es: &[Expr], span: Span) -> R<Out> {
        let name = f.name();
        let i64t = Ty::Base(Base::I64);
        match f {
            DecimalFn::Div => {
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
                let rt = self.check(r, &Self::named_ty(self.schema.prelude().rounding()))?;
                self.prim(Prim::DecimalDiv, vec![x.tm, y.tm, ut.tm, rt.tm], None, span)
            }
            DecimalFn::Round | DecimalFn::Rescale => {
                let n = if f == DecimalFn::Round { 3 } else { 2 };
                let args = self.arity(name, es, n, span)?;
                let x = self.synth(&args[0])?;
                let Ty::Base(Base::Decimal(s)) = x.ty else {
                    return Err(self.wrong(name, &[&x.ty], span));
                };
                let scale = self.constant_int(&args[1])?;
                if scale > 38 {
                    return Err(diag(args[1].span, "a decimal scale is at most 38"));
                }
                if f == DecimalFn::Round && scale >= u64::from(s) {
                    return Err(diag(
                        args[1].span,
                        format!(
                            "`round` takes a scale below the operand's {s}; use `rescale` to widen"
                        ),
                    ));
                }
                if f == DecimalFn::Rescale && scale < u64::from(s) {
                    return Err(diag(
                        args[1].span,
                        format!(
                            "`rescale` takes a scale of at least the operand's {s}; use `round` to narrow"
                        ),
                    ));
                }
                let mut tms = vec![x.tm, self.check(&args[1], &i64t)?.tm];
                if f == DecimalFn::Round {
                    let rt =
                        self.check(&args[2], &Self::named_ty(self.schema.prelude().rounding()))?;
                    tms.push(rt.tm);
                }
                let p = if f == DecimalFn::Round {
                    Prim::Round
                } else {
                    Prim::Rescale
                };
                self.prim(p, tms, None, span)
            }
            DecimalFn::Abs => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.synth(a)?;
                if !self.is_num(&x.ty) {
                    return Err(self.wrong(name, &[&x.ty], span));
                }
                self.prim(Prim::Abs, vec![x.tm], None, span)
            }
        }
    }

    fn convert_call(&mut self, f: ConvertFn, es: &[Expr], span: Span) -> R<Out> {
        let name = f.name();
        let i64t = Ty::Base(Base::I64);
        match f {
            ConvertFn::I8
            | ConvertFn::I16
            | ConvertFn::I32
            | ConvertFn::I64
            | ConvertFn::CheckedI8
            | ConvertFn::CheckedI16
            | ConvertFn::CheckedI32
            | ConvertFn::CheckedI64 => {
                let (checked, target) = match f {
                    ConvertFn::I8 => (false, Base::I8),
                    ConvertFn::I16 => (false, Base::I16),
                    ConvertFn::I32 => (false, Base::I32),
                    ConvertFn::I64 => (false, Base::I64),
                    ConvertFn::CheckedI8 => (true, Base::I8),
                    ConvertFn::CheckedI16 => (true, Base::I16),
                    ConvertFn::CheckedI32 => (true, Base::I32),
                    _ => (true, Base::I64),
                };
                let x = match es {
                    [a] | [a, _] => self.synth(a)?,
                    _ => return Err(self.arity(name, es, 1, span).expect_err("wrong arity")),
                };
                let mut tms = vec![x.tm];
                match es {
                    [_] if Self::is_integer(&x.ty) => {}
                    [_, r] if Self::is_decimal(&x.ty) || Self::is_float(&x.ty) => {
                        tms.push(
                            self.check(r, &Self::named_ty(self.schema.prelude().rounding()))?
                                .tm,
                        );
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
                self.prim(Prim::Convert { checked }, tms, Some(ty), span)
            }
            ConvertFn::Decimal | ConvertFn::CheckedDecimal => {
                let checked = f == ConvertFn::CheckedDecimal;
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
                        tms.push(
                            self.check(r, &Self::named_ty(self.schema.prelude().rounding()))?
                                .tm,
                        );
                    }
                    _ => return Err(self.wrong(name, &[&xo.ty], span)),
                }
                let t = Ty::Base(Base::Decimal(scale as u8));
                let ty = if checked { self.option_ty(t) } else { t };
                self.prim(Prim::Convert { checked }, tms, Some(ty), span)
            }
            ConvertFn::F32 | ConvertFn::F64 | ConvertFn::CheckedF32 => {
                let (checked, target) = match f {
                    ConvertFn::F32 => (false, Base::F32),
                    ConvertFn::F64 => (false, Base::F64),
                    _ => (true, Base::F32),
                };
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.synth(a)?;
                let ty = if checked {
                    self.option_ty(Ty::Base(target))
                } else {
                    Ty::Base(target)
                };
                self.prim(Prim::Convert { checked }, vec![x.tm], Some(ty), span)
                    .map_err(|_| self.wrong(name, &[&x.ty], span))
            }
            ConvertFn::Unwrap => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.synth(a)?;
                self.prim(Prim::Unwrap, vec![x.tm], None, span)
                    .map_err(|_| self.wrong(name, &[&x.ty], span))
            }
            ConvertFn::Value => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.synth(a)?;
                if !matches!(x.ty, Ty::Key { .. }) {
                    return Err(self.wrong(name, &[&x.ty], span));
                }
                self.prim(Prim::KeyValue, vec![x.tm], None, span)
            }
        }
    }

    fn string_call(&mut self, f: StringFn, es: &[Expr], span: Span) -> R<Out> {
        let name = f.name();
        match f {
            StringFn::StartsWith | StringFn::EndsWith | StringFn::Contains => {
                let [a, b] = self.arity(name, es, 2, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::Text)?;
                let y = self.base_arg(b, Base::Text)?;
                let p = match f {
                    StringFn::StartsWith => Prim::StartsWith,
                    StringFn::EndsWith => Prim::EndsWith,
                    _ => Prim::Contains,
                };
                self.prim(p, vec![x, y], None, span)
            }
            StringFn::Matches => {
                let [a, b] = self.arity(name, es, 2, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::Text)?;
                let re = self.constant_text(b)?;
                match crate::pattern::Pattern::new(&re) {
                    Ok(_) => {}
                    Err(e @ crate::pattern::Refusal::TooBig) => {
                        return Err(diag(b.span, format!("`{re}`: {e}")));
                    }
                    Err(e) => {
                        return Err(diag(
                            b.span,
                            format!("`{re}` is not a regular expression: {e}"),
                        ));
                    }
                }
                let y = self.base_arg(b, Base::Text)?;
                self.prim(Prim::Matches, vec![x, y], None, span)
            }
            StringFn::Lower | StringFn::Upper | StringFn::Fold => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::Text)?;
                let p = match f {
                    StringFn::Lower => Prim::Lower,
                    StringFn::Upper => Prim::Upper,
                    _ => Prim::Fold,
                };
                self.prim(p, vec![x], None, span)
            }
            StringFn::Collate => {
                let [a, b] = self.arity(name, es, 2, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::Text)?;
                let c = self.constant_text(b)?;
                crate::collation::parse(&c).map_err(|m| diag(b.span, m))?;
                let y = self.base_arg(b, Base::Text)?;
                self.prim(Prim::Collate, vec![x, y], None, span)
            }
        }
    }

    fn time_call(&mut self, f: TimeFn, es: &[Expr], span: Span) -> R<Out> {
        let name = f.name();
        match f {
            TimeFn::Micros
            | TimeFn::Millis
            | TimeFn::Seconds
            | TimeFn::Minutes
            | TimeFn::Hours
            | TimeFn::Days
            | TimeFn::Weeks
            | TimeFn::Months
            | TimeFn::Years => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::I64)?;
                let p = match f {
                    TimeFn::Micros => Prim::Micros,
                    TimeFn::Millis => Prim::Millis,
                    TimeFn::Seconds => Prim::Seconds,
                    TimeFn::Minutes => Prim::Minutes,
                    TimeFn::Hours => Prim::Hours,
                    TimeFn::Days => Prim::Days,
                    TimeFn::Weeks => Prim::Weeks,
                    TimeFn::Months => Prim::Months,
                    _ => Prim::Years,
                };
                self.prim(p, vec![x], None, span)
            }
            TimeFn::AddPeriod => {
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
                self.constant_overflow(o)?;
                let ot = self.check(o, &Self::named_ty(self.schema.prelude().overflow()))?;
                self.prim(Prim::AddPeriod, vec![x.tm, p, ot.tm], None, span)
            }
            TimeFn::Zoned => {
                let [a, b, o] = self.arity(name, es, 3, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::DateTime)?;
                let z = self.base_arg(b, Base::Text)?;
                self.constant_overflow(o)?;
                let ot = self.check(o, &Self::named_ty(self.schema.prelude().overflow()))?;
                self.prim(Prim::Zoned, vec![x, z, ot.tm], None, span)
            }
            TimeFn::InZone => {
                let [a, b] = self.arity(name, es, 2, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::Instant)?;
                let z = self.base_arg(b, Base::Text)?;
                self.prim(Prim::InZone, vec![x, z], None, span)
            }
            TimeFn::ToInstant => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::ZonedDateTime)?;
                self.prim(Prim::ToInstant, vec![x], None, span)
            }
        }
    }

    fn change_call(&mut self, f: ChangeFn, es: &[Expr], span: Span) -> R<Out> {
        let name = f.name();
        match f {
            ChangeFn::Tag => {
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
                Ok(Out::of(self.b().tag(x.tm), ty))
            }
            ChangeFn::Print => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let x = self.synth(a)?;
                if x.ty.mentions_fn() {
                    return Err(diag(span, "`print` takes a value, not a function"));
                }
                self.prim(Prim::Print, vec![x.tm], None, span)
            }
            ChangeFn::Inserted | ChangeFn::Deleted | ChangeFn::Prior => {
                let [a] = self.arity(name, es, 1, span)? else {
                    unreachable!()
                };
                let (rel, ctors) = self.relation_ref_expr(a)?;
                let base = self.rel_out(rel, ctors.clone());
                let t = match f {
                    ChangeFn::Inserted => Transition::Inserted,
                    ChangeFn::Deleted => Transition::Deleted,
                    _ => Transition::Prior,
                };
                let provenance = match t {
                    Transition::Inserted => Provenance::Snapshot,
                    Transition::Deleted | Transition::Prior => Provenance::Built,
                };
                let mut o = Out::of(self.b().transition(t, rel, ctors), base.ty);
                o.rows = Some(rel);
                o.provenance = provenance;
                Ok(o)
            }
        }
    }

    /// `read_csv(path, T)` and the other file scans, each read at the declared row type `T`.
    fn file_scan(
        &mut self,
        f: FileFn,
        es: &[Expr],
        pins: Option<&ast::Version>,
        span: Span,
    ) -> R<Out> {
        let name = f.name();
        let format = match f {
            FileFn::Csv => FileFormat::Csv,
            FileFn::Ndjson => FileFormat::Ndjson,
            FileFn::Parquet => FileFormat::Parquet,
            FileFn::Arrow => FileFormat::Arrow,
        };
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
        let pins = pins.map(StatedVersion::of);
        self.prim(
            Prim::ReadFile { format, pins },
            vec![path],
            Some(Ty::Set(Box::new(row))),
            span,
        )
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
            Err(m) => return Err(m.at(e.span)),
        };
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
            Ty::Data { def, args, .. } if *def == self.schema.prelude().span() => {
                args.first().cloned()
            }
            _ => None,
        }
    }

    /// Returns whether `e` is a call of the prelude's `span`, before checking it.
    fn calls_span(&self, e: &Expr) -> bool {
        matches!(&e.kind, ExprKind::Call { callee, .. }
            if matches!(self.callee(callee), Callee::Prelude(PreludeFn::Span(SpanFn::Span))))
    }

    pub(super) fn type_call(
        &mut self,
        t: &ast::Type,
        func: &ast::Ident,
        args: &ast::CallArgs,
        span: Span,
    ) -> R<Out> {
        let ty = self.resolve_type(
            t,
            FnTypes::Refused("`T.parse` reads a value type, never a function type"),
        )?;
        let ast::CallArgs::List(es) = args else {
            return Err(diag(span, "`T.parse` takes one text"));
        };
        match func.text.as_str() {
            "parse" => {
                let [a] = self.arity("parse", es, 1, span)? else {
                    unreachable!()
                };
                let x = self.base_arg(a, Base::Text)?;
                self.schema
                    .holds_no_ahead_key(&ty)
                    .map_err(|e| diag(span, e))?;
                let ty = self.option_ty(ty);
                self.prim(Prim::Parse, vec![x], Some(ty), span)
            }
            other => Err(diag(
                func.span,
                format!("the prelude defines no `T.{other}`"),
            )),
        }
    }
}
