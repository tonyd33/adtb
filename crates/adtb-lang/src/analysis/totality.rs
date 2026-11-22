use std::collections::HashMap;

use crate::ast::FnBody;
use crate::kernel::{Agg, Const, Kind, Prim, Terms, Tm};
use crate::schema::Schema;
use crate::types::*;

use super::fix;

/// How evaluating a term may fail to produce a value, ordered by what `try` leaves of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Abort {
    Never,
    /// It may fault, and `try` catches the fault.
    Faults,
    /// It may fail, be rejected, not terminate or reach an `Unreachable`, which `try` does not catch.
    Escapes,
}

impl Abort {
    fn faults_when(partial: bool) -> Abort {
        if partial { Abort::Faults } else { Abort::Never }
    }
}

/// The classifier of kernel terms as total or partial, remembering each term's class.
#[derive(Debug)]
pub struct Totality<'s> {
    schema: &'s Schema,
    hiding: bool,
    memo: HashMap<Tm, Abort>,
    draws: HashMap<Tm, bool>,
    /// The class of each user-defined aggregate, by position among the schema's aggregates.
    aggregates: Vec<Abort>,
}

impl<'s> Totality<'s> {
    /// Returns a classifier for terms of `schema`; set `hiding` for a bundle declaring `hide new constructors`.
    pub fn new(schema: &'s Schema, hiding: bool) -> Self {
        Totality {
            schema,
            hiding,
            memo: HashMap::new(),
            draws: HashMap::new(),
            aggregates: Vec::new(),
        }
    }

    /// Judge each user-defined aggregate from its parts in `terms`: its init, step, merge and
    /// finish bodies, `None` for one with no parts. A user-defined aggregate judged no other way
    /// stays partial under `try`.
    pub fn with_aggregates(
        mut self,
        terms: &Terms,
        parts: impl IntoIterator<Item = Option<[Tm; 4]>>,
    ) -> Self {
        let mut judge = Totality::new(self.schema, self.hiding);
        self.aggregates = parts
            .into_iter()
            .map(|p| match p {
                Some(bodies) => judge.within(terms, bodies),
                None => Abort::Escapes,
            })
            .collect();
        self
    }

    pub fn schema(&self) -> &'s Schema {
        self.schema
    }

    /// Returns whether `t` cannot abort: it terminates with a value in every environment its type admits.
    ///
    /// Preconditions:
    /// - every term this classifier has seen belongs to `terms`, and no term of `terms` changes once added.
    pub fn total(&mut self, terms: &Terms, t: Tm) -> bool {
        self.abort(terms, t) == Abort::Never
    }

    /// Returns whether `t` may abort in a way `try` does not catch: fail, be rejected, not
    /// terminate or reach an `Unreachable`.
    ///
    /// Preconditions:
    /// - as [`Totality::total`].
    pub fn escapes(&mut self, terms: &Terms, t: Tm) -> bool {
        self.abort(terms, t) == Abort::Escapes
    }

    fn abort(&mut self, terms: &Terms, t: Tm) -> Abort {
        if let Some(&r) = self.memo.get(&t) {
            return r;
        }
        let kind = terms.kind(t);
        let r = match kind {
            Kind::Try(x) => match self.abort(terms, *x) {
                Abort::Never | Abort::Faults => Abort::Never,
                Abort::Escapes => Abort::Escapes,
            },
            Kind::Lam(..) => Abort::Never,
            Kind::Case { .. } | Kind::If(..) => {
                let reached = kind
                    .children()
                    .into_iter()
                    .filter(|c| !matches!(terms.kind(*c), Kind::Unreachable));
                self.within(terms, reached)
            }
            _ => self
                .here(terms, t, kind)
                .max(self.within(terms, kind.children())),
        };
        self.memo.insert(t, r);
        r
    }

    fn within(&mut self, terms: &Terms, ts: impl IntoIterator<Item = Tm>) -> Abort {
        ts.into_iter()
            .map(|c| self.abort(terms, c))
            .max()
            .unwrap_or(Abort::Never)
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
    #[deny(
        clippy::wildcard_enum_match_arm,
        clippy::match_wildcard_for_single_variants
    )]
    pub fn draws(&mut self, terms: &Terms, t: Tm) -> bool {
        if let Some(&d) = self.draws.get(&t) {
            return d;
        }
        let here = match terms.kind(t) {
            Kind::Fresh(..) => true,
            Kind::Call { func, .. } => matches!(
                self.schema.function(*func).body,
                FnBody::Host {
                    deterministic: false,
                    ..
                }
            ),
            Kind::Var(_)
            | Kind::Const(_)
            | Kind::Prim(..)
            | Kind::Record(_)
            | Kind::Field(..)
            | Kind::Ctor { .. }
            | Kind::Case { .. }
            | Kind::If(..)
            | Kind::Let(..)
            | Kind::Lam(..)
            | Kind::App(..)
            | Kind::Caller
            | Kind::Key(_)
            | Kind::KeyOf(..)
            | Kind::Deref { .. }
            | Kind::Tag(_)
            | Kind::Try(_)
            | Kind::Rel(..)
            | Kind::Transition(..)
            | Kind::Restrict(..)
            | Kind::EmptySet
            | Kind::Singleton(_)
            | Kind::Union(..)
            | Kind::For(..)
            | Kind::IsEmpty(_)
            | Kind::Exists(_)
            | Kind::Agg { .. }
            | Kind::Group { .. }
            | Kind::Top { .. }
            | Kind::Fix(..)
            | Kind::Placeholder { .. }
            | Kind::Only(_)
            | Kind::At { .. }
            | Kind::Present { .. }
            | Kind::Reject(_)
            | Kind::Unreachable
            | Kind::Skip
            | Kind::Insert(..)
            | Kind::Delete(..)
            | Kind::Replace(..)
            | Kind::Upsert { .. }
            | Kind::Rejects { .. }
            | Kind::Par(_)
            | Kind::Fail(_)
            | Kind::LetBang(..)
            | Kind::Block(..)
            | Kind::Transact(..) => false,
        };
        let d = here || terms.children(t).into_iter().any(|c| self.draws(terms, c));
        self.draws.insert(t, d);
        d
    }

    /// Returns how the form at the root of `t` may abort whatever its operands do.
    #[deny(
        clippy::wildcard_enum_match_arm,
        clippy::match_wildcard_for_single_variants
    )]
    fn here(&self, terms: &Terms, t: Tm, k: &Kind) -> Abort {
        match k {
            Kind::Prim(p, xs) => {
                let args: Vec<&Ty> = xs.iter().map(|x| terms.ty(*x)).collect();
                self.prim(*p, &args, terms.ty(t))
            }
            Kind::Agg { op, .. } => match op {
                Agg::Count | Agg::Min | Agg::Max | Agg::Avg => Abort::Never,
                Agg::Sum => Abort::faults_when(!is_float(terms.ty(t))),
                Agg::User(a) => self
                    .aggregates
                    .get(*a as usize)
                    .copied()
                    .unwrap_or(Abort::Escapes),
            },
            Kind::Top { count, .. } => Abort::faults_when(!literal_count(terms, *count)),
            Kind::Present { take, .. } => {
                Abort::faults_when(take.is_some_and(|n| !literal_count(terms, n)))
            }
            Kind::Fix(..) => {
                if fix::bounded(terms, t) {
                    Abort::Never
                } else {
                    Abort::Escapes
                }
            }
            Kind::Deref {
                key,
                reference,
                admits,
            } => {
                let narrowed = admits.is_some()
                    && !matches!(terms.ty(*key), Ty::Key { ctors, .. } if match ctors {
                        KeyCtors::All | KeyCtors::Late(_) => admits.is_none(),
                        KeyCtors::Only(cs) => admits.as_ref() == Some(cs),
                    });
                Abort::faults_when(self.hiding || !reference || narrowed)
            }
            Kind::Call { func, .. } => match self.schema.function(*func).body {
                FnBody::Wasm { .. } => Abort::Faults,
                FnBody::Host { .. } | FnBody::Expr(_) => Abort::Escapes,
            },
            Kind::App(..)
            | Kind::At { .. }
            | Kind::Fail(_)
            | Kind::Reject(_)
            | Kind::Replace(..)
            | Kind::Unreachable => Abort::Escapes,
            Kind::Var(_)
            | Kind::Const(_)
            | Kind::Record(_)
            | Kind::Field(..)
            | Kind::Ctor { .. }
            | Kind::Case { .. }
            | Kind::If(..)
            | Kind::Let(..)
            | Kind::Lam(..)
            | Kind::Caller
            | Kind::Key(_)
            | Kind::KeyOf(..)
            | Kind::Tag(_)
            | Kind::Try(_)
            | Kind::Rel(..)
            | Kind::Transition(..)
            | Kind::Restrict(..)
            | Kind::EmptySet
            | Kind::Singleton(_)
            | Kind::Union(..)
            | Kind::For(..)
            | Kind::IsEmpty(_)
            | Kind::Exists(_)
            | Kind::Group { .. }
            | Kind::Fresh(..)
            | Kind::Placeholder { .. }
            | Kind::Only(_)
            | Kind::Skip
            | Kind::Insert(..)
            | Kind::Delete(..)
            | Kind::Upsert { .. }
            | Kind::Rejects { .. }
            | Kind::Par(_)
            | Kind::LetBang(..)
            | Kind::Block(..)
            | Kind::Transact(..) => Abort::Never,
        }
    }

    #[deny(
        clippy::wildcard_enum_match_arm,
        clippy::match_wildcard_for_single_variants
    )]
    fn prim(&self, p: Prim, args: &[&Ty], result: &Ty) -> Abort {
        use Prim::*;
        match p {
            Add | Sub | Mul | Div => Abort::faults_when(!is_float(result)),
            Rem | DecimalDiv | Rescale | ParseUuid | Zoned | InZone | ToInstant | Span
            | AddPeriod | Micros | Millis | Seconds | Minutes | Hours | Days | Weeks | Months
            | Years => Abort::Faults,
            ReadFile { .. } => Abort::Escapes,
            Neg | Abs => Abort::faults_when(!matches!(
                result,
                Ty::Base(Base::Decimal(_) | Base::F32 | Base::F64)
            )),
            Convert { checked: false } => {
                Abort::faults_when(!args.first().is_some_and(|a| widens(a, result)))
            }
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
            | Start
            | End
            | Overlaps
            | Covers
            | Parse
            | Print => Abort::Never,
        }
    }
}

/// Returns whether `count` is a nonnegative integer literal.
pub fn literal_count(terms: &Terms, count: Tm) -> bool {
    matches!(terms.kind(count), Kind::Const(Const::Int(n)) if *n >= 0)
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
