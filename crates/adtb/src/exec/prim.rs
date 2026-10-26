use std::cmp::Ordering;
use std::sync::Arc;

use adtb_lang::kernel::Prim;
use adtb_lang::schema::Schema;
use adtb_lang::types::{Base, Ty};

use super::types::{base, option_arg};
use super::value::{F32, F64, Val};
use super::wide::{Wide, cmp_decimal, rescaled};
use super::{Error, Fault};

const MICROS_PER_DAY: i64 = 86_400_000_000;

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

/// A rounding mode of the prelude's `Rounding`, by constructor position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rounding {
    HalfEven,
    HalfUp,
    HalfDown,
    Up,
    Down,
    Ceiling,
    Floor,
}

impl Rounding {
    fn of(v: &Val) -> Rounding {
        match v.ctor_of() {
            0 => Rounding::HalfEven,
            1 => Rounding::HalfUp,
            2 => Rounding::HalfDown,
            3 => Rounding::Up,
            4 => Rounding::Down,
            5 => Rounding::Ceiling,
            6 => Rounding::Floor,
            c => panic!("constructor {c} is not a Rounding"),
        }
    }

    /// Returns `n / d` rounded, `d > 0`.
    fn div(self, n: i128, d: i128) -> i128 {
        let (q, r) = (n / d, n % d);
        if r == 0 {
            return q;
        }
        let away = if n < 0 { q - 1 } else { q + 1 };
        let twice = r.unsigned_abs() * 2;
        let half = twice.cmp(&d.unsigned_abs());
        let up = match self {
            Rounding::Down => false,
            Rounding::Up => true,
            Rounding::Ceiling => n > 0,
            Rounding::Floor => n < 0,
            Rounding::HalfUp => half != Ordering::Less,
            Rounding::HalfDown => half == Ordering::Greater,
            Rounding::HalfEven => match half {
                Ordering::Less => false,
                Ordering::Greater => true,
                Ordering::Equal => q % 2 != 0,
            },
        };
        if up { away } else { q }
    }
}

/// A primitive operation, chosen for its operand types when the plan is compiled.
#[derive(Clone, Debug, PartialEq)]
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
}

fn unsupported<T>(what: impl std::fmt::Display) -> Result<T, Error> {
    Err(Error::Unsupported(what.to_string()))
}

/// Chooses the operation `p` performs on operands of types `args` with result type `ty`.
pub fn choose(schema: &Schema, p: Prim, args: &[Ty], ty: &Ty) -> Result<Op, Error> {
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
        option_arg(schema, ty).expect("a checked primitive returns an Option")
    } else {
        ty.clone()
    };
    let op = match p {
        _ if arith.is_some() => {
            let a = arith.unwrap();
            match (arg(0), arg(1)) {
                (Some(x), _) if Width::of(x).is_some() => {
                    let w = Width::of(b(&result).expect("an integer result")).unwrap();
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
                option_arg(schema, ty).expect("a checked conversion returns an Option")
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

fn text(v: &Val) -> &str {
    match v {
        Val::Text(t) => t,
        other => panic!("expected text, found {other:?}"),
    }
}

fn bytes(v: &Val) -> &[u8] {
    match v {
        Val::Bytes(b) | Val::SortKey(b) => b,
        other => panic!("expected bytes, found {other:?}"),
    }
}

fn dec(v: &Val) -> i128 {
    match v {
        Val::Decimal(d) => *d,
        other => panic!("expected a decimal, found {other:?}"),
    }
}

fn duration(v: &Val) -> i64 {
    match v {
        Val::Duration(d) => *d,
        other => panic!("expected a duration, found {other:?}"),
    }
}

fn period(v: &Val) -> (i32, i32) {
    match v {
        Val::Period { months, days } => (*months, *days),
        other => panic!("expected a period, found {other:?}"),
    }
}

fn float(v: &Val) -> f64 {
    match v {
        Val::F32(x) => f64::from(x.get()),
        Val::F64(x) => x.get(),
        other => panic!("expected a float, found {other:?}"),
    }
}

/// Returns a float of the same type as `like`.
fn float_like(like: &Val, x: f64) -> Val {
    match like {
        Val::F32(_) => Val::F32(F32::new(x as f32)),
        _ => Val::F64(F64::new(x)),
    }
}

/// Returns whether NFC texts `a` and `b` have an NFC concatenation, judged from `b` alone: true
/// when `b` is empty or starts with ASCII, false when unknown.
fn concat_is_nfc(b: &str) -> bool {
    b.chars().next().is_none_or(|c| c.is_ascii())
}

fn digits38(v: i128) -> Option<i128> {
    (v.unsigned_abs() < 10u128.pow(38)).then_some(v)
}

/// Returns `op` of `xs`, or the fault it raises. Fails with `Unsupported` for values it cannot
/// compute yet.
pub fn apply(op: &Op, xs: &[&Val]) -> Result<Result<Val, Fault>, Error> {
    let x = || xs[0];
    let y = || xs[1];
    let v = match op {
        Op::Int(a, w) => {
            let (p, q) = (i128::from(x().int()), i128::from(y().int()));
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
            let p = i128::from(x().int());
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
            let a = rescaled(dec(x()), *s, m);
            let mut b = rescaled(dec(y()), *t, m);
            if *sub {
                b = b.neg();
            }
            match a.add(b).and_then(Wide::digits38) {
                Some(v) => Val::Decimal(v),
                None => return Ok(Err(Fault::ArithmeticOverflow)),
            }
        }
        Op::DecMul => match dec(x()).checked_mul(dec(y())).and_then(digits38) {
            Some(v) => Val::Decimal(v),
            None => return Ok(Err(Fault::ArithmeticOverflow)),
        },
        Op::DecNeg => Val::Decimal(-dec(x())),
        Op::DecAbs => Val::Decimal(dec(x()).abs()),
        Op::Float(a) => {
            let (p, q) = (float(x()), float(y()));
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
        Op::FloatNeg => float_like(x(), -float(x())),
        Op::FloatAbs => float_like(x(), float(x()).abs()),
        Op::DurAdd | Op::DurSub | Op::DurNeg | Op::DurAbs | Op::DurMul | Op::DurDiv => {
            let d = duration(x());
            let r = match op {
                Op::DurAdd => d.checked_add(duration(y())),
                Op::DurSub => d.checked_sub(duration(y())),
                Op::DurNeg => d.checked_neg(),
                Op::DurAbs => d.checked_abs(),
                Op::DurMul => d.checked_mul(y().int()),
                _ if y().int() == 0 => return Ok(Err(Fault::DivisionByZero)),
                _ => d.checked_div(y().int()),
            };
            match overflow(r) {
                Ok(v) => Val::Duration(v),
                Err(f) => return Ok(Err(f)),
            }
        }
        Op::PeriodAdd | Op::PeriodSub | Op::PeriodNeg | Op::PeriodMul => {
            let (m, d) = period(x());
            let by = |c: i32, n: i64| i32::try_from(i64::from(c).checked_mul(n)?).ok();
            let (m, d) = match op {
                Op::PeriodAdd => {
                    let (n, e) = period(y());
                    (m.checked_add(n), d.checked_add(e))
                }
                Op::PeriodSub => {
                    let (n, e) = period(y());
                    (m.checked_sub(n), d.checked_sub(e))
                }
                Op::PeriodNeg => (m.checked_neg(), d.checked_neg()),
                _ => (by(m, y().int()), by(d, y().int())),
            };
            match (m, d) {
                (Some(months), Some(days)) => Val::Period { months, days },
                _ => return Ok(Err(Fault::ArithmeticOverflow)),
            }
        }
        Op::PointShift { sub } => {
            let d = duration(y());
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
                other => panic!("a shift of {other:?}"),
            }
        }
        Op::PointDiff => {
            let r = match (x(), y()) {
                (Val::Instant(a), Val::Instant(b)) | (Val::DateTime(a), Val::DateTime(b)) => {
                    a.checked_sub(*b)
                }
                (a, b) => panic!("a difference of {a:?} and {b:?}"),
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
            (a, b) => panic!("a difference of {a:?} and {b:?}"),
        },
        Op::Eq => Val::bool(x() == y()),
        Op::Ne => Val::bool(x() != y()),
        Op::Cmp(want, or_eq) => {
            let o = x().cmp(y());
            Val::bool(o == *want || (*or_eq && o == Ordering::Equal))
        }
        Op::DecEq { s, t, ne } => {
            let eq = cmp_decimal(dec(x()), *s, dec(y()), *t) == Ordering::Equal;
            Val::bool(eq != *ne)
        }
        Op::DecCmp { s, t, want, or_eq } => {
            let o = cmp_decimal(dec(x()), *s, dec(y()), *t);
            Val::bool(o == *want || (*or_eq && o == Ordering::Equal))
        }
        Op::ConcatText => {
            let (a, b) = (text(x()), text(y()));
            if !concat_is_nfc(b) {
                return Err(Error::Unsupported(
                    "normalizing a concatenation to NFC".into(),
                ));
            }
            Val::Text(Arc::from(format!("{a}{b}")))
        }
        Op::ConcatBytes => {
            let mut out = bytes(x()).to_vec();
            out.extend_from_slice(bytes(y()));
            Val::Bytes(Arc::from(out))
        }
        Op::Checked(inner) => {
            return Ok(Ok(match apply(inner, xs)? {
                Ok(v) => Val::some(v),
                Err(_) => Val::none(),
            }));
        }
        Op::Same => x().clone(),
        Op::IntToInt(w) => match w.fit(i128::from(x().int())) {
            Some(v) => Val::Int(v),
            None => return Ok(Err(Fault::OutOfRange)),
        },
        Op::IntToDec(s) => {
            match Wide::from_i128(i128::from(x().int()))
                .scale(u32::from(*s))
                .and_then(Wide::digits38)
            {
                Some(v) => Val::Decimal(v),
                None => return Ok(Err(Fault::OutOfRange)),
            }
        }
        Op::IntToF32 => Val::F32(F32::new(x().int() as f32)),
        Op::IntToF64 => Val::F64(F64::new(x().int() as f64)),
        Op::F32ToF64 => Val::F64(F64::new(float(x()))),
        Op::F64ToF32 => {
            let v = float(x());
            if v.is_finite() && v.abs() > f64::from(f32::MAX) {
                return Ok(Err(Fault::OutOfRange));
            }
            Val::F32(F32::new(v as f32))
        }
        Op::Rescale { from, to } => match rescaled(dec(x()), *from, *to).digits38() {
            Some(v) => Val::Decimal(v),
            None => return Ok(Err(Fault::ArithmeticOverflow)),
        },
        Op::Round { from, to } => {
            let d = 10i128.pow(u32::from(from - to));
            Val::Decimal(Rounding::of(xs[2]).div(dec(x()), d))
        }
        Op::BytesLen => Val::Int(match x() {
            Val::Text(t) => t.len() as i64,
            other => bytes(other).len() as i64,
        }),
        Op::ScalarLen => Val::Int(text(x()).chars().count() as i64),
        Op::StartsWith => Val::bool(text(x()).starts_with(text(y()))),
        Op::EndsWith => Val::bool(text(x()).ends_with(text(y()))),
        Op::Contains => Val::bool(text(x()).contains(text(y()))),
        Op::ToBytes => Val::Bytes(Arc::from(text(x()).as_bytes())),
        Op::Utf8 => match std::str::from_utf8(bytes(x())) {
            Ok(t) if t.is_ascii() => Val::some(Val::text(t)),
            Ok(_) => {
                return Err(Error::Unsupported("normalizing decoded text to NFC".into()));
            }
            Err(_) => Val::none(),
        },
        Op::Duration(per) => match x().int().checked_mul(*per) {
            Some(v) => Val::Duration(v),
            None => return Ok(Err(Fault::ArithmeticOverflow)),
        },
        Op::Period { months, per } => {
            let n = x()
                .int()
                .checked_mul(*per)
                .and_then(|n| i32::try_from(n).ok());
            match (n, months) {
                (None, _) => return Ok(Err(Fault::OutOfRange)),
                (Some(n), true) => Val::Period { months: n, days: 0 },
                (Some(n), false) => Val::Period { months: 0, days: n },
            }
        }
    };
    Ok(Ok(v))
}
