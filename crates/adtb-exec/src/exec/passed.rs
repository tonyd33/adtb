use std::ops::ControlFlow;
use std::sync::Arc;

use crate::heap::Version;
use crate::key::{F32, F64, Value};
use crate::runtime::Runtime;

use super::memory::Grant;
use super::spill::{Distinct, Spill};
use super::value::Val;
use super::{Error, Rot};

/// The deleted versions a read of the past passed in a current heap, by inserter, deleter and
/// row, held in a set of the call's grant that spills.
pub(super) struct Passed {
    held: Distinct,
    /// The deleted versions of the history heap whose judgement waits for the end of the scan,
    /// once `held` has spilled.
    later: Distinct,
}

impl Passed {
    pub(super) fn new(grant: &Arc<Grant>) -> Passed {
        Passed {
            held: Distinct::new(grant),
            later: Distinct::new(grant),
        }
    }

    /// Hold `v`, a deleted version of the current heap.
    pub(super) fn hold<R: Runtime>(&mut self, v: &Version, sp: &Spill<R>) -> Result<(), Error> {
        self.held.insert(version(v), sp)
    }

    /// Returns whether `v`, a deleted version of the history heap, is passed on now: not when a
    /// version with its writers and row was held, nor when that is known only once the scan
    /// ends, at [`Passed::rest`].
    pub(super) fn admits<R: Runtime>(&mut self, v: &Version, sp: &Spill<R>) -> Result<bool, Error> {
        let key = version(v);
        match self.held.holds(&key) {
            Some(held) => Ok(!held),
            None => {
                self.later.insert(key, sp)?;
                Ok(false)
            }
        }
    }

    /// Pass the row of each version [`Passed::admits`] left for the end of the scan and no held
    /// version matches to `each`, until it breaks.
    pub(super) fn rest<R: Runtime>(
        self,
        sp: &Spill<R>,
        mut each: impl FnMut(Vec<Value>) -> Result<ControlFlow<()>, Error>,
    ) -> Result<ControlFlow<()>, Error> {
        if self.later.is_empty() {
            return Ok(ControlFlow::Continue(()));
        }
        let mut held = self.held.merge(sp)?;
        let mut later = self.later.merge(sp)?;
        let mut head = held.pop()?;
        while let Some(v) = later.pop()? {
            while head.as_ref().is_some_and(|h| *h < v) {
                head = held.pop()?;
            }
            if head.as_ref() == Some(&v) {
                continue;
            }
            if each(row(v)?)?.is_break() {
                return Ok(ControlFlow::Break(()));
            }
        }
        Ok(ControlFlow::Continue(()))
    }
}

/// Returns `v`'s writers and row as one value, equal for two versions exactly when their writers
/// and rows are.
fn version(v: &Version) -> Val {
    Val::record(vec![
        Val::Bytes(Arc::from(v.header.writers.encode().as_slice())),
        Val::List(v.row.iter().map(stored).collect()),
    ])
}

/// Returns the row of a value [`version`] returned.
fn row(v: Val) -> Result<Vec<Value>, Error> {
    match v {
        Val::Record(fs) if fs.len() == 2 => match &fs[1] {
            Val::List(row) => row.iter().map(unstored).collect(),
            other => Err(unheld(other)),
        },
        other => Err(unheld(&other)),
    }
}

fn unheld(_: &Val) -> Error {
    Rot::SpillUndecoded.into()
}

fn tagged(tag: u32, fields: Vec<Val>) -> Val {
    Val::ctor(tag, fields)
}

fn stored(v: &Value) -> Val {
    let int = |tag, i: i64| tagged(tag, vec![Val::Int(i)]);
    match v {
        Value::I8(i) => int(0, i64::from(*i)),
        Value::I16(i) => int(1, i64::from(*i)),
        Value::I32(i) => int(2, i64::from(*i)),
        Value::I64(i) => int(3, *i),
        Value::F32(x) => int(4, i64::from(x.get().to_bits())),
        Value::F64(x) => tagged(
            5,
            vec![Val::Bytes(Arc::from(x.get().to_bits().to_le_bytes()))],
        ),
        Value::Decimal(d) => tagged(6, vec![Val::Decimal(*d)]),
        Value::Text(t) => tagged(7, vec![Val::Text(Arc::from(t.as_str()))]),
        Value::Bytes(b) => tagged(8, vec![Val::Bytes(Arc::from(b.as_slice()))]),
        Value::SortKey(b) => tagged(9, vec![Val::Bytes(Arc::from(b.as_slice()))]),
        Value::Instant(i) => int(10, *i),
        Value::Date(d) => int(11, i64::from(*d)),
        Value::Time(i) => int(12, *i),
        Value::DateTime(i) => int(13, *i),
        Value::ZonedDateTime { local, zone } => tagged(
            14,
            vec![Val::Int(*local), Val::Text(Arc::from(zone.as_str()))],
        ),
        Value::Duration(i) => int(15, *i),
        Value::Period { months, days } => tagged(
            16,
            vec![Val::Int(i64::from(*months)), Val::Int(i64::from(*days))],
        ),
        Value::Record(fs) => tagged(17, fs.iter().map(stored).collect()),
        Value::Sum { ctor, fields } => tagged(
            18,
            vec![
                Val::Int(i64::from(*ctor)),
                Val::List(fields.iter().map(stored).collect()),
            ],
        ),
    }
}

fn unstored(v: &Val) -> Result<Value, Error> {
    let bad = || unheld(v);
    let Val::Ctor(tag, fs) = v else {
        return Err(bad());
    };
    let int = |i: usize| match fs.get(i) {
        Some(Val::Int(n)) => Ok(*n),
        _ => Err(bad()),
    };
    let narrow = |n: i64| i32::try_from(n).map_err(|_| bad());
    let bytes = || match fs.first() {
        Some(Val::Bytes(b)) => Ok(b.to_vec()),
        _ => Err(bad()),
    };
    let text = |i: usize| match fs.get(i) {
        Some(Val::Text(t)) => Ok(t.to_string()),
        _ => Err(bad()),
    };
    Ok(match tag {
        0 => Value::I8(i8::try_from(int(0)?).map_err(|_| bad())?),
        1 => Value::I16(i16::try_from(int(0)?).map_err(|_| bad())?),
        2 => Value::I32(narrow(int(0)?)?),
        3 => Value::I64(int(0)?),
        4 => Value::F32(F32::new(f32::from_bits(
            u32::try_from(int(0)?).map_err(|_| bad())?,
        ))),
        5 => {
            let b: [u8; 8] = bytes()?.try_into().map_err(|_| bad())?;
            Value::F64(F64::new(f64::from_bits(u64::from_le_bytes(b))))
        }
        6 => match fs.first() {
            Some(Val::Decimal(d)) => Value::Decimal(*d),
            _ => return Err(bad()),
        },
        7 => Value::Text(text(0)?),
        8 => Value::Bytes(bytes()?),
        9 => Value::SortKey(bytes()?),
        10 => Value::Instant(int(0)?),
        11 => Value::Date(narrow(int(0)?)?),
        12 => Value::Time(int(0)?),
        13 => Value::DateTime(int(0)?),
        14 => Value::ZonedDateTime {
            local: int(0)?,
            zone: text(1)?,
        },
        15 => Value::Duration(int(0)?),
        16 => Value::Period {
            months: narrow(int(0)?)?,
            days: narrow(int(1)?)?,
        },
        17 => Value::Record(fs.iter().map(unstored).collect::<Result<_, _>>()?),
        18 => match fs.get(1) {
            Some(Val::List(fields)) => Value::Sum {
                ctor: u32::try_from(int(0)?).map_err(|_| bad())?,
                fields: fields.iter().map(unstored).collect::<Result<_, _>>()?,
            },
            _ => return Err(bad()),
        },
        _ => return Err(bad()),
    })
}
