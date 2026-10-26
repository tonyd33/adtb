use std::sync::Arc;

use adtb_lang::textual::{Scalar, Textual};
use adtb_lang::types::Base;

use super::time::zone;
use super::{F32, F64, Val};

impl Textual for Val {
    fn scalar(&self) -> Scalar<'_> {
        match self {
            Val::Int(x) => Scalar::Int(*x),
            Val::Decimal(d) => Scalar::Decimal(*d),
            Val::F32(f) => Scalar::F32(f.get()),
            Val::F64(f) => Scalar::F64(f.get()),
            Val::Text(t) => Scalar::Text(t),
            Val::Bytes(b) | Val::SortKey(b) => Scalar::Bytes(b),
            Val::Instant(t) => Scalar::Instant(*t),
            Val::Date(d) => Scalar::Date(i64::from(*d)),
            Val::Time(t) => Scalar::Time(*t),
            Val::DateTime(t) => Scalar::DateTime(*t),
            Val::Zoned { local, zone } => Scalar::Zoned {
                local: *local,
                zone,
            },
            Val::Duration(d) => Scalar::Duration(*d),
            Val::Period { months, days } => Scalar::Period {
                months: *months,
                days: *days,
            },
            v => panic!("expected a scalar, found {v:?}"),
        }
    }

    fn ctor(&self) -> u32 {
        Val::ctor_of(self)
    }

    fn fields(&self) -> impl Iterator<Item = &Val> {
        Val::fields(self).iter()
    }

    fn key(&self) -> &Val {
        self
    }

    fn elems(&self) -> impl Iterator<Item = &Val> {
        self.as_set().iter()
    }

    fn of_scalar(s: Scalar<'_>, at: Base) -> Val {
        match (s, at) {
            (Scalar::Int(x), _) => Val::Int(x),
            (Scalar::Decimal(d), _) => Val::Decimal(d),
            (Scalar::F32(f), _) => Val::F32(F32::new(f)),
            (Scalar::F64(f), _) => Val::F64(F64::new(f)),
            (Scalar::Text(t), _) => Val::text(t),
            (Scalar::Bytes(b), Base::SortKey) => Val::SortKey(Arc::from(b)),
            (Scalar::Bytes(b), _) => Val::Bytes(Arc::from(b)),
            (Scalar::Instant(t), _) => Val::Instant(t),
            (Scalar::Date(d), _) => Val::Date(
                i32::try_from(d).expect("a date within the range of `Instant` fits an `I32`"),
            ),
            (Scalar::Time(t), _) => Val::Time(t),
            (Scalar::DateTime(t), _) => Val::DateTime(t),
            (Scalar::Zoned { local, zone }, _) => Val::Zoned {
                local,
                zone: Arc::from(zone),
            },
            (Scalar::Duration(d), _) => Val::Duration(d),
            (Scalar::Period { months, days }, _) => Val::Period { months, days },
        }
    }

    fn of_ctor(ctor: u32, fields: Vec<(&str, Val)>) -> Val {
        Val::ctor(ctor, fields.into_iter().map(|(_, v)| v).collect())
    }

    fn of_record(fields: Vec<(&str, Val)>) -> Val {
        Val::record(fields.into_iter().map(|(_, v)| v).collect())
    }

    fn of_key(value: Val) -> Val {
        value
    }

    fn of_set(elems: Vec<Val>) -> Val {
        Val::set(elems.into_iter().collect())
    }

    fn zone_exists(name: &str) -> bool {
        zone(name).is_ok()
    }
}
