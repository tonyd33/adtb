use std::cmp::Ordering;
use std::sync::Arc;

use adtb_lang::kernel::Prim;
use adtb_lang::schema::Schema;
use adtb_lang::textual::{self, Form};
use adtb_lang::types::{Base, Ty};

use super::types::{base, option_arg};
use super::value::{F32, F64, Val};
use super::wide::{Wide, cmp_decimal, rescaled};
use super::{Error, Fault};

mod num;
mod syntax;

/// Returns the canonical printed form of `v`, a value of type `ty`.
pub(crate) fn print(schema: &Schema, ty: &Ty, v: &Val) -> Result<String, Error> {
    Ok(textual::print(v, &form(schema, ty)?)?)
}

fn form(schema: &Schema, ty: &Ty) -> Result<Form, Error> {
    let f = Form::of(schema, ty);
    if !f.has_syntax() {
        return unsupported(format!("the textual form of {ty:?}"));
    }
    Ok(f)
}
mod time;
pub(super) mod unicode;

use num::Rounding;
use time::MICROS_PER_DAY;
use unicode::{Case, Collator, Pattern};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    I8,
    I16,
    I32,
    I64,
}

impl Width {
    fn of(b: Base) -> Option<Width> {
        Some(match b {
            Base::I8 => Width::I8,
            Base::I16 => Width::I16,
            Base::I32 => Width::I32,
            Base::I64 => Width::I64,
            _ => return None,
        })
    }

    fn fit(self, v: i128) -> Option<i64> {
        let (lo, hi) = match self {
            Width::I8 => (i8::MIN.into(), i8::MAX.into()),
            Width::I16 => (i16::MIN.into(), i16::MAX.into()),
            Width::I32 => (i32::MIN.into(), i32::MAX.into()),
            Width::I64 => (i64::MIN.into(), i64::MAX.into()),
        };
        (lo..=hi).contains(&v).then_some(v as i64)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arith {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

/// A primitive operation, chosen for its operand types when the plan is compiled.
#[derive(Clone, Debug)]
pub enum Op {
    Int(Arith, Width),
    IntNeg(Width),
    IntAbs(Width),
    /// Decimals of scales `s` and `t` added or subtracted at the larger scale.
    DecAdd {
        s: u8,
        t: u8,
        sub: bool,
    },
    DecMul,
    DecNeg,
    DecAbs,
    Float(Arith),
    FloatNeg,
    FloatAbs,
    DurAdd,
    DurSub,
    DurNeg,
    DurAbs,
    DurMul,
    DurDiv,
    PeriodAdd,
    PeriodSub,
    PeriodNeg,
    PeriodMul,
    /// `Instant` or `DateTime` plus or minus a `Duration`.
    PointShift {
        sub: bool,
    },
    /// The `Duration` between two `Instant`s or two `DateTime`s.
    PointDiff,
    DateDiff,
    Eq,
    Ne,
    Cmp(Ordering, bool),
    /// Decimals of scales `s` and `t`.
    DecEq {
        s: u8,
        t: u8,
        ne: bool,
    },
    DecCmp {
        s: u8,
        t: u8,
        want: Ordering,
        or_eq: bool,
    },
    ConcatText,
    ConcatBytes,
    /// The operation's value as `Some`, or `None` where it faults.
    Checked(Box<Op>),
    Same,
    IntToInt(Width),
    IntToDec(u8),
    IntToF32,
    IntToF64,
    F32ToF64,
    F64ToF32,
    /// A decimal at scale `from` at the larger scale `to`.
    Rescale {
        from: u8,
        to: u8,
    },
    /// A decimal at scale `from` rounded to the smaller scale `to`.
    Round {
        from: u8,
        to: u8,
    },
    BytesLen,
    ScalarLen,
    StartsWith,
    EndsWith,
    Contains,
    ToBytes,
    Utf8,
    /// A count of units of this many microseconds.
    Duration(i64),
    /// A count of units of this many days, or of months when `months`.
    Period {
        months: bool,
        per: i64,
    },
    /// Decimals of scales `s` and `t` divided to scale `u`.
    DecDiv {
        s: u8,
        t: u8,
        u: u8,
    },
    /// A decimal of scale `s` rounded to an integer.
    DecToInt(u8, Width),
    FloatToInt(Width),
    /// A float rounded to a decimal of this scale.
    FloatToDec(u8),
    /// A decimal of scale `s` at the nearest float, an `F32` when `single`.
    DecToFloat {
        s: u8,
        single: bool,
    },
    ZonedShift {
        sub: bool,
    },
    GraphemeLen,
    Case(Case),
    Matches(Arc<Pattern>),
    Collate(Arc<Collator>),
    ParseUuid,
    ToText(Arc<Form>),
    Print(Arc<Form>),
    Parse(Arc<Form>),
    /// `add_period` on a `Date`, `DateTime` or `ZonedDateTime`, giving an `Option` when `reject`.
    AddPeriod {
        reject: bool,
    },
    Zoned {
        reject: bool,
    },
    InZone,
    ToInstant,
    Span,
    Start,
    End,
    Overlaps,
    /// `covers` of a span, or of a point when `point`.
    Covers {
        point: bool,
    },
}

fn unsupported<T>(what: impl std::fmt::Display) -> Result<T, Error> {
    Err(Error::Unsupported(what.to_string()))
}

/// Chooses the operation `p` performs on operands of types `args` with result type `ty`; `consts`
/// holds the value of each operand that is a literal.
#[expect(
    clippy::too_many_lines,
    clippy::cognitive_complexity,
    reason = "the choice for each family of primitives would have to be a function"
)]
pub fn choose(
    schema: &Schema,
    p: Prim,
    args: &[Ty],
    consts: &[Option<Val>],
    ty: &Ty,
) -> Result<Op, Error> {
    let b = |t: &Ty| base(schema, t);
    let arg = |i: usize| args.get(i).and_then(b);
    let arith = match p {
        Prim::Add | Prim::CheckedAdd => Some(Arith::Add),
        Prim::Sub | Prim::CheckedSub => Some(Arith::Sub),
        Prim::Mul | Prim::CheckedMul => Some(Arith::Mul),
        Prim::Div | Prim::CheckedDiv => Some(Arith::Div),
        Prim::Rem | Prim::CheckedRem => Some(Arith::Rem),
        _ => None,
    };
    let checked = matches!(
        p,
        Prim::CheckedAdd
            | Prim::CheckedSub
            | Prim::CheckedMul
            | Prim::CheckedDiv
            | Prim::CheckedRem
            | Prim::CheckedNeg
    );
    let result = if checked {
        option_arg(schema, ty)
            .ok_or_else(|| Error::Internal("a checked primitive returns an Option".into()))?
    } else {
        ty.clone()
    };
    let op = match p {
        _ if arith.is_some() => {
            let a = arith.unwrap();
            match (arg(0), arg(1)) {
                (Some(x), _) if Width::of(x).is_some() => {
                    let w = b(&result).and_then(Width::of).ok_or_else(|| {
                        Error::Internal("integer arithmetic has an integer result".into())
                    })?;
                    Op::Int(a, w)
                }
                (Some(Base::Decimal(s)), Some(Base::Decimal(t))) => match a {
                    Arith::Add => Op::DecAdd { s, t, sub: false },
                    Arith::Sub => Op::DecAdd { s, t, sub: true },
                    Arith::Mul => Op::DecMul,
                    _ => return unsupported(format!("{p:?} on decimals")),
                },
                (Some(Base::F32 | Base::F64), _) => Op::Float(a),
                (Some(Base::Duration), Some(Base::Duration)) => match a {
                    Arith::Add => Op::DurAdd,
                    Arith::Sub => Op::DurSub,
                    _ => return unsupported(format!("{p:?} on durations")),
                },
                (Some(Base::Duration), Some(_)) => match a {
                    Arith::Mul => Op::DurMul,
                    Arith::Div => Op::DurDiv,
                    _ => return unsupported(format!("{p:?} on a duration")),
                },
                (Some(Base::Period), Some(Base::Period)) => match a {
                    Arith::Add => Op::PeriodAdd,
                    Arith::Sub => Op::PeriodSub,
                    _ => return unsupported(format!("{p:?} on periods")),
                },
                (Some(Base::Period), Some(_)) if a == Arith::Mul => Op::PeriodMul,
                (Some(Base::Instant | Base::DateTime), Some(Base::Duration)) => match a {
                    Arith::Add => Op::PointShift { sub: false },
                    Arith::Sub => Op::PointShift { sub: true },
                    _ => return unsupported(format!("{p:?} on a point in time")),
                },
                (Some(Base::ZonedDateTime), Some(Base::Duration)) => match a {
                    Arith::Add => Op::ZonedShift { sub: false },
                    Arith::Sub => Op::ZonedShift { sub: true },
                    _ => return unsupported(format!("{p:?} on a zoned date-time")),
                },
                (Some(Base::Instant), Some(Base::Instant))
                | (Some(Base::DateTime), Some(Base::DateTime))
                    if a == Arith::Sub =>
                {
                    Op::PointDiff
                }
                (Some(Base::Date), Some(Base::Date)) if a == Arith::Sub => Op::DateDiff,
                _ => return unsupported(format!("{p:?} on {args:?}")),
            }
        }
        Prim::Neg | Prim::CheckedNeg | Prim::Abs => {
            let abs = p == Prim::Abs;
            match arg(0) {
                Some(x) if Width::of(x).is_some() => {
                    let w = Width::of(x).unwrap();
                    if abs { Op::IntAbs(w) } else { Op::IntNeg(w) }
                }
                Some(Base::Decimal(_)) => {
                    if abs {
                        Op::DecAbs
                    } else {
                        Op::DecNeg
                    }
                }
                Some(Base::F32 | Base::F64) => {
                    if abs {
                        Op::FloatAbs
                    } else {
                        Op::FloatNeg
                    }
                }
                Some(Base::Duration) => {
                    if abs {
                        Op::DurAbs
                    } else {
                        Op::DurNeg
                    }
                }
                Some(Base::Period) if !abs => Op::PeriodNeg,
                _ => return unsupported(format!("{p:?} on {args:?}")),
            }
        }
        Prim::Eq | Prim::Ne => match (arg(0), arg(1)) {
            (Some(Base::Decimal(s)), Some(Base::Decimal(t))) if s != t => Op::DecEq {
                s,
                t,
                ne: p == Prim::Ne,
            },
            _ if p == Prim::Eq => Op::Eq,
            _ => Op::Ne,
        },
        Prim::Lt | Prim::Le | Prim::Gt | Prim::Ge => {
            let (want, or_eq) = match p {
                Prim::Lt => (Ordering::Less, false),
                Prim::Le => (Ordering::Less, true),
                Prim::Gt => (Ordering::Greater, false),
                _ => (Ordering::Greater, true),
            };
            match (arg(0), arg(1)) {
                (Some(Base::Decimal(s)), Some(Base::Decimal(t))) if s != t => {
                    Op::DecCmp { s, t, want, or_eq }
                }
                (Some(Base::ZonedDateTime), _) => {
                    return unsupported("ordering zoned date-times by their instants");
                }
                _ => Op::Cmp(want, or_eq),
            }
        }
        Prim::Concat => match arg(0) {
            Some(Base::Text) => Op::ConcatText,
            _ => Op::ConcatBytes,
        },
        Prim::Wrap | Prim::Unwrap | Prim::KeyValue => Op::Same,
        Prim::Convert { checked: c } => {
            let to = if c {
                option_arg(schema, ty).ok_or_else(|| {
                    Error::Internal("a checked conversion returns an Option".into())
                })?
            } else {
                ty.clone()
            };
            let op = match (arg(0), b(&to)) {
                (Some(x), Some(y)) if Width::of(x).is_some() && Width::of(y).is_some() => {
                    Op::IntToInt(Width::of(y).unwrap())
                }
                (Some(x), Some(Base::Decimal(s))) if Width::of(x).is_some() => Op::IntToDec(s),
                (Some(x), Some(Base::F32)) if Width::of(x).is_some() => Op::IntToF32,
                (Some(x), Some(Base::F64)) if Width::of(x).is_some() => Op::IntToF64,
                (Some(Base::F32), Some(Base::F64)) => Op::F32ToF64,
                (Some(Base::F64), Some(Base::F32)) => Op::F64ToF32,
                (Some(Base::Decimal(s)), Some(y)) if Width::of(y).is_some() => {
                    Op::DecToInt(s, Width::of(y).unwrap())
                }
                (Some(Base::F32 | Base::F64), Some(y)) if Width::of(y).is_some() => {
                    Op::FloatToInt(Width::of(y).unwrap())
                }
                (Some(Base::F32 | Base::F64), Some(Base::Decimal(s))) => Op::FloatToDec(s),
                (Some(Base::Decimal(s)), Some(y @ (Base::F32 | Base::F64))) => Op::DecToFloat {
                    s,
                    single: y == Base::F32,
                },
                (x, y) => return unsupported(format!("a conversion from {x:?} to {y:?}")),
            };
            if c {
                return Ok(Op::Checked(Box::new(op)));
            }
            op
        }
        Prim::Rescale => match (arg(0), b(ty)) {
            (Some(Base::Decimal(from)), Some(Base::Decimal(to))) => Op::Rescale { from, to },
            _ => return unsupported("`rescale` off decimals"),
        },
        Prim::Round => match (arg(0), b(ty)) {
            (Some(Base::Decimal(from)), Some(Base::Decimal(to))) => Op::Round { from, to },
            _ => return unsupported("`round` off decimals"),
        },
        Prim::DecimalDiv => match (arg(0), arg(1), b(ty)) {
            (Some(Base::Decimal(s)), Some(Base::Decimal(t)), Some(Base::Decimal(u))) => {
                Op::DecDiv { s, t, u }
            }
            _ => return unsupported("`div` off decimals"),
        },
        Prim::GraphemeLen => Op::GraphemeLen,
        Prim::Lower => Op::Case(Case::Lower),
        Prim::Upper => Op::Case(Case::Upper),
        Prim::Fold => Op::Case(Case::Fold),
        Prim::Matches | Prim::Collate => {
            let Some(Some(Val::Text(c))) = consts.get(1) else {
                return unsupported(format!("{p:?} with an operand that is not a literal"));
            };
            if p == Prim::Matches {
                Op::Matches(Arc::new(Pattern::new(c).map_err(Error::Unsupported)?))
            } else {
                Op::Collate(Arc::new(Collator::new(c).map_err(Error::Unsupported)?))
            }
        }
        Prim::ParseUuid => Op::ParseUuid,
        Prim::ToText => Op::ToText(Arc::new(form(schema, &args[0])?)),
        Prim::Print => Op::Print(Arc::new(form(schema, &args[0])?)),
        Prim::Parse => {
            let t = option_arg(schema, ty)
                .ok_or_else(|| Error::Internal("`parse` returns an Option".into()))?;
            Op::Parse(Arc::new(form(schema, &t)?))
        }
        Prim::AddPeriod => Op::AddPeriod {
            reject: option_arg(schema, ty).is_some(),
        },
        Prim::Zoned => Op::Zoned {
            reject: option_arg(schema, ty).is_some(),
        },
        Prim::InZone => Op::InZone,
        Prim::ToInstant => Op::ToInstant,
        Prim::Span => Op::Span,
        Prim::Start => Op::Start,
        Prim::End => Op::End,
        Prim::Overlaps => Op::Overlaps,
        Prim::Covers => Op::Covers {
            point: args[1] != args[0],
        },
        Prim::BytesLen => Op::BytesLen,
        Prim::ScalarLen => Op::ScalarLen,
        Prim::StartsWith => Op::StartsWith,
        Prim::EndsWith => Op::EndsWith,
        Prim::Contains => Op::Contains,
        Prim::Bytes => Op::ToBytes,
        Prim::Utf8 => Op::Utf8,
        Prim::Micros => Op::Duration(1),
        Prim::Millis => Op::Duration(1_000),
        Prim::Seconds => Op::Duration(1_000_000),
        Prim::Minutes => Op::Duration(60_000_000),
        Prim::Hours => Op::Duration(3_600_000_000),
        Prim::Days => Op::Period {
            months: false,
            per: 1,
        },
        Prim::Weeks => Op::Period {
            months: false,
            per: 7,
        },
        Prim::Months => Op::Period {
            months: true,
            per: 1,
        },
        Prim::Years => Op::Period {
            months: true,
            per: 12,
        },
        other => return unsupported(format!("the primitive {other:?}")),
    };
    Ok(if checked {
        Op::Checked(Box::new(op))
    } else {
        op
    })
}

fn overflow<T>(v: Option<T>) -> Result<T, Fault> {
    v.ok_or(Fault::ArithmeticOverflow)
}

fn mistyped(want: &str, found: impl std::fmt::Debug) -> Error {
    Error::Internal(format!(
        "a primitive's operands are {want}, found {found:?}"
    ))
}

fn text(v: &Val) -> Result<&str, Error> {
    match v {
        Val::Text(t) => Ok(t),
        other => Err(mistyped("text", other)),
    }
}

fn bytes(v: &Val) -> Result<&[u8], Error> {
    match v {
        Val::Bytes(b) | Val::SortKey(b) => Ok(b),
        other => Err(mistyped("bytes", other)),
    }
}

fn dec(v: &Val) -> Result<i128, Error> {
    match v {
        Val::Decimal(d) => Ok(*d),
        other => Err(mistyped("a decimal", other)),
    }
}

fn int(v: &Val) -> Result<i64, Error> {
    match v {
        Val::Int(i) => Ok(*i),
        other => Err(mistyped("an integer", other)),
    }
}

fn duration(v: &Val) -> Result<i64, Error> {
    match v {
        Val::Duration(d) => Ok(*d),
        other => Err(mistyped("a duration", other)),
    }
}

fn period(v: &Val) -> Result<(i32, i32), Error> {
    match v {
        Val::Period { months, days } => Ok((*months, *days)),
        other => Err(mistyped("a period", other)),
    }
}

fn float(v: &Val) -> Result<f64, Error> {
    match v {
        Val::F32(x) => Ok(f64::from(x.get())),
        Val::F64(x) => Ok(x.get()),
        other => Err(mistyped("a float", other)),
    }
}

fn fields(v: &Val) -> Result<&[Val], Error> {
    match v {
        Val::Ctor(_, fs) | Val::Record(fs) => Ok(fs),
        other => Err(mistyped("values with fields", other)),
    }
}

/// Returns a float of the same type as `like`.
fn float_like(like: &Val, x: f64) -> Val {
    match like {
        Val::F32(_) => Val::F32(F32::new(x as f32)),
        _ => Val::F64(F64::new(x)),
    }
}

fn digits38(v: i128) -> Option<i128> {
    (v.unsigned_abs() < 10u128.pow(38)).then_some(v)
}

/// Returns `op` of `xs`, or the fault it raises. Fails with `Unsupported` for values it cannot
/// compute yet.
#[expect(
    clippy::too_many_lines,
    reason = "one arm per operation; each family of operations would have to be a function"
)]
pub fn apply(op: &Op, xs: &[&Val]) -> Result<Result<Val, Fault>, Error> {
    let x = || xs[0];
    let y = || xs[1];
    let v = match op {
        Op::Int(a, w) => {
            let (p, q) = (i128::from(int(x())?), i128::from(int(y())?));
            let r = match a {
                Arith::Add => p + q,
                Arith::Sub => p - q,
                Arith::Mul => p * q,
                Arith::Div | Arith::Rem if q == 0 => return Ok(Err(Fault::DivisionByZero)),
                Arith::Div => p / q,
                Arith::Rem => p % q,
            };
            Val::Int(match w.fit(r) {
                Some(v) => v,
                None => return Ok(Err(Fault::ArithmeticOverflow)),
            })
        }
        Op::IntNeg(w) | Op::IntAbs(w) => {
            let p = i128::from(int(x())?);
            let r = if matches!(op, Op::IntNeg(_)) {
                -p
            } else {
                p.abs()
            };
            match w.fit(r) {
                Some(v) => Val::Int(v),
                None => return Ok(Err(Fault::ArithmeticOverflow)),
            }
        }
        Op::DecAdd { s, t, sub } => {
            let m = (*s).max(*t);
            let a = rescaled(dec(x())?, *s, m);
            let mut b = rescaled(dec(y())?, *t, m);
            if *sub {
                b = b.neg();
            }
            match a.add(b).and_then(Wide::digits38) {
                Some(v) => Val::Decimal(v),
                None => return Ok(Err(Fault::ArithmeticOverflow)),
            }
        }
        Op::DecMul => match dec(x())?.checked_mul(dec(y())?).and_then(digits38) {
            Some(v) => Val::Decimal(v),
            None => return Ok(Err(Fault::ArithmeticOverflow)),
        },
        Op::DecNeg => Val::Decimal(-dec(x())?),
        Op::DecAbs => Val::Decimal(dec(x())?.abs()),
        Op::Float(a) => {
            let (p, q) = (float(x())?, float(y())?);
            let r = match (x(), a) {
                (Val::F32(_), _) => {
                    let (p, q) = (p as f32, q as f32);
                    f64::from(match a {
                        Arith::Add => p + q,
                        Arith::Sub => p - q,
                        Arith::Mul => p * q,
                        Arith::Div => p / q,
                        Arith::Rem => return Err(Error::Unsupported("`%` on floats".into())),
                    })
                }
                _ => match a {
                    Arith::Add => p + q,
                    Arith::Sub => p - q,
                    Arith::Mul => p * q,
                    Arith::Div => p / q,
                    Arith::Rem => return Err(Error::Unsupported("`%` on floats".into())),
                },
            };
            float_like(x(), r)
        }
        Op::FloatNeg => float_like(x(), -float(x())?),
        Op::FloatAbs => float_like(x(), float(x())?.abs()),
        Op::DurAdd | Op::DurSub | Op::DurNeg | Op::DurAbs | Op::DurMul | Op::DurDiv => {
            let d = duration(x())?;
            let r = match op {
                Op::DurAdd => d.checked_add(duration(y())?),
                Op::DurSub => d.checked_sub(duration(y())?),
                Op::DurNeg => d.checked_neg(),
                Op::DurAbs => d.checked_abs(),
                Op::DurMul => d.checked_mul(int(y())?),
                _ if int(y())? == 0 => return Ok(Err(Fault::DivisionByZero)),
                _ => d.checked_div(int(y())?),
            };
            match overflow(r) {
                Ok(v) => Val::Duration(v),
                Err(f) => return Ok(Err(f)),
            }
        }
        Op::PeriodAdd | Op::PeriodSub | Op::PeriodNeg | Op::PeriodMul => {
            let (m, d) = period(x())?;
            let by = |c: i32, n: i64| i32::try_from(i64::from(c).checked_mul(n)?).ok();
            let (m, d) = match op {
                Op::PeriodAdd => {
                    let (n, e) = period(y())?;
                    (m.checked_add(n), d.checked_add(e))
                }
                Op::PeriodSub => {
                    let (n, e) = period(y())?;
                    (m.checked_sub(n), d.checked_sub(e))
                }
                Op::PeriodNeg => (m.checked_neg(), d.checked_neg()),
                _ => (by(m, int(y())?), by(d, int(y())?)),
            };
            match (m, d) {
                (Some(months), Some(days)) => Val::Period { months, days },
                _ => return Ok(Err(Fault::ArithmeticOverflow)),
            }
        }
        Op::PointShift { sub } => {
            let d = duration(y())?;
            let shift = |p: i64| {
                if *sub {
                    p.checked_sub(d)
                } else {
                    p.checked_add(d)
                }
            };
            match x() {
                Val::Instant(p) => match shift(*p) {
                    Some(v) => Val::Instant(v),
                    None => return Ok(Err(Fault::ArithmeticOverflow)),
                },
                Val::DateTime(p) => match shift(*p) {
                    Some(v) => Val::DateTime(v),
                    None => return Ok(Err(Fault::ArithmeticOverflow)),
                },
                other => return Err(mistyped("an instant or a date-time", other)),
            }
        }
        Op::PointDiff => {
            let r = match (x(), y()) {
                (Val::Instant(a), Val::Instant(b)) | (Val::DateTime(a), Val::DateTime(b)) => {
                    a.checked_sub(*b)
                }
                (a, b) => return Err(mistyped("two instants or two date-times", (a, b))),
            };
            match r {
                Some(v) => Val::Duration(v),
                None => return Ok(Err(Fault::ArithmeticOverflow)),
            }
        }
        Op::DateDiff => match (x(), y()) {
            (Val::Date(a), Val::Date(b)) => {
                match (i64::from(*a) - i64::from(*b)).checked_mul(MICROS_PER_DAY) {
                    Some(v) => Val::Duration(v),
                    None => return Ok(Err(Fault::ArithmeticOverflow)),
                }
            }
            (a, b) => return Err(mistyped("two dates", (a, b))),
        },
        Op::Eq => Val::bool(x() == y()),
        Op::Ne => Val::bool(x() != y()),
        Op::Cmp(want, or_eq) => {
            let o = x().cmp(y());
            Val::bool(o == *want || (*or_eq && o == Ordering::Equal))
        }
        Op::DecEq { s, t, ne } => {
            let eq = cmp_decimal(dec(x())?, *s, dec(y())?, *t) == Ordering::Equal;
            Val::bool(eq != *ne)
        }
        Op::DecCmp { s, t, want, or_eq } => {
            let o = cmp_decimal(dec(x())?, *s, dec(y())?, *t);
            Val::bool(o == *want || (*or_eq && o == Ordering::Equal))
        }
        Op::ConcatText => {
            let (a, b) = (text(x())?, text(y())?);
            let joined = format!("{a}{b}");
            // NFC text followed by NFC text that starts with ASCII is NFC.
            if b.chars().next().is_none_or(|c| c.is_ascii()) {
                Val::Text(Arc::from(joined))
            } else {
                Val::Text(Arc::from(unicode::nfc(&joined)))
            }
        }
        Op::ConcatBytes => {
            let mut out = bytes(x())?.to_vec();
            out.extend_from_slice(bytes(y())?);
            Val::Bytes(Arc::from(out))
        }
        Op::Checked(inner) => {
            return Ok(Ok(match apply(inner, xs)? {
                Ok(v) => Val::some(v),
                Err(_) => Val::none(),
            }));
        }
        Op::Same => x().clone(),
        Op::IntToInt(w) => match w.fit(i128::from(int(x())?)) {
            Some(v) => Val::Int(v),
            None => return Ok(Err(Fault::OutOfRange)),
        },
        Op::IntToDec(s) => {
            match Wide::from_i128(i128::from(int(x())?))
                .scale(u32::from(*s))
                .and_then(Wide::digits38)
            {
                Some(v) => Val::Decimal(v),
                None => return Ok(Err(Fault::OutOfRange)),
            }
        }
        Op::IntToF32 => Val::F32(F32::new(int(x())? as f32)),
        Op::IntToF64 => Val::F64(F64::new(int(x())? as f64)),
        Op::F32ToF64 => Val::F64(F64::new(float(x())?)),
        Op::F64ToF32 => {
            let v = float(x())?;
            if v.is_finite() && v.abs() > f64::from(f32::MAX) {
                return Ok(Err(Fault::OutOfRange));
            }
            Val::F32(F32::new(v as f32))
        }
        Op::Rescale { from, to } => match rescaled(dec(x())?, *from, *to).digits38() {
            Some(v) => Val::Decimal(v),
            None => return Ok(Err(Fault::ArithmeticOverflow)),
        },
        Op::Round { from, to } => {
            let d = 10i128.pow(u32::from(from - to));
            Val::Decimal(Rounding::of(xs[2])?.div(dec(x())?, d))
        }
        Op::BytesLen => Val::Int(match x() {
            Val::Text(t) => t.len() as i64,
            other => bytes(other)?.len() as i64,
        }),
        Op::ScalarLen => Val::Int(text(x())?.chars().count() as i64),
        Op::StartsWith => Val::bool(text(x())?.starts_with(text(y())?)),
        Op::EndsWith => Val::bool(text(x())?.ends_with(text(y())?)),
        Op::Contains => Val::bool(text(x())?.contains(text(y())?)),
        Op::ToBytes => Val::Bytes(Arc::from(text(x())?.as_bytes())),
        Op::Utf8 => match std::str::from_utf8(bytes(x())?) {
            Ok(t) => Val::some(Val::text(&unicode::nfc(t))),
            Err(_) => Val::none(),
        },
        Op::Duration(per) => match int(x())?.checked_mul(*per) {
            Some(v) => Val::Duration(v),
            None => return Ok(Err(Fault::ArithmeticOverflow)),
        },
        Op::Period { months, per } => {
            let n = int(x())?
                .checked_mul(*per)
                .and_then(|n| i32::try_from(n).ok());
            match (n, months) {
                (None, _) => return Ok(Err(Fault::OutOfRange)),
                (Some(n), true) => Val::Period { months: n, days: 0 },
                (Some(n), false) => Val::Period { months: 0, days: n },
            }
        }
        Op::DecDiv { s, t, u } => {
            match num::decimal_div(dec(x())?, *s, dec(y())?, *t, *u, Rounding::of(xs[3])?) {
                Ok(v) => Val::Decimal(v),
                Err(f) => return Ok(Err(f)),
            }
        }
        Op::DecToInt(s, w) => {
            let r = Rounding::of(y())?.div(dec(x())?, 10i128.pow(u32::from(*s)));
            match w.fit(r) {
                Some(v) => Val::Int(v),
                None => return Ok(Err(Fault::OutOfRange)),
            }
        }
        Op::FloatToInt(w) => match num::float_to_int(float(x())?, Rounding::of(y())?, *w) {
            Ok(v) => Val::Int(v),
            Err(f) => return Ok(Err(f)),
        },
        Op::FloatToDec(s) => match num::float_to_decimal(float(x())?, *s, Rounding::of(xs[2])?) {
            Ok(v) => Val::Decimal(v),
            Err(f) => return Ok(Err(f)),
        },
        Op::DecToFloat { s, single } => {
            let t = num::decimal_text(dec(x())?, *s);
            if *single {
                Val::F32(F32::new(t.parse().expect("a decimal reads as a float")))
            } else {
                Val::F64(F64::new(t.parse().expect("a decimal reads as a float")))
            }
        }
        Op::ZonedShift { sub } => {
            let (local, name) = zoned(x())?;
            let d = duration(y())?;
            let r = time::zone(name).and_then(|z| {
                let i = time::to_instant(local, &z)?;
                let i = if *sub {
                    i.checked_sub(d)
                } else {
                    i.checked_add(d)
                }
                .ok_or(Fault::ArithmeticOverflow)?;
                time::in_zone(i, &z)
            });
            match r {
                Ok(local) => zoned_val(local, x())?,
                Err(f) => return Ok(Err(f)),
            }
        }
        Op::GraphemeLen => Val::Int(unicode::graphemes(text(x())?) as i64),
        Op::Case(c) => Val::Text(Arc::from(unicode::case(text(x())?, *c))),
        Op::Matches(p) => Val::bool(p.is_match(text(x())?)),
        Op::Collate(c) => Val::SortKey(Arc::from(c.key(text(x())?))),
        Op::ParseUuid => match textual::parse_uuid(text(x())?) {
            Some(b) => Val::Bytes(Arc::from(b)),
            None => return Ok(Err(Fault::InvalidText)),
        },
        Op::ToText(f) => Val::Text(Arc::from(textual::to_text(x(), f)?)),
        Op::Print(f) => Val::Text(Arc::from(textual::print(x(), f)?)),
        Op::Parse(f) => match textual::parse(text(x())?, f) {
            Ok(v) => Val::some(v),
            Err(_) => Val::none(),
        },
        Op::AddPeriod { reject } => {
            let (months, days) = period(y())?;
            let r = match x() {
                Val::Date(d) => {
                    time::add_period_date(*d, months, days, *reject).map(|o| o.map(Val::Date))
                }
                Val::DateTime(t) => time::add_period_datetime(*t, months, days, *reject)
                    .map(|o| o.map(Val::DateTime)),
                Val::Zoned { local, zone: name } => time::zone(name).and_then(|zone| {
                    let Some(moved) = time::add_period_datetime(*local, months, days, *reject)?
                    else {
                        return Ok(None);
                    };
                    Ok(time::resolve(moved, &zone, *reject)?.map(|l| Val::Zoned {
                        local: l,
                        zone: name.clone(),
                    }))
                }),
                other => return Err(mistyped("a date, a date-time or a zoned date-time", other)),
            };
            match (r, reject) {
                (Ok(Some(v)), false) => v,
                (Ok(Some(v)), true) => Val::some(v),
                (Ok(None), _) => Val::none(),
                (Err(f), _) => return Ok(Err(f)),
            }
        }
        Op::Zoned { reject } => {
            let Val::DateTime(local) = x() else {
                return Err(mistyped("a date-time", x()));
            };
            let name = text(y())?;
            let r = time::zone(name).and_then(|z| time::resolve(*local, &z, *reject));
            let made = |l: i64| Val::Zoned {
                local: l,
                zone: Arc::from(name),
            };
            match (r, reject) {
                (Ok(Some(l)), false) => made(l),
                (Ok(Some(l)), true) => Val::some(made(l)),
                (Ok(None), _) => Val::none(),
                (Err(f), _) => return Ok(Err(f)),
            }
        }
        Op::InZone => {
            let Val::Instant(i) = x() else {
                return Err(mistyped("an instant", x()));
            };
            let name = text(y())?;
            match time::zone(name).and_then(|z| time::in_zone(*i, &z)) {
                Ok(local) => Val::Zoned {
                    local,
                    zone: Arc::from(name),
                },
                Err(f) => return Ok(Err(f)),
            }
        }
        Op::ToInstant => {
            let (local, name) = zoned(x())?;
            match time::zone(name).and_then(|z| time::to_instant(local, &z)) {
                Ok(i) => Val::Instant(i),
                Err(f) => return Ok(Err(f)),
            }
        }
        Op::Span => {
            if x() > y() {
                return Ok(Err(Fault::InvalidSpan));
            }
            Val::ctor(0, vec![x().clone(), y().clone()])
        }
        Op::Start => fields(x())?[0].clone(),
        Op::End => fields(x())?[1].clone(),
        Op::Overlaps => {
            let (a, b) = (fields(x())?, fields(y())?);
            Val::bool(a[0] < b[1] && b[0] < a[1])
        }
        Op::Covers { point } => {
            let a = fields(x())?;
            if *point {
                Val::bool(a[0] <= *y() && *y() < a[1])
            } else {
                let b = fields(y())?;
                Val::bool(a[0] <= b[0] && b[1] <= a[1])
            }
        }
    };
    Ok(Ok(v))
}

fn zoned(v: &Val) -> Result<(i64, &str), Error> {
    match v {
        Val::Zoned { local, zone } => Ok((*local, zone)),
        other => Err(mistyped("a zoned date-time", other)),
    }
}

/// Returns civil time `local` in the zone of `like`.
fn zoned_val(local: i64, like: &Val) -> Result<Val, Error> {
    match like {
        Val::Zoned { zone, .. } => Ok(Val::Zoned {
            local,
            zone: zone.clone(),
        }),
        other => Err(mistyped("a zoned date-time", other)),
    }
}
