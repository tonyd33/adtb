use adtb_lang::kernel::Prim;
use adtb_lang::types::{Base, Ty};

use crate::big::Big;
use crate::eval::{Abort, Fault, Interpreter, R};
use crate::num::{
    Rounding, dec_add, dec_div, dec_mul, dec_rescale, dec_round, digits38, float_scaled, in_range,
};
use crate::textual;
use crate::time::{MICROS_PER_DAY, add_period, at_day, date_in_range};
use crate::value::{Value, canon32, canon64};
use crate::{unicode, zone};

fn fault<T>(f: Fault) -> R<T> {
    Err(Abort::Fault(f))
}

fn int_base(ty: &Ty) -> Base {
    match ty {
        Ty::Base(b @ (Base::I8 | Base::I16 | Base::I32 | Base::I64)) => *b,
        t => panic!("an integer operation at {t:?}"),
    }
}

fn int_at(v: i128, ty: &Ty) -> R<Value> {
    in_range(v, int_base(ty))
        .map(Value::Int)
        .ok_or(Abort::Fault(Fault::ArithmeticOverflow))
}

fn checked(v: R<Value>) -> R<Value> {
    match v {
        Ok(v) => Ok(Value::some(v)),
        Err(Abort::Fault(_)) => Ok(Value::none()),
        Err(e) => Err(e),
    }
}

fn i64_or(v: Option<i64>) -> R<i64> {
    v.ok_or(Abort::Fault(Fault::ArithmeticOverflow))
}

fn i32_or(v: Option<i32>) -> R<i32> {
    v.ok_or(Abort::Fault(Fault::ArithmeticOverflow))
}

fn text(v: &Value) -> &str {
    match v {
        Value::Text(t) => t,
        other => panic!("expected text, found {other:?}"),
    }
}

fn rounding(v: &Value) -> Rounding {
    Rounding::from_ctor(v.tag().expect("a `Rounding` constructor"))
}

/// The type an `Option` type holds.
fn option_arg(ty: &Ty) -> &Ty {
    match ty {
        Ty::Data { args, .. } if args.len() == 1 => &args[0],
        t => panic!("expected an `Option` type, found {t:?}"),
    }
}

/// Applies `+`, `-`, `*`, `/` or `%` at result type `ty`.
fn arith(p: Prim, a: &Value, b: &Value, ty: &Ty) -> R<Value> {
    use Value::*;
    match (a, b) {
        (Int(x), Int(y)) => {
            let (x, y) = (i128::from(*x), i128::from(*y));
            match p {
                Prim::Add | Prim::CheckedAdd => int_at(x + y, ty),
                Prim::Sub | Prim::CheckedSub => int_at(x - y, ty),
                Prim::Mul | Prim::CheckedMul => int_at(x * y, ty),
                Prim::Div | Prim::CheckedDiv | Prim::Rem | Prim::CheckedRem if y == 0 => {
                    fault(Fault::DivisionByZero)
                }
                Prim::Div | Prim::CheckedDiv => int_at(x / y, ty),
                Prim::Rem | Prim::CheckedRem => int_at(x % y, ty),
                _ => panic!("{p:?} on integers"),
            }
        }
        (Decimal(x, s), Decimal(y, t)) => {
            let (v, u) = match p {
                Prim::Add | Prim::CheckedAdd => dec_add(*x, *s, *y, *t, false)?,
                Prim::Sub | Prim::CheckedSub => dec_add(*x, *s, *y, *t, true)?,
                Prim::Mul | Prim::CheckedMul => dec_mul(*x, *s, *y, *t)?,
                _ => panic!("{p:?} on decimals"),
            };
            Ok(Decimal(v, u))
        }
        (F64(x), F64(y)) => Ok(F64(canon64(match p {
            Prim::Add => x + y,
            Prim::Sub => x - y,
            Prim::Mul => x * y,
            Prim::Div => x / y,
            _ => panic!("{p:?} on floats"),
        }))),
        (F32(x), F32(y)) => Ok(F32(canon32(match p {
            Prim::Add => x + y,
            Prim::Sub => x - y,
            Prim::Mul => x * y,
            Prim::Div => x / y,
            _ => panic!("{p:?} on floats"),
        }))),
        (Duration(x), Duration(y)) => Ok(Duration(i64_or(match p {
            Prim::Add => x.checked_add(*y),
            _ => x.checked_sub(*y),
        })?)),
        (Duration(x), Int(n)) => match p {
            Prim::Mul => Ok(Duration(i64_or(x.checked_mul(*n))?)),
            _ if *n == 0 => fault(Fault::DivisionByZero),
            _ => Ok(Duration(i64_or(x.checked_div(*n))?)),
        },
        (Period { months, days }, Period { months: m, days: d }) => {
            let (months, days) = match p {
                Prim::Add => (months.checked_add(*m), days.checked_add(*d)),
                _ => (months.checked_sub(*m), days.checked_sub(*d)),
            };
            Ok(Period {
                months: i32_or(months)?,
                days: i32_or(days)?,
            })
        }
        (Period { months, days }, Int(n)) => {
            let by = |c: i32| i32_or(i32::try_from(i128::from(c) * i128::from(*n)).ok());
            Ok(Period {
                months: by(*months)?,
                days: by(*days)?,
            })
        }
        (Instant(i), Duration(d)) => Ok(Instant(i64_or(match p {
            Prim::Add => i.checked_add(*d),
            _ => i.checked_sub(*d),
        })?)),
        (DateTime(i), Duration(d)) => Ok(DateTime(i64_or(match p {
            Prim::Add => i.checked_add(*d),
            _ => i.checked_sub(*d),
        })?)),
        (Instant(x), Instant(y)) | (DateTime(x), DateTime(y)) => {
            Ok(Duration(i64_or(x.checked_sub(*y))?))
        }
        (Date(x), Date(y)) => Ok(Duration(i64_or((x - y).checked_mul(MICROS_PER_DAY))?)),
        (Zoned(c, z), Duration(d)) => {
            let i = zone::to_instant(*c, z)?;
            let moved = i64_or(match p {
                Prim::Add => i.checked_add(*d),
                _ => i.checked_sub(*d),
            })?;
            Ok(Zoned(zone::in_zone(moved, z)?, z.clone()))
        }
        _ => panic!("{p:?} on {a:?} and {b:?}"),
    }
}

fn neg(v: &Value, ty: &Ty) -> R<Value> {
    use Value::*;
    match v {
        Int(x) => int_at(-i128::from(*x), ty),
        Decimal(x, s) => Ok(Decimal(-x, *s)),
        F64(x) => Ok(F64(canon64(-x))),
        F32(x) => Ok(F32(canon32(-x))),
        Duration(x) => Ok(Duration(i64_or(x.checked_neg())?)),
        Period { months, days } => Ok(Period {
            months: i32_or(months.checked_neg())?,
            days: i32_or(days.checked_neg())?,
        }),
        _ => panic!("`-` on {v:?}"),
    }
}

fn to_float64(v: &Value) -> f64 {
    match v {
        Value::Int(x) => *x as f64,
        Value::Decimal(d, s) => format!("{d}e-{s}")
            .parse()
            .expect("a decimal reads as a float"),
        Value::F32(f) => f64::from(*f),
        Value::F64(f) => *f,
        other => panic!("`f64` of {other:?}"),
    }
}

/// Converts `v` to numeric type `to`; `args` are the conversion's further arguments.
fn convert(v: &Value, args: &[Value], to: &Ty) -> R<Value> {
    let range = |b: Option<i128>| b.ok_or(Abort::Fault(Fault::OutOfRange));
    match to {
        Ty::Base(b @ (Base::I8 | Base::I16 | Base::I32 | Base::I64)) => {
            let wide: i128 = match v {
                Value::Int(x) => i128::from(*x),
                Value::Decimal(d, s) => range(dec_round(*d, *s, 0, rounding(&args[0])).to_i128())?,
                Value::F32(f) => {
                    range(float_scaled(f64::from(*f), 0, rounding(&args[0]))?.to_i128())?
                }
                Value::F64(f) => range(float_scaled(*f, 0, rounding(&args[0]))?.to_i128())?,
                other => panic!("an integer conversion of {other:?}"),
            };
            in_range(wide, *b)
                .map(Value::Int)
                .ok_or(Abort::Fault(Fault::OutOfRange))
        }
        Ty::Base(Base::Decimal(s)) => {
            let d = match v {
                Value::Int(x) => Big::from_i128(i128::from(*x)).mul(&Big::pow10(u32::from(*s))),
                Value::F32(f) => float_scaled(f64::from(*f), *s, rounding(&args[1]))?,
                Value::F64(f) => float_scaled(*f, *s, rounding(&args[1]))?,
                other => panic!("a decimal conversion of {other:?}"),
            };
            range(digits38(&d)).map(|d| Value::Decimal(d, *s))
        }
        Ty::Base(Base::F64) => Ok(Value::F64(canon64(to_float64(v)))),
        Ty::Base(Base::F32) => {
            let f = match v {
                Value::Int(x) => *x as f32,
                Value::Decimal(d, s) => format!("{d}e-{s}")
                    .parse()
                    .expect("a decimal reads as a float"),
                Value::F64(f) => {
                    let g = *f as f32;
                    if g.is_infinite() && f.is_finite() {
                        return fault(Fault::OutOfRange);
                    }
                    g
                }
                other => panic!("`f32` of {other:?}"),
            };
            Ok(Value::F32(canon32(f)))
        }
        t => panic!("a numeric conversion to {t:?}"),
    }
}

fn span_of(v: &Value) -> (&Value, &Value) {
    (
        v.field("start").expect("a span has a start"),
        v.field("end").expect("a span has an end"),
    )
}

pub(crate) fn parse_uuid(t: &str) -> Option<Vec<u8>> {
    let groups: Vec<&str> = t.split('-').collect();
    if groups.iter().map(|g| g.len()).collect::<Vec<_>>() != [8, 4, 4, 4, 12] {
        return None;
    }
    let hex: String = groups.concat();
    (0..16)
        .map(|i| {
            let pair = hex.get(2 * i..2 * i + 2)?;
            pair.chars()
                .all(|c| c.is_ascii_hexdigit())
                .then(|| u8::from_str_radix(pair, 16).ok())
                .flatten()
        })
        .collect()
}

/// Applies primitive `p` to its argument values `vs`, of types `tys`, at the node's type `ty`.
#[expect(
    clippy::too_many_lines,
    reason = "one arm per primitive; each family of primitives would have to be a function"
)]
pub fn apply(it: &Interpreter, p: Prim, vs: Vec<Value>, ty: &Ty, tys: &[Ty]) -> R<Value> {
    use Value::*;
    let v = &vs;
    match p {
        Prim::Add | Prim::Sub | Prim::Mul | Prim::Div | Prim::Rem => arith(p, &v[0], &v[1], ty),
        Prim::CheckedAdd
        | Prim::CheckedSub
        | Prim::CheckedMul
        | Prim::CheckedDiv
        | Prim::CheckedRem => checked(arith(p, &v[0], &v[1], option_arg(ty))),
        Prim::Neg => neg(&v[0], ty),
        Prim::CheckedNeg => checked(neg(&v[0], option_arg(ty))),
        Prim::Eq => Ok(Value::bool(v[0] == v[1])),
        Prim::Ne => Ok(Value::bool(v[0] != v[1])),
        Prim::Lt => Ok(Value::bool(v[0] < v[1])),
        Prim::Le => Ok(Value::bool(v[0] <= v[1])),
        Prim::Gt => Ok(Value::bool(v[0] > v[1])),
        Prim::Ge => Ok(Value::bool(v[0] >= v[1])),
        Prim::Concat => match (&v[0], &v[1]) {
            (Text(a), Text(b)) => Ok(Text(unicode::nfc(&format!("{a}{b}")))),
            (Bytes(a), Bytes(b)) => Ok(Bytes([a.as_slice(), b].concat())),
            _ => panic!("`++` on {v:?}"),
        },
        Prim::DecimalDiv => {
            let (Decimal(a, s), Decimal(b, t), Ty::Base(Base::Decimal(u))) = (&v[0], &v[1], ty)
            else {
                panic!("`div` on decimals")
            };
            Ok(Decimal(dec_div(*a, *s, *b, *t, *u, rounding(&v[3]))?, *u))
        }
        Prim::Round => {
            let (Decimal(a, s), Ty::Base(Base::Decimal(u))) = (&v[0], ty) else {
                panic!("`round` on a decimal")
            };
            let r = dec_round(*a, *s, *u, rounding(&v[2]));
            Ok(Decimal(r.to_i128().expect("rounding narrows"), *u))
        }
        Prim::Rescale => {
            let (Decimal(a, s), Ty::Base(Base::Decimal(u))) = (&v[0], ty) else {
                panic!("`rescale` on a decimal")
            };
            Ok(Decimal(dec_rescale(*a, *s, *u)?, *u))
        }
        Prim::Abs => match &v[0] {
            Int(x) => int_at(i128::from(*x).abs(), ty),
            Decimal(x, s) => Ok(Decimal(x.abs(), *s)),
            F64(x) => Ok(F64(x.abs())),
            F32(x) => Ok(F32(x.abs())),
            other => panic!("`abs` of {other:?}"),
        },
        Prim::Convert { checked: false } => convert(&v[0], &v[1..], ty),
        Prim::Convert { checked: true } => checked(convert(&v[0], &v[1..], option_arg(ty))),
        Prim::Wrap | Prim::Unwrap => Ok(v[0].clone()),
        Prim::KeyValue => match &v[0] {
            Key(k) => Ok((**k).clone()),
            other => panic!("`value` of {other:?}"),
        },
        Prim::ParseUuid => parse_uuid(text(&v[0]))
            .map(Bytes)
            .ok_or(Abort::Fault(Fault::InvalidText)),
        Prim::ToText => Ok(Text(textual::to_text(it.schema, &v[0], &tys[0]))),
        Prim::Print => Ok(Text(textual::print(it.schema, &v[0], &tys[0]))),
        Prim::ReadFile { .. } => crate::eval::not_yet("a file scan"),
        Prim::Parse => Ok(
            match textual::parse(it.schema, text(&v[0]), option_arg(ty)) {
                Ok(x) => Value::some(x),
                Err(_) => Value::none(),
            },
        ),
        Prim::BytesLen => match &v[0] {
            Text(t) => Ok(Int(t.len() as i64)),
            Bytes(b) => Ok(Int(b.len() as i64)),
            other => panic!("`bytes_len` of {other:?}"),
        },
        Prim::ScalarLen => Ok(Int(text(&v[0]).chars().count() as i64)),
        Prim::GraphemeLen => Ok(Int(unicode::grapheme_len(text(&v[0])) as i64)),
        Prim::StartsWith => Ok(Value::bool(text(&v[0]).starts_with(text(&v[1])))),
        Prim::EndsWith => Ok(Value::bool(text(&v[0]).ends_with(text(&v[1])))),
        Prim::Contains => Ok(Value::bool(text(&v[0]).contains(text(&v[1])))),
        Prim::Matches => Ok(Value::bool(it.compiled.matches(text(&v[0]), text(&v[1])))),
        Prim::Lower => Ok(Text(unicode::lower(text(&v[0])))),
        Prim::Upper => Ok(Text(unicode::upper(text(&v[0])))),
        Prim::Fold => Ok(Text(unicode::fold(text(&v[0])))),
        Prim::Collate => Ok(SortKey(it.compiled.collate(text(&v[0]), text(&v[1])))),
        Prim::Utf8 => match &v[0] {
            Bytes(b) => Ok(match String::from_utf8(b.clone()) {
                Ok(t) => Value::some(Text(unicode::nfc(&t))),
                Err(_) => Value::none(),
            }),
            other => panic!("`utf8` of {other:?}"),
        },
        Prim::Bytes => Ok(Bytes(text(&v[0]).as_bytes().to_vec())),
        Prim::Micros | Prim::Millis | Prim::Seconds | Prim::Minutes | Prim::Hours => {
            let unit: i64 = match p {
                Prim::Micros => 1,
                Prim::Millis => 1_000,
                Prim::Seconds => 1_000_000,
                Prim::Minutes => 60_000_000,
                _ => 3_600_000_000,
            };
            let Int(n) = v[0] else {
                panic!("a length of time from an integer")
            };
            Ok(Duration(i64_or(n.checked_mul(unit))?))
        }
        Prim::Days | Prim::Weeks | Prim::Months | Prim::Years => {
            let Int(n) = v[0] else {
                panic!("a period from an integer")
            };
            let count = |k: i128| {
                i32::try_from(i128::from(n) * k).map_err(|_| Abort::Fault(Fault::OutOfRange))
            };
            Ok(match p {
                Prim::Days => Period {
                    months: 0,
                    days: count(1)?,
                },
                Prim::Weeks => Period {
                    months: 0,
                    days: count(7)?,
                },
                Prim::Months => Period {
                    months: count(1)?,
                    days: 0,
                },
                _ => Period {
                    months: count(12)?,
                    days: 0,
                },
            })
        }
        Prim::AddPeriod => {
            let Period { months, days } = v[1] else {
                panic!("`add_period` adds a period")
            };
            let reject = v[2].tag() == Some(1);
            let shifted = |date: i64| -> R<Option<i64>> {
                match add_period(date, months, days, !reject) {
                    Some(d) if date_in_range(d) => Ok(Some(d)),
                    Some(_) => fault(Fault::ArithmeticOverflow),
                    None => Ok(None),
                }
            };
            let out = match &v[0] {
                Date(d) => shifted(*d)?.map(Date),
                DateTime(t) => {
                    let (d, rest) = (t.div_euclid(MICROS_PER_DAY), t.rem_euclid(MICROS_PER_DAY));
                    match shifted(d)? {
                        Some(nd) => Some(DateTime(i64_or(at_day(nd, rest))?)),
                        None => None,
                    }
                }
                Zoned(c, z) => {
                    let (d, rest) = (c.div_euclid(MICROS_PER_DAY), c.rem_euclid(MICROS_PER_DAY));
                    match shifted(d)? {
                        Some(nd) => {
                            let civil = i64_or(at_day(nd, rest))?;
                            zone::resolve(civil, z, !reject)?.map(|(c, _)| Zoned(c, z.clone()))
                        }
                        None => None,
                    }
                }
                other => panic!("`add_period` on {other:?}"),
            };
            Ok(match (reject, out) {
                (true, Some(x)) => Value::some(x),
                (true, None) => Value::none(),
                (false, Some(x)) => x,
                (false, None) => panic!("`Clamp` always reaches a date"),
            })
        }
        Prim::Zoned => {
            let DateTime(c) = v[0] else {
                panic!("`zoned` of a date and time")
            };
            let z = text(&v[1]);
            let reject = v[2].tag() == Some(1);
            let r = zone::resolve(c, z, !reject)?.map(|(c, _)| Zoned(c, z.to_string()));
            Ok(match (reject, r) {
                (true, Some(x)) => Value::some(x),
                (true, None) => Value::none(),
                (false, Some(x)) => x,
                (false, None) => panic!("`Clamp` always resolves a civil time"),
            })
        }
        Prim::InZone => {
            let Instant(i) = v[0] else {
                panic!("`in_zone` of an instant")
            };
            let z = text(&v[1]);
            Ok(Zoned(zone::in_zone(i, z)?, z.to_string()))
        }
        Prim::ToInstant => match &v[0] {
            Zoned(c, z) => Ok(Instant(zone::to_instant(*c, z)?)),
            other => panic!("`to_instant` of {other:?}"),
        },
        Prim::Span => {
            if v[0] > v[1] {
                return fault(Fault::InvalidSpan);
            }
            Ok(Ctor(
                0,
                vec![("start".into(), v[0].clone()), ("end".into(), v[1].clone())],
            ))
        }
        Prim::Start => Ok(span_of(&v[0]).0.clone()),
        Prim::End => Ok(span_of(&v[0]).1.clone()),
        Prim::Overlaps => {
            let ((xs, xe), (ys, ye)) = (span_of(&v[0]), span_of(&v[1]));
            Ok(Value::bool(xs < ye && ys < xe))
        }
        Prim::Covers => {
            let (xs, xe) = span_of(&v[0]);
            if it.is_prelude(&tys[1], "Span") {
                let (ys, ye) = span_of(&v[1]);
                Ok(Value::bool(xs <= ys && ye <= xe))
            } else {
                Ok(Value::bool(*xs <= v[1] && v[1] < *xe))
            }
        }
    }
}
