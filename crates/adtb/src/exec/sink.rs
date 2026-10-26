use std::cmp::Ordering;

use adtb_lang::types::Base;

use super::ir::AggOp;
use super::value::{F32, F64, Val};
use super::wide::Wide;
use super::{Error, Fault};

fn fits(b: Base, v: i128) -> Option<i64> {
    let (lo, hi): (i128, i128) = match b {
        Base::I8 => (i8::MIN.into(), i8::MAX.into()),
        Base::I16 => (i16::MIN.into(), i16::MAX.into()),
        Base::I32 => (i32::MIN.into(), i32::MAX.into()),
        _ => (i64::MIN.into(), i64::MAX.into()),
    };
    (lo..=hi).contains(&v).then_some(v as i64)
}

fn mistyped(want: &str, v: &Val) -> Error {
    Error::Internal(format!("an aggregate's range binds {want}, found {v:?}"))
}

fn float(v: &Val) -> Result<f64, Error> {
    match v {
        Val::F32(x) => Ok(f64::from(x.get())),
        Val::F64(x) => Ok(x.get()),
        other => Err(mistyped("floats", other)),
    }
}

fn sum_f32(ms: &[Val]) -> Result<f32, Error> {
    ms.iter().map(|m| Ok(float(m)? as f32)).sum()
}

fn sum_f64(ms: &[Val]) -> Result<f64, Error> {
    ms.iter().map(float).sum()
}

/// Returns aggregate `op` of `ms`, one value per binding of its range.
pub fn aggregate(op: AggOp, ms: Vec<Val>) -> Result<Val, Error> {
    let wrap = |w: bool, v: Option<Val>| match (w, v) {
        (true, Some(v)) => Ok(Val::some(v)),
        (true, None) => Ok(Val::none()),
        (false, Some(v)) => Ok(v),
        (false, None) => Err(Error::Internal(
            "an aggregate that is not an Option ranges over a nonempty range".into(),
        )),
    };
    let n = ms.len();
    match op {
        AggOp::Count => Ok(Val::Int(n as i64)),
        AggOp::SumInt(b) => {
            let mut total: i128 = 0;
            for m in &ms {
                let Val::Int(i) = m else {
                    return Err(mistyped("integers", m));
                };
                total += i128::from(*i);
            }
            Ok(Val::Int(
                fits(b, total).ok_or(Error::Fault(Fault::ArithmeticOverflow))?,
            ))
        }
        AggOp::SumDecimal => {
            let mut total = Wide::ZERO;
            for m in &ms {
                let Val::Decimal(d) = m else {
                    return Err(mistyped("decimals", m));
                };
                total = total
                    .add(Wide::from_i128(*d))
                    .ok_or(Error::Fault(Fault::ArithmeticOverflow))?;
            }
            Ok(Val::Decimal(
                total
                    .digits38()
                    .ok_or(Error::Fault(Fault::ArithmeticOverflow))?,
            ))
        }
        AggOp::SumF32 => Ok(Val::F32(F32::new(sum_f32(&ms)?))),
        AggOp::SumF64 => Ok(Val::F64(F64::new(sum_f64(&ms)?))),
        AggOp::Min { wrap: w } => wrap(w, ms.into_iter().min()),
        AggOp::Max { wrap: w } => wrap(w, ms.into_iter().max()),
        AggOp::AvgF32 { wrap: w } => {
            let v = if n > 0 {
                Some(Val::F32(F32::new(sum_f32(&ms)? / n as f32)))
            } else {
                None
            };
            wrap(w, v)
        }
        AggOp::AvgF64 { wrap: w } => {
            let v = if n > 0 {
                Some(Val::F64(F64::new(sum_f64(&ms)? / n as f64)))
            } else {
                None
            };
            wrap(w, v)
        }
    }
}

/// Returns the order of two key lists under directions `desc`.
pub fn cmp_keys(a: &[Val], b: &[Val], desc: &[bool]) -> Ordering {
    for ((x, y), d) in a.iter().zip(b).zip(desc) {
        let o = x.cmp(y);
        let o = if *d { o.reverse() } else { o };
        if o != Ordering::Equal {
            return o;
        }
    }
    Ordering::Equal
}

/// Returns `keyed`, each element with its keys, ordered by the keys and then by the element.
pub fn sorted(mut keyed: Vec<(Vec<Val>, Val)>, desc: &[bool]) -> Vec<(Vec<Val>, Val)> {
    keyed.sort_by(|(ka, a), (kb, b)| cmp_keys(ka, kb, desc).then_with(|| a.cmp(b)));
    keyed
}
