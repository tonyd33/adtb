//! Convert reference values to the values the executor holds.

use std::sync::Arc;

use adtb::catalog::layout::{key_value_ty, repr};
use adtb::exec::{self, Val};
use adtb_lang::schema::Schema;
use adtb_lang::types::Ty;
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
