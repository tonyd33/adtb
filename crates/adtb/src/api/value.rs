use std::cmp::Ordering;
use std::collections::HashSet;
use std::sync::Arc;

use adtb_lang::calendar::{MICROS_PER_DAY, date_in_range};
use adtb_lang::schema::{Schema, TypeDefKind};
use adtb_lang::textual::Textual;
use adtb_lang::types::{Base, Ty};
use unicode_normalization::is_nfc;

use crate::catalog::layout;
use crate::exec::types::{ctor_set, ctors};
use crate::exec::{self, Val};

type Node = (*const Val, Ty);

enum Work<'v> {
    Check(Ty, &'v Val),
    Complete(Node),
    Span(&'v Val),
}

pub(super) fn valid(schema: &Schema, ty: &Ty, value: &Val) -> bool {
    let mut work = vec![Work::Check(ty.clone(), value)];
    let mut pending = HashSet::new();
    let mut checked = HashSet::new();
    while let Some(next) = work.pop() {
        match next {
            Work::Complete(node) => {
                assert!(
                    pending.remove(&node),
                    "a completed value was pending validation"
                );
                checked.insert(node);
            }
            Work::Span(Val::Ctor(_, fields)) => {
                let [start, end] = fields.as_ref() else {
                    return false;
                };
                if ordered(start, end).is_none_or(|order| order == Ordering::Greater) {
                    return false;
                }
            }
            Work::Span(_) => return false,
            Work::Check(ty, value) => {
                let node = (std::ptr::from_ref(value), ty.clone());
                if checked.contains(&node) {
                    continue;
                }
                if !pending.insert(node.clone()) {
                    return false;
                }
                work.push(Work::Complete(node));
                if !check(schema, &ty, value, &mut work) {
                    return false;
                }
            }
        }
    }
    true
}

fn ordered(left: &Val, right: &Val) -> Option<Ordering> {
    let mut work = vec![(left, right)];
    let mut checked = HashSet::new();
    while let Some((left, right)) = work.pop() {
        if std::ptr::eq(left, right)
            || !checked.insert((std::ptr::from_ref(left), std::ptr::from_ref(right)))
        {
            continue;
        }
        let fields = match (left, right) {
            (Val::Ctor(a, fields), Val::Ctor(b, other)) => {
                let order = a.cmp(b);
                if order != Ordering::Equal {
                    return Some(order);
                }
                Some((fields, other))
            }
            (Val::Record(fields), Val::Record(other)) => Some((fields, other)),
            (
                Val::Ctor(..)
                | Val::Record(..)
                | Val::Set(..)
                | Val::List(..)
                | Val::Zoned { .. }
                | Val::Period { .. },
                _,
            )
            | (
                _,
                Val::Ctor(..)
                | Val::Record(..)
                | Val::Set(..)
                | Val::List(..)
                | Val::Zoned { .. }
                | Val::Period { .. },
            ) => return None,
            _ => {
                let order = left.cmp(right);
                if order != Ordering::Equal {
                    return Some(order);
                }
                None
            }
        };
        if let Some((fields, other)) = fields {
            if fields.len() != other.len() {
                return None;
            }
            if !Arc::ptr_eq(fields, other) {
                work.extend(fields.iter().zip(other.iter()).rev());
            }
        }
    }
    Some(Ordering::Equal)
}

fn check<'v>(schema: &Schema, ty: &Ty, value: &'v Val, work: &mut Vec<Work<'v>>) -> bool {
    match ty {
        Ty::Base(base) => scalar(*base, value),
        Ty::Data { def, args, .. } => {
            let declared = schema.ty(*def);
            if args.len() != declared.params.len() {
                return false;
            }
            match &declared.kind {
                TypeDefKind::Newtype { base, .. } => {
                    work.push(Work::Check(base.subst(args), value));
                    true
                }
                TypeDefKind::Sum { .. } => sum(schema, ty, value, work),
            }
        }
        Ty::Sum { .. } | Ty::Tag(_) => sum(schema, ty, value, work),
        Ty::Key { rel, .. } => {
            work.push(Work::Check(layout::key_value_ty(schema, *rel), value));
            true
        }
        Ty::Record(fields) => {
            let Val::Record(values) = value else {
                return false;
            };
            fields_match(fields, values, work)
        }
        Ty::Unit => matches!(value, Val::Record(fields) if fields.is_empty()),
        Ty::Set(elem) | Ty::NonEmpty(elem) => {
            let Val::Set(values) = value else {
                return false;
            };
            if matches!(ty, Ty::NonEmpty(_)) && values.is_empty() {
                return false;
            }
            work.extend(values.iter().map(|v| Work::Check(*elem.clone(), v)));
            true
        }
        Ty::Param(_) | Ty::Group(..) | Ty::Fn(..) | Ty::Change => false,
    }
}

fn sum<'v>(schema: &Schema, ty: &Ty, value: &'v Val, work: &mut Vec<Work<'v>>) -> bool {
    let Val::Ctor(ctor, values) = value else {
        return false;
    };
    if !ctor_set(schema, ty).contains(ctor) {
        return false;
    }
    let Some(constructors) = ctors(schema, ty) else {
        return false;
    };
    let Some(fields) = constructors.get(*ctor as usize) else {
        return false;
    };
    if matches!(ty, Ty::Data { def, .. } if *def == schema.prelude_type("Span")) {
        work.push(Work::Span(value));
    }
    fields_match(fields, values, work)
}

fn fields_match<'v>(fields: &[(String, Ty)], values: &'v [Val], work: &mut Vec<Work<'v>>) -> bool {
    if fields.len() != values.len() {
        return false;
    }
    work.extend(
        fields
            .iter()
            .zip(values)
            .map(|((_, ty), v)| Work::Check(ty.clone(), v)),
    );
    true
}

fn scalar(base: Base, value: &Val) -> bool {
    match (base, value) {
        (Base::I8 | Base::I16 | Base::I32 | Base::I64, Val::Int(n)) => base
            .int_range()
            .is_some_and(|(lo, hi)| (lo..=hi).contains(n)),
        (Base::Decimal(scale), Val::Decimal(n)) => scale <= 38 && exec::decimal_fits(*n),
        (Base::Text, Val::Text(text)) => is_nfc(text),
        (Base::Date, Val::Date(day)) => date_in_range(i64::from(*day)),
        (Base::Time, Val::Time(time)) => (0..MICROS_PER_DAY).contains(time),
        (Base::ZonedDateTime, Val::Zoned { zone, .. }) => Val::zone_exists(zone),
        (Base::F32, Val::F32(_))
        | (Base::F64, Val::F64(_))
        | (Base::Bytes, Val::Bytes(_))
        | (Base::SortKey, Val::SortKey(_))
        | (Base::Instant, Val::Instant(_))
        | (Base::DateTime, Val::DateTime(_))
        | (Base::Duration, Val::Duration(_))
        | (Base::Period, Val::Period { .. }) => true,
        _ => false,
    }
}

#[cfg(test)]
#[path = "value_tests.rs"]
mod tests;
