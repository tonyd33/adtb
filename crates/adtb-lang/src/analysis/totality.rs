use std::collections::HashMap;

use crate::kernel::{Agg, Const, Kind, Prim, Terms, Tm};
use crate::schema::Schema;
use crate::types::*;

use super::fix;

/// Classifies kernel terms as total or partial, remembering each term's class.
#[derive(Debug)]
pub struct Totality<'s> {
    schema: &'s Schema,
    hiding: bool,
    memo: HashMap<Tm, bool>,
    draws: HashMap<Tm, bool>,
}

impl<'s> Totality<'s> {
    /// Returns a classifier for terms of `schema`; set `hiding` for a bundle declaring `hide new constructors`.
    pub fn new(schema: &'s Schema, hiding: bool) -> Self {
        Totality {
            schema,
            hiding,
            memo: HashMap::new(),
            draws: HashMap::new(),
        }
    }

    pub fn schema(&self) -> &'s Schema {
        self.schema
    }

    /// Returns whether `t` cannot abort: it terminates with a value in every environment its type admits.
    ///
    /// Preconditions:
    /// - every term this classifier has seen belongs to `terms`, and no term of `terms` changes once added.
    pub fn total(&mut self, terms: &Terms, t: Tm) -> bool {
        if let Some(&r) = self.memo.get(&t) {
            return r;
        }
        let r = match terms.kind(t) {
            Kind::Try(x) => !rejects(terms, *x),
            Kind::Lam(..) => true,
            k if self.partial_here(terms, t, k) => false,
            _ => terms.children(t).into_iter().all(|c| self.total(terms, c)),
        };
        self.memo.insert(t, r);
        r
    }

    /// Returns whether `t` may be duplicated or dropped: it is total and neither draws a fresh
    /// value nor calls a volatile host function.
    ///
    /// Preconditions:
    /// - as [`Totality::total`].
    pub fn copyable(&mut self, terms: &Terms, t: Tm) -> bool {
        self.total(terms, t) && !self.draws(terms, t)
    }

    /// Returns whether evaluating `t` draws a fresh value or calls a volatile host function.
    ///
    /// Preconditions:
    /// - as [`Totality::total`].
    pub fn draws(&mut self, terms: &Terms, t: Tm) -> bool {
        if let Some(&d) = self.draws.get(&t) {
            return d;
        }
        let d = match terms.kind(t) {
            Kind::Fresh(..) => true,
            Kind::Call { func, .. }
                if matches!(
                    self.schema.function(*func).body,
                    crate::ast::FnBody::Host {
                        deterministic: false,
                        ..
                    }
                ) =>
            {
                true
            }
            _ => terms.children(t).into_iter().any(|c| self.draws(terms, c)),
        };
        self.draws.insert(t, d);
        d
    }

    /// Returns whether the form at the root of `t` may abort whatever its operands do.
    fn partial_here(&self, terms: &Terms, t: Tm, k: &Kind) -> bool {
        match k {
            Kind::Prim(p, xs) => {
                let args: Vec<&Ty> = xs.iter().map(|x| terms.ty(*x)).collect();
                self.prim_partial(*p, &args, terms.ty(t))
            }
            Kind::Agg { op, .. } => match op {
                Agg::Count | Agg::Min | Agg::Max | Agg::Avg => false,
                Agg::Sum => !is_float(terms.ty(t)),
                Agg::User(_) => true,
            },
            Kind::Top { count, .. } => {
                !matches!(terms.kind(*count), Kind::Const(Const::Int(n)) if *n >= 0)
            }
            Kind::Fix(..) => !fix::bounded(terms, t),
            Kind::Deref { reference, .. } => self.hiding || !reference,
            Kind::Call { .. }
            | Kind::App(..)
            | Kind::At { .. }
            | Kind::Fail(_)
            | Kind::Reject(_)
            | Kind::Replace(..) => true,
            _ => false,
        }
    }

    fn prim_partial(&self, p: Prim, args: &[&Ty], result: &Ty) -> bool {
        use Prim::*;
        match p {
            Add | Sub | Mul | Div => !is_float(result),
            Rem
            | DecimalDiv
            | Rescale
            | ParseUuid
            | Zoned
            | InZone
            | Span
            | AddPeriod
            | ReadFile { .. }
            | Micros
            | Millis
            | Seconds
            | Minutes
            | Hours
            | Days
            | Weeks
            | Months
            | Years => true,
            Neg => !matches!(result, Ty::Base(Base::Decimal(_) | Base::F32 | Base::F64)),
            Abs => !matches!(result, Ty::Base(Base::Decimal(_) | Base::F32 | Base::F64)),
            Convert { checked: false } => !args.first().is_some_and(|a| widens(a, result)),
            Convert { checked: true }
            | Concat
            | Eq
            | Ne
            | Lt
            | Le
            | Gt
            | Ge
            | CheckedAdd
            | CheckedSub
            | CheckedMul
            | CheckedDiv
            | CheckedRem
            | CheckedNeg
            | Round
            | Wrap
            | Unwrap
            | KeyValue
            | ToText
            | BytesLen
            | ScalarLen
            | GraphemeLen
            | StartsWith
            | EndsWith
            | Contains
            | Matches
            | Lower
            | Upper
            | Fold
            | Collate
            | Utf8
            | Bytes
            | ToInstant
            | Start
            | End
            | Overlaps
            | Covers
            | Parse
            | Print => false,
        }
    }
}

fn rejects(terms: &Terms, t: Tm) -> bool {
    matches!(terms.kind(t), Kind::Reject(_))
        || terms.children(t).into_iter().any(|c| rejects(terms, c))
}

fn is_float(t: &Ty) -> bool {
    matches!(t, Ty::Base(Base::F32 | Base::F64))
}

fn int_digits(b: Base) -> Option<u32> {
    match b {
        Base::I8 => Some(3),
        Base::I16 => Some(5),
        Base::I32 => Some(10),
        Base::I64 => Some(19),
        _ => None,
    }
}

/// Returns whether every value of `from` converts to `to` without leaving its range.
fn widens(from: &Ty, to: &Ty) -> bool {
    let (Ty::Base(f), Ty::Base(t)) = (from, to) else {
        return false;
    };
    match (int_digits(*f), *t) {
        (Some(d), Base::Decimal(s)) => d + u32::from(s) <= 38,
        (Some(_), Base::F32 | Base::F64) => true,
        (Some(_), t) => match (f, t) {
            (Base::I8, _) => int_digits(t).is_some(),
            (Base::I16, Base::I16 | Base::I32 | Base::I64)
            | (Base::I32, Base::I32 | Base::I64)
            | (Base::I64, Base::I64) => true,
            _ => false,
        },
        (None, Base::F64) => matches!(f, Base::Decimal(_) | Base::F32 | Base::F64),
        (None, Base::F32) => matches!(f, Base::Decimal(_) | Base::F32),
        _ => false,
    }
}
