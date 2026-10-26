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

fn float(v: &Val) -> f64 {
    match v {
        Val::F32(x) => f64::from(x.get()),
        Val::F64(x) => x.get(),
        other => panic!("expected a float, found {other:?}"),
    }
}

/// Returns aggregate `op` of `ms`, one value per binding of its range.
pub fn aggregate(op: AggOp, ms: Vec<Val>) -> Result<Val, Error> {
    let wrap = |w: bool, v: Option<Val>| match (w, v) {
        (true, Some(v)) => Val::some(v),
        (true, None) => Val::none(),
        (false, Some(v)) => v,
        (false, None) => panic!("an aggregate over a nonempty range saw no binding"),
    };
    Ok(match op {
        AggOp::Count => Val::Int(ms.len() as i64),
        AggOp::SumInt(b) => {
            let total: i128 = ms.iter().map(|m| i128::from(m.int())).sum();
            Val::Int(fits(b, total).ok_or(Error::Fault(Fault::ArithmeticOverflow))?)
        }
        AggOp::SumDecimal => {
            let mut total = Wide::ZERO;
            for m in &ms {
                let Val::Decimal(d) = m else {
                    panic!("`sum` at a decimal type over {m:?}")
                };
                total = total
                    .add(Wide::from_i128(*d))
                    .ok_or(Error::Fault(Fault::ArithmeticOverflow))?;
            }
            Val::Decimal(
                total
                    .digits38()
                    .ok_or(Error::Fault(Fault::ArithmeticOverflow))?,
            )
        }
        AggOp::SumF32 => Val::F32(F32::new(ms.iter().map(|m| float(m) as f32).sum())),
        AggOp::SumF64 => Val::F64(F64::new(ms.iter().map(float).sum())),
        AggOp::Min { wrap: w } => wrap(w, ms.into_iter().min()),
        AggOp::Max { wrap: w } => wrap(w, ms.into_iter().max()),
        AggOp::AvgF32 { wrap: w } => {
            let n = ms.len();
            let v = (n > 0).then(|| {
                let s: f32 = ms.iter().map(|m| float(m) as f32).sum();
                Val::F32(F32::new(s / n as f32))
            });
            wrap(w, v)
        }
        AggOp::AvgF64 { wrap: w } => {
            let n = ms.len();
            let v = (n > 0).then(|| {
                let s: f64 = ms.iter().map(float).sum();
                Val::F64(F64::new(s / n as f64))
            });
            wrap(w, v)
        }
    })
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
