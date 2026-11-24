use super::sums::forms_of_one_class;
use super::ty::{join, subtype};
use crate::kernel::{Const, Kind, Prim, Terms, Tm};
use crate::schema::prelude::ctor;
use crate::schema::{Schema, TypeDefKind};
use crate::types::{Base, Bound, Ty, TypeId};

/// The bounds of the type parameters the types of a body may name, by position; `Unknown` where
/// the body is not known, as when one node is checked alone.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Bounds<'a> {
    Declared(&'a [(String, Vec<Bound>)]),
    Unknown,
}

impl Bounds<'_> {
    /// Returns whether type parameter `i` has bound `b`; under `Unknown`, every bound holds.
    pub(crate) fn has(self, i: u32, b: Bound) -> bool {
        match self {
            Bounds::Declared(ps) => ps.get(i as usize).is_some_and(|(_, bs)| bs.contains(&b)),
            Bounds::Unknown => true,
        }
    }
}

/// Returns the type primitive `p` gives over `args`, or why its operands are not its own.
/// `target` names the result of a `Convert`, `Wrap`, `Parse` or `ReadFile`, whose operands do
/// not fix it; the other primitives ignore it.
pub(crate) fn prim_ty(
    schema: &Schema,
    terms: &Terms,
    bounds: Bounds<'_>,
    p: &Prim,
    args: &[Tm],
    target: Option<&Ty>,
) -> Result<Ty, String> {
    Signature {
        schema,
        terms,
        bounds,
        p,
        args,
    }
    .result(target)
}

struct Signature<'a> {
    schema: &'a Schema,
    terms: &'a Terms,
    bounds: Bounds<'a>,
    p: &'a Prim,
    args: &'a [Tm],
}

impl Signature<'_> {
    fn result(&self, target: Option<&Ty>) -> Result<Ty, String> {
        match self.p {
            Prim::Add
            | Prim::Sub
            | Prim::Mul
            | Prim::Div
            | Prim::Rem
            | Prim::Neg
            | Prim::Abs
            | Prim::Eq
            | Prim::Ne
            | Prim::Lt
            | Prim::Le
            | Prim::Gt
            | Prim::Ge
            | Prim::Concat
            | Prim::CheckedAdd
            | Prim::CheckedSub
            | Prim::CheckedMul
            | Prim::CheckedDiv
            | Prim::CheckedRem
            | Prim::CheckedNeg
            | Prim::DecimalDiv
            | Prim::Round
            | Prim::Rescale => self.numeric(),
            Prim::Convert { .. }
            | Prim::Wrap
            | Prim::Unwrap
            | Prim::KeyValue
            | Prim::ParseUuid
            | Prim::ToText
            | Prim::Print
            | Prim::Parse
            | Prim::ReadFile { .. } => self.value(target),
            Prim::BytesLen
            | Prim::ScalarLen
            | Prim::GraphemeLen
            | Prim::StartsWith
            | Prim::EndsWith
            | Prim::Contains
            | Prim::Matches
            | Prim::Lower
            | Prim::Upper
            | Prim::Fold
            | Prim::Collate
            | Prim::Utf8
            | Prim::Bytes => self.text(),
            Prim::Micros
            | Prim::Millis
            | Prim::Seconds
            | Prim::Minutes
            | Prim::Hours
            | Prim::Days
            | Prim::Weeks
            | Prim::Months
            | Prim::Years
            | Prim::AddPeriod
            | Prim::Zoned
            | Prim::InZone
            | Prim::ToInstant => self.time(),
            Prim::Span | Prim::Start | Prim::End | Prim::Overlaps | Prim::Covers => self.span(),
        }
    }

    fn numeric(&self) -> Result<Ty, String> {
        let p = self.p;
        match p {
            Prim::Add | Prim::Sub | Prim::Mul | Prim::Div | Prim::Rem => {
                let [a, b] = self.arity()?;
                self.arith(a, b)
            }
            Prim::Neg => {
                let [a] = self.arity()?;
                let ok = match a {
                    Ty::Base(b) => b.is_num() || matches!(b, Base::Duration | Base::Period),
                    t => self.is_num(t),
                };
                self.admits(ok, &[a])?;
                Ok(a.clone())
            }
            Prim::Abs => {
                let [a] = self.arity()?;
                self.admits(self.is_num(a), &[a])?;
                Ok(a.clone())
            }
            Prim::Eq | Prim::Ne | Prim::Lt | Prim::Le | Prim::Gt | Prim::Ge => {
                let [a, b] = self.arity()?;
                let decimals = is_decimal(a) && is_decimal(b);
                let joined = comparable(self.schema, a, b);
                self.admits(decimals || joined.is_some(), &[a, b])?;
                if let Some(j) = &joined {
                    expect(!j.mentions_fn(), || format!("`{p:?}` compares functions"))?;
                    if !matches!(p, Prim::Eq | Prim::Ne) {
                        self.ordered(j)?;
                    }
                }
                Ok(self.bool())
            }
            Prim::Concat => {
                let [a, b] = self.arity()?;
                self.admits(
                    a == b && matches!(a, Ty::Base(Base::Text | Base::Bytes)),
                    &[a, b],
                )?;
                Ok(a.clone())
            }
            Prim::CheckedAdd | Prim::CheckedSub | Prim::CheckedMul => {
                let [a, b] = self.arity()?;
                self.admits(is_integer(a) || is_decimal(a), &[a, b])?;
                let r = self.arith(a, b)?;
                Ok(self.option(r))
            }
            Prim::CheckedDiv | Prim::CheckedRem => {
                let [a, b] = self.arity()?;
                self.admits(is_integer(a) && a == b, &[a, b])?;
                Ok(self.option(a.clone()))
            }
            Prim::CheckedNeg => {
                let [a] = self.arity()?;
                self.admits(is_integer(a), &[a])?;
                Ok(self.option(a.clone()))
            }
            Prim::DecimalDiv => {
                let [a, b, _, r] = self.arity()?;
                self.admits(is_decimal(a) && is_decimal(b), &[a, b])?;
                self.rounding(r)?;
                Ok(Ty::Base(Base::Decimal(self.scale(2)?)))
            }
            Prim::Round => {
                let [a, _, r] = self.arity()?;
                let Ty::Base(Base::Decimal(s)) = a else {
                    return Err(self.refused(&[a]));
                };
                self.rounding(r)?;
                let u = self.scale(1)?;
                expect(u < *s, || {
                    format!("`Round` to scale {u} of a `Decimal {s}`")
                })?;
                Ok(Ty::Base(Base::Decimal(u)))
            }
            Prim::Rescale => {
                let [a, _] = self.arity()?;
                let Ty::Base(Base::Decimal(s)) = a else {
                    return Err(self.refused(&[a]));
                };
                let u = self.scale(1)?;
                expect(u >= *s, || {
                    format!("`Rescale` to scale {u} of a `Decimal {s}`")
                })?;
                Ok(Ty::Base(Base::Decimal(u)))
            }
            _ => Err(format!("`{p:?}` is not an arithmetic primitive")),
        }
    }

    fn value(&self, target: Option<&Ty>) -> Result<Ty, String> {
        let p = self.p;
        let text = Ty::Base(Base::Text);
        match p {
            Prim::Convert { checked } => self.convert(*checked, self.target(target)?),
            Prim::Wrap => {
                let [a] = self.arity()?;
                let t = self.target(target)?;
                let base = match t {
                    Ty::Data { def, .. } => self.newtype_base(*def),
                    _ => None,
                };
                self.admits(base.is_some_and(|b| subtype(self.schema, a, b)), &[a])?;
                Ok(t.clone())
            }
            Prim::Unwrap => {
                let [a] = self.arity()?;
                match a {
                    Ty::Data { def, .. } => self.newtype_base(*def).cloned(),
                    _ => None,
                }
                .ok_or_else(|| self.refused(&[a]))
            }
            Prim::KeyValue => {
                let [a] = self.arity()?;
                let Ty::Key { rel, .. } = a else {
                    return Err(self.refused(&[a]));
                };
                let rel = self.schema.relation_of(*rel)?;
                super::rel::key_value_ty(self.schema, rel)
            }
            Prim::ParseUuid => {
                self.operands(&[&text])?;
                Ok(data(self.schema.prelude().uuid()))
            }
            Prim::ToText => {
                let [a] = self.arity()?;
                let ok = match a {
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
                        *def == self.schema.prelude().uuid()
                            || self.newtype_base(*def) == Some(&text)
                    }
                    _ => false,
                };
                self.admits(ok, &[a])?;
                Ok(text)
            }
            Prim::Print => {
                let [a] = self.arity()?;
                self.admits(!a.mentions_fn(), &[a])?;
                Ok(text)
            }
            Prim::Parse => {
                self.operands(&[&text])?;
                let t = self.target(target)?;
                expect(self.option_of(t).is_some(), || {
                    format!("`Parse` is `{}`, not an `Option`", self.schema.show_ty(t))
                })?;
                Ok(t.clone())
            }
            Prim::ReadFile { .. } => {
                self.operands(&[&text])?;
                let t = self.target(target)?;
                let row = match t {
                    Ty::Set(row) => match row.as_ref() {
                        Ty::Data { def, args, .. } => {
                            args.is_empty() && self.newtype_base(*def).is_none()
                        }
                        _ => false,
                    },
                    _ => false,
                };
                expect(row, || {
                    format!(
                        "a file scan is `{}`, not a set of a declared type",
                        self.schema.show_ty(t)
                    )
                })?;
                Ok(t.clone())
            }
            _ => Err(format!("`{p:?}` is not a value primitive")),
        }
    }

    fn text(&self) -> Result<Ty, String> {
        let p = self.p;
        let text = Ty::Base(Base::Text);
        let i64t = Ty::Base(Base::I64);
        match p {
            Prim::BytesLen => {
                let [a] = self.arity()?;
                self.admits(matches!(a, Ty::Base(Base::Text | Base::Bytes)), &[a])?;
                Ok(i64t)
            }
            Prim::ScalarLen | Prim::GraphemeLen => {
                self.operands(&[&text])?;
                Ok(i64t)
            }
            Prim::StartsWith | Prim::EndsWith | Prim::Contains => {
                self.operands(&[&text, &text])?;
                Ok(self.bool())
            }
            Prim::Matches => {
                self.operands(&[&text, &text])?;
                self.constant_text(1)?;
                Ok(self.bool())
            }
            Prim::Lower | Prim::Upper | Prim::Fold => {
                self.operands(&[&text])?;
                Ok(text)
            }
            Prim::Collate => {
                self.operands(&[&text, &text])?;
                self.constant_text(1)?;
                Ok(Ty::Base(Base::SortKey))
            }
            Prim::Utf8 => {
                self.operands(&[&Ty::Base(Base::Bytes)])?;
                Ok(self.option(text))
            }
            Prim::Bytes => {
                self.operands(&[&text])?;
                Ok(Ty::Base(Base::Bytes))
            }
            _ => Err(format!("`{p:?}` is not a text primitive")),
        }
    }

    fn time(&self) -> Result<Ty, String> {
        let p = self.p;
        let text = Ty::Base(Base::Text);
        let i64t = Ty::Base(Base::I64);
        match p {
            Prim::Micros | Prim::Millis | Prim::Seconds | Prim::Minutes | Prim::Hours => {
                self.operands(&[&i64t])?;
                Ok(Ty::Base(Base::Duration))
            }
            Prim::Days | Prim::Weeks | Prim::Months | Prim::Years => {
                self.operands(&[&i64t])?;
                Ok(Ty::Base(Base::Period))
            }
            Prim::AddPeriod => {
                let [a, b, _] = self.arity()?;
                self.admits(
                    matches!(
                        a,
                        Ty::Base(Base::Date | Base::DateTime | Base::ZonedDateTime)
                    ) && *b == Ty::Base(Base::Period),
                    &[a, b],
                )?;
                self.overflowing(a.clone())
            }
            Prim::Zoned => {
                let [a, b, _] = self.arity()?;
                self.admits(*a == Ty::Base(Base::DateTime) && *b == text, &[a, b])?;
                self.overflowing(Ty::Base(Base::ZonedDateTime))
            }
            Prim::InZone => {
                self.operands(&[&Ty::Base(Base::Instant), &text])?;
                Ok(Ty::Base(Base::ZonedDateTime))
            }
            Prim::ToInstant => {
                self.operands(&[&Ty::Base(Base::ZonedDateTime)])?;
                Ok(Ty::Base(Base::Instant))
            }
            _ => Err(format!("`{p:?}` is not a time primitive")),
        }
    }

    fn span(&self) -> Result<Ty, String> {
        let p = self.p;
        match p {
            Prim::Span => {
                let [a, b] = self.arity()?;
                let t = comparable(self.schema, a, b).ok_or_else(|| self.refused(&[a, b]))?;
                self.ordered(&t)?;
                Ok(Ty::Data {
                    def: self.schema.prelude().span(),
                    args: vec![t],
                    ctors: None,
                })
            }
            Prim::Start | Prim::End => {
                let [a] = self.arity()?;
                self.span_arg(a).cloned().ok_or_else(|| self.refused(&[a]))
            }
            Prim::Overlaps | Prim::Covers => {
                let [a, b] = self.arity()?;
                let t = self.span_arg(a).ok_or_else(|| self.refused(&[a, b]))?;
                let ok = subtype(self.schema, b, a)
                    || (*p == Prim::Covers
                        && self.span_arg(b).is_none()
                        && subtype(self.schema, b, t));
                self.admits(ok, &[a, b])?;
                Ok(self.bool())
            }
            _ => Err(format!("`{p:?}` is not a span primitive")),
        }
    }

    /// Returns the result of arithmetic operator `p` over `a` and `b`; a checked operator reads
    /// as its operator.
    fn arith(&self, a: &Ty, b: &Ty) -> Result<Ty, String> {
        use Base::*;
        let p = self.p;
        let additive = matches!(
            p,
            Prim::Add | Prim::Sub | Prim::CheckedAdd | Prim::CheckedSub
        );
        let product = matches!(p, Prim::Mul | Prim::CheckedMul);
        let quotient = *p == Prim::Div;
        let difference = matches!(p, Prim::Sub | Prim::CheckedSub);
        let r = match (a, b) {
            (Ty::Base(x), Ty::Base(y)) if is_integer(a) && x == y => *x,
            (Ty::Base(Decimal(s)), Ty::Base(Decimal(t))) if additive => Decimal((*s).max(*t)),
            (Ty::Base(Decimal(s)), Ty::Base(Decimal(t))) if product => {
                let scale = u32::from(*s) + u32::from(*t);
                expect(scale <= 38, || {
                    format!("`{p:?}` of `Decimal {s}` and `Decimal {t}` has scale {scale}")
                })?;
                Decimal(s + t)
            }
            (Ty::Base(x @ (F32 | F64)), Ty::Base(y)) if x == y && *p != Prim::Rem => *x,
            (Ty::Base(Duration), Ty::Base(Duration)) if additive => Duration,
            (Ty::Base(Duration), Ty::Base(I64)) if product || quotient => Duration,
            (Ty::Base(Period), Ty::Base(Period)) if additive => Period,
            (Ty::Base(Period), Ty::Base(I64)) if product => Period,
            (Ty::Base(t @ (Instant | DateTime | ZonedDateTime)), Ty::Base(Duration))
                if additive =>
            {
                *t
            }
            (Ty::Base(Instant), Ty::Base(Instant))
            | (Ty::Base(DateTime), Ty::Base(DateTime))
            | (Ty::Base(Date), Ty::Base(Date))
                if difference =>
            {
                Duration
            }
            (Ty::Param(i), Ty::Param(j))
                if i == j && additive && self.bounds.has(*i, Bound::Num) =>
            {
                return Ok(a.clone());
            }
            _ => return Err(self.refused(&[a, b])),
        };
        Ok(Ty::Base(r))
    }

    fn convert(&self, checked: bool, target: &Ty) -> Result<Ty, String> {
        let to = if checked {
            self.option_of(target)
        } else {
            Some(target)
        };
        let Some(Ty::Base(to)) = to else {
            return Err(format!("a conversion to `{}`", self.schema.show_ty(target)));
        };
        let to = *to;
        let tys: Vec<&Ty> = self.args.iter().map(|a| self.terms.ty(*a)).collect();
        let Some(from) = tys.first().copied() else {
            return Err(format!("`{:?}` takes an operand", self.p));
        };
        let rounded = |at: usize| tys.len() == at + 1 && self.rounding(tys[at]).is_ok();
        let ok = match to {
            Base::I8 | Base::I16 | Base::I32 | Base::I64 => {
                (is_integer(from) && tys.len() == 1)
                    || ((is_decimal(from) || is_float(from)) && rounded(1))
            }
            Base::Decimal(s) => {
                tys.len() >= 2
                    && self.scale(1) == Ok(s)
                    && ((is_integer(from) && tys.len() == 2) || (is_float(from) && rounded(2)))
            }
            Base::F32 | Base::F64 => {
                tys.len() == 1
                    && (is_integer(from)
                        || is_decimal(from)
                        || (is_float(from) && *from != Ty::Base(to)))
            }
            _ => false,
        };
        expect(ok, || {
            format!(
                "`{:?}` takes no {} to `{}`",
                self.p,
                show_all(self.schema, &tys),
                self.schema.show_ty(target)
            )
        })?;
        Ok(target.clone())
    }

    /// Returns `ty`, or its `Option` when the last operand is the constant `Reject`.
    fn overflowing(&self, ty: Ty) -> Result<Ty, String> {
        let last = *self
            .args
            .last()
            .expect("an overflowing primitive has operands");
        expect(self.is(self.terms.ty(last), self.overflow()), || {
            format!("the last operand of `{:?}` is not an `Overflow`", self.p)
        })?;
        match self.terms.kind(last) {
            Kind::Ctor {
                ctor: ctor::REJECT, ..
            } => Ok(self.option(ty)),
            Kind::Ctor {
                ctor: ctor::CLAMP, ..
            } => Ok(ty),
            _ => Err(format!(
                "the `Overflow` of `{:?}` is not a constant",
                self.p
            )),
        }
    }

    fn arity<const N: usize>(&self) -> Result<[&Ty; N], String> {
        let tys: Vec<&Ty> = self.args.iter().map(|a| self.terms.ty(*a)).collect();
        tys.try_into().map_err(|tys: Vec<&Ty>| {
            format!("`{:?}` takes {N} arguments, given {}", self.p, tys.len())
        })
    }

    fn operands(&self, want: &[&Ty]) -> Result<(), String> {
        let tys: Vec<&Ty> = self.args.iter().map(|a| self.terms.ty(*a)).collect();
        expect(tys.len() == want.len(), || {
            format!(
                "`{:?}` takes {} arguments, given {}",
                self.p,
                want.len(),
                tys.len()
            )
        })?;
        self.admits(tys.iter().zip(want).all(|(t, w)| t == w), &tys)
    }

    /// Check that `ty` is not known to have no order; a type parameter is ordered by its `Ord`
    /// bound.
    fn ordered(&self, ty: &Ty) -> Result<(), String> {
        let bounded = |i| self.bounds.has(i, Bound::Ord);
        let unordered = matches!(self.schema.is_ordered_with(ty, &bounded), Ok(false));
        expect(!unordered, || {
            format!(
                "`{:?}` orders `{}`, which has no order",
                self.p,
                self.schema.show_ty(ty)
            )
        })
    }

    fn admits(&self, holds: bool, tys: &[&Ty]) -> Result<(), String> {
        expect(holds, || self.refused(tys))
    }

    fn refused(&self, tys: &[&Ty]) -> String {
        format!(
            "`{:?}` does not take {}",
            self.p,
            show_all(self.schema, tys)
        )
    }

    fn target<'t>(&self, target: Option<&'t Ty>) -> Result<&'t Ty, String> {
        target.ok_or_else(|| format!("`{:?}` names no result type", self.p))
    }

    /// Returns operand `at`, a `Decimal` scale: an `I64` constant from 0 to 38.
    fn scale(&self, at: usize) -> Result<u8, String> {
        let u = self.args[at];
        let constant = match self.terms.kind(u) {
            Kind::Const(Const::Int(u)) => u8::try_from(*u).ok().filter(|u| *u <= 38),
            _ => None,
        };
        expect(*self.terms.ty(u) == Ty::Base(Base::I64), || {
            format!("the scale of `{:?}` is not an `I64`", self.p)
        })?;
        constant
            .ok_or_else(|| format!("the scale of `{:?}` is not a constant from 0 to 38", self.p))
    }

    fn constant_text(&self, at: usize) -> Result<(), String> {
        expect(
            matches!(self.terms.kind(self.args[at]), Kind::Const(Const::Text(_))),
            || format!("operand {at} of `{:?}` is not a constant", self.p),
        )
    }

    fn rounding(&self, ty: &Ty) -> Result<(), String> {
        expect(self.is(ty, self.schema.prelude().rounding()), || {
            format!("the rounding of `{:?}` is not a `Rounding`", self.p)
        })
    }

    fn is_num(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Base(b) => b.is_num(),
            Ty::Param(i) => self.bounds.has(*i, Bound::Num),
            _ => false,
        }
    }

    fn newtype_base(&self, def: TypeId) -> Option<&Ty> {
        match &self.schema.ty(def).kind {
            TypeDefKind::Newtype { base, .. } => Some(base),
            _ => None,
        }
    }

    fn span_arg<'t>(&self, ty: &'t Ty) -> Option<&'t Ty> {
        match ty {
            Ty::Data { def, args, .. } if *def == self.schema.prelude().span() => args.first(),
            _ => None,
        }
    }

    fn option_of<'t>(&self, ty: &'t Ty) -> Option<&'t Ty> {
        match ty {
            Ty::Data { def, args, .. } if *def == self.schema.prelude().option() => args.first(),
            _ => None,
        }
    }

    fn option(&self, t: Ty) -> Ty {
        Ty::Data {
            def: self.schema.prelude().option(),
            args: vec![t],
            ctors: None,
        }
    }

    fn bool(&self) -> Ty {
        data(self.schema.prelude().bool())
    }

    fn is(&self, ty: &Ty, def: TypeId) -> bool {
        subtype(self.schema, ty, &data(def))
    }

    fn overflow(&self) -> TypeId {
        self.schema.prelude().overflow()
    }
}

fn data(def: TypeId) -> Ty {
    Ty::Data {
        def,
        args: Vec::new(),
        ctors: None,
    }
}

fn expect(holds: bool, what: impl FnOnce() -> String) -> Result<(), String> {
    if holds { Ok(()) } else { Err(what()) }
}

/// Returns the join of `a` and `b`, or `a` when the two are forms of the same sums still being
/// inferred.
pub(super) fn comparable(schema: &Schema, a: &Ty, b: &Ty) -> Option<Ty> {
    join(schema, a, b).or_else(|| forms_of_one_class(a, b).then(|| a.clone()))
}

fn show_all(schema: &Schema, tys: &[&Ty]) -> String {
    tys.iter()
        .map(|t| format!("`{}`", schema.show_ty(t)))
        .collect::<Vec<_>>()
        .join(" and ")
}

fn is_integer(ty: &Ty) -> bool {
    matches!(ty, Ty::Base(Base::I8 | Base::I16 | Base::I32 | Base::I64))
}

fn is_decimal(ty: &Ty) -> bool {
    matches!(ty, Ty::Base(Base::Decimal(_)))
}

fn is_float(ty: &Ty) -> bool {
    matches!(ty, Ty::Base(Base::F32 | Base::F64))
}
