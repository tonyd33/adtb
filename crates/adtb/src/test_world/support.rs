//! Convert values between the reference interpreter, the executor and the heaps.

use std::sync::Arc;

use crate::catalog::layout::{is_record, key_value_ty, repr};
use crate::exec::{self, Val};
use crate::key::{F32, F64, Value as KValue};
use adtb_lang::schema::Schema;
use adtb_lang::types::{Base, RelId, Ty};
use adtb_reference::Value;

/// Returns the constructors of a sum type, each with its header and own fields.
fn ctors_of(schema: &Schema, ty: &Ty) -> Vec<(String, Vec<(String, Ty)>)> {
    match repr(schema, ty) {
        Ty::Data { def, args, .. } => {
            let d = schema.ty(def);
            let header: Vec<(String, Ty)> = d
                .header()
                .iter()
                .map(|f| (f.name.clone(), f.ty.subst(&args)))
                .collect();
            d.ctors()
                .iter()
                .map(|c| {
                    let mut fs = header.clone();
                    fs.extend(c.fields.iter().map(|f| (f.name.clone(), f.ty.subst(&args))));
                    (c.name.clone(), fs)
                })
                .collect()
        }
        Ty::Sum { ctors, .. } => ctors,
        Ty::Tag(t) => ctors_of(schema, &t)
            .into_iter()
            .map(|(n, _)| (n, Vec::new()))
            .collect(),
        t => panic!("{t:?} is not a sum"),
    }
}

/// Returns reference value `v` of type `ty` as a heap stores it.
pub fn stored(schema: &Schema, ty: &Ty, v: &Value) -> KValue {
    let t = repr(schema, ty);
    match (&t, v) {
        (Ty::Key { rel, .. }, Value::Key(inner)) => {
            stored(schema, &key_value_ty(schema, *rel), inner)
        }
        (Ty::Base(b), v) => match (b, v) {
            (Base::I8, Value::Int(x)) => KValue::I8(*x as i8),
            (Base::I16, Value::Int(x)) => KValue::I16(*x as i16),
            (Base::I32, Value::Int(x)) => KValue::I32(*x as i32),
            (Base::I64, Value::Int(x)) => KValue::I64(*x),
            (Base::Decimal(_), Value::Decimal(d, _)) => KValue::Decimal(*d),
            (Base::F32, Value::F32(x)) => KValue::F32(F32::new(*x)),
            (Base::F64, Value::F64(x)) => KValue::F64(F64::new(*x)),
            (Base::Text, Value::Text(x)) => KValue::Text(x.clone()),
            (Base::Bytes, Value::Bytes(x)) => KValue::Bytes(x.clone()),
            (Base::SortKey, Value::SortKey(x)) => KValue::SortKey(x.clone()),
            (Base::Instant, Value::Instant(x)) => KValue::Instant(*x),
            (Base::Date, Value::Date(x)) => KValue::Date(*x as i32),
            (Base::Time, Value::Time(x)) => KValue::Time(*x),
            (Base::DateTime, Value::DateTime(x)) => KValue::DateTime(*x),
            (Base::Duration, Value::Duration(x)) => KValue::Duration(*x),
            (Base::ZonedDateTime, Value::Zoned(l, z)) => KValue::ZonedDateTime {
                local: *l,
                zone: z.clone(),
            },
            (Base::Period, Value::Period { months, days }) => KValue::Period {
                months: *months,
                days: *days,
            },
            (b, v) => panic!("{v:?} is not a {b:?}"),
        },
        (Ty::Record(fs), Value::Record(vs)) => KValue::Record(
            fs.iter()
                .zip(vs)
                .map(|((_, t), (_, v))| stored(schema, t, v))
                .collect(),
        ),
        (_, Value::Ctor(c, vs)) => {
            let cs = ctors_of(schema, &t);
            let fields: Vec<KValue> = cs[*c as usize]
                .1
                .iter()
                .map(|(n, f)| {
                    let x = &vs.iter().find(|(m, _)| m == n).unwrap().1;
                    stored(schema, f, x)
                })
                .collect();
            if is_record(schema, &t) {
                KValue::Record(fields)
            } else {
                KValue::Sum { ctor: *c, fields }
            }
        }
        (t, v) => panic!("{v:?} is not a {t:?}"),
    }
}

/// Returns reference value `v` of type `ty` as the executor holds it.
pub fn to_val(schema: &Schema, ty: &Ty, v: &Value) -> Val {
    let t = repr(schema, ty);
    match (&t, v) {
        (Ty::Key { rel, .. }, Value::Key(inner)) => {
            to_val(schema, &key_value_ty(schema, *rel), inner)
        }
        (_, Value::Int(x)) => Val::Int(*x),
        (_, Value::Decimal(d, _)) => Val::Decimal(*d),
        (_, Value::F32(x)) => Val::F32(exec::F32::new(*x)),
        (_, Value::F64(x)) => Val::F64(exec::F64::new(*x)),
        (_, Value::Text(x)) => Val::text(x),
        (_, Value::Bytes(x)) => Val::Bytes(Arc::from(x.as_slice())),
        (_, Value::SortKey(x)) => Val::SortKey(Arc::from(x.as_slice())),
        (_, Value::Instant(x)) => Val::Instant(*x),
        (_, Value::Date(x)) => Val::Date(*x as i32),
        (_, Value::Time(x)) => Val::Time(*x),
        (_, Value::DateTime(x)) => Val::DateTime(*x),
        (_, Value::Duration(x)) => Val::Duration(*x),
        (_, Value::Zoned(l, z)) => Val::Zoned {
            local: *l,
            zone: Arc::from(z.as_str()),
        },
        (_, Value::Period { months, days }) => Val::Period {
            months: *months,
            days: *days,
        },
        (Ty::Record(fs), Value::Record(vs)) => Val::record(
            fs.iter()
                .zip(vs)
                .map(|((_, t), (_, v))| to_val(schema, t, v))
                .collect(),
        ),
        (Ty::Set(e) | Ty::NonEmpty(e), Value::Set(vs)) => {
            Val::set(vs.iter().map(|v| to_val(schema, e, v)).collect())
        }
        (_, Value::Ctor(c, vs)) => {
            let cs = ctors_of(schema, &t);
            Val::ctor(
                *c,
                cs[*c as usize]
                    .1
                    .iter()
                    .map(|(n, f)| to_val(schema, f, &vs.iter().find(|(m, _)| m == n).unwrap().1))
                    .collect(),
            )
        }
        (t, v) => panic!("{v:?} is not a {t:?}"),
    }
}

/// Returns executor value `v` of type `ty` as the reference interpreter holds it.
pub fn to_ref(schema: &Schema, ty: &Ty, v: &Val) -> Value {
    let t = repr(schema, ty);
    match (&t, v) {
        (Ty::Key { rel, .. }, v) => Value::key(to_ref(schema, &key_value_ty(schema, *rel), v)),
        (Ty::Set(e) | Ty::NonEmpty(e), Val::List(xs)) => {
            Value::List(xs.iter().map(|x| to_ref(schema, e, x)).collect())
        }
        (Ty::Set(e) | Ty::NonEmpty(e), Val::Set(xs)) => {
            Value::Set(xs.iter().map(|x| to_ref(schema, e, x)).collect())
        }
        (Ty::Record(fs), Val::Record(xs)) => Value::Record(
            fs.iter()
                .zip(xs.iter())
                .map(|((n, t), x)| (n.clone(), to_ref(schema, t, x)))
                .collect(),
        ),
        (Ty::Base(b), v) => match (b, v) {
            (Base::Decimal(s), Val::Decimal(d)) => Value::Decimal(*d, *s),
            (_, Val::Int(x)) => Value::Int(*x),
            (_, Val::F32(x)) => Value::F32(x.get()),
            (_, Val::F64(x)) => Value::F64(x.get()),
            (_, Val::Text(x)) => Value::Text(x.to_string()),
            (_, Val::Bytes(x)) => Value::Bytes(x.to_vec()),
            (_, Val::SortKey(x)) => Value::SortKey(x.to_vec()),
            (_, Val::Instant(x)) => Value::Instant(*x),
            (_, Val::Date(x)) => Value::Date(i64::from(*x)),
            (_, Val::Time(x)) => Value::Time(*x),
            (_, Val::DateTime(x)) => Value::DateTime(*x),
            (_, Val::Duration(x)) => Value::Duration(*x),
            (_, Val::Zoned { local, zone }) => Value::Zoned(*local, zone.to_string()),
            (_, Val::Period { months, days }) => Value::Period {
                months: *months,
                days: *days,
            },
            (b, v) => panic!("{v:?} is not a {b:?}"),
        },
        (_, Val::Ctor(c, xs)) => {
            let cs = ctors_of(schema, &t);
            Value::Ctor(
                *c,
                cs[*c as usize]
                    .1
                    .iter()
                    .zip(xs.iter())
                    .map(|((n, f), x)| (n.clone(), to_ref(schema, f, x)))
                    .collect(),
            )
        }
        (t, v) => panic!("{v:?} is not a {t:?}"),
    }
}

/// Returns the fields of constructor `c` of relation `rel`, its header's first, each with its
/// type.
pub fn row_ty(schema: &Schema, rel: RelId, c: u32) -> Vec<(String, Ty)> {
    ctors_of(schema, &schema.relation(rel).elem)[c as usize]
        .1
        .clone()
}
