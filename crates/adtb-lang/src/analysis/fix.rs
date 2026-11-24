use std::collections::{HashMap, HashSet};

use crate::kernel::{Kind, Prim, Terms, Tm, Var};
use crate::schema::prelude::ctor;

/// Where a subterm of a `fix` body sits, for stratification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Pos {
    Positive,
    Condition,
    /// Not positive, for the reason given.
    Not(&'static str),
}

/// Returns an error naming the first binder of the `fix` at `t` that occurs in one of its bodies at a position that is not positive.
pub fn stratified(terms: &Terms, t: Tm) -> Result<(), String> {
    let Kind::Fix(binds, _) = terms.kind(t) else {
        return Ok(());
    };
    let xs: Vec<Var> = binds.iter().map(|(x, _)| *x).collect();
    let mut s = Strata {
        terms,
        xs: &xs,
        seen: HashSet::new(),
    };
    for (_, e) in binds {
        s.walk(*e, Pos::Positive)?;
    }
    Ok(())
}

struct Strata<'t> {
    terms: &'t Terms,
    xs: &'t [Var],
    seen: HashSet<(Tm, Pos)>,
}

impl Strata<'_> {
    fn walk(&mut self, t: Tm, pos: Pos) -> Result<(), String> {
        if !self.seen.insert((t, pos)) {
            return Ok(());
        }
        let terms = self.terms;
        let kind = terms.kind(t);
        if let Kind::Var(v) = kind {
            return match pos {
                Pos::Not(why) if self.xs.contains(v) => Err(format!(
                    "`{}` occurs in a body of its own `fix` at a position that is not positive ({why}); negation and aggregation reach only relations of a lower stratum",
                    terms.var_info(*v).name
                )),
                Pos::Positive | Pos::Condition | Pos::Not(_) => Ok(()),
            };
        }
        if let Some(operands) = self.operands(kind, pos) {
            return operands.into_iter().try_for_each(|(c, p)| self.walk(c, p));
        }
        let why = match (reason(terms, kind), pos) {
            (Some(why), _) | (None, Pos::Not(why)) => why,
            (None, Pos::Positive | Pos::Condition) => {
                if matches!(kind, Kind::Exists(_)) {
                    "an `exists` that is not a positive condition"
                } else if matches!(kind, Kind::Case { .. } | Kind::If(..)) {
                    "a condition that is not positive"
                } else {
                    "a position that is not positive"
                }
            }
        };
        terms
            .children(t)
            .into_iter()
            .try_for_each(|c| self.walk(c, Pos::Not(why)))
    }

    /// Returns the position of each operand of `kind` at `pos`, or `None` when the form makes none
    /// of them positive or a positive condition there.
    #[deny(
        clippy::wildcard_enum_match_arm,
        clippy::match_wildcard_for_single_variants
    )]
    fn operands(&self, kind: &Kind, pos: Pos) -> Option<Vec<(Tm, Pos)>> {
        let positive = pos == Pos::Positive;
        match kind {
            Kind::Union(a, b) | Kind::For(_, a, b) => positive.then(|| vec![(*a, pos), (*b, pos)]),
            Kind::If(c, a, b) => match pos {
                Pos::Positive => {
                    let cond = if matches!(self.terms.kind(*b), Kind::EmptySet) {
                        Pos::Condition
                    } else {
                        Pos::Not("a condition whose `else` is not empty")
                    };
                    Some(vec![(*c, cond), (*a, pos), (*b, pos)])
                }
                Pos::Condition if self.is_bool(*b, false) || self.is_bool(*a, true) => {
                    Some(vec![(*c, pos), (*a, pos), (*b, pos)])
                }
                Pos::Condition | Pos::Not(_) => None,
            },
            Kind::Case { scrutinee, arms } => positive.then(|| {
                let mut ops = vec![(*scrutinee, Pos::Not("a scrutinee"))];
                ops.extend(arms.iter().map(|(_, _, e)| (*e, pos)));
                ops
            }),
            Kind::Let(_, a, b) => {
                positive.then(|| vec![(*a, Pos::Not("the bound expression of a `let`")), (*b, pos)])
            }
            Kind::Fix(binds, body) => positive.then(|| {
                let mut ops: Vec<(Tm, Pos)> = binds.iter().map(|(_, e)| (*e, pos)).collect();
                ops.push((*body, pos));
                ops
            }),
            Kind::Restrict(x, _) => positive.then(|| vec![(*x, pos)]),
            Kind::Exists(x) => (pos == Pos::Condition).then(|| vec![(*x, Pos::Positive)]),
            Kind::Var(_)
            | Kind::Const(_)
            | Kind::Prim(..)
            | Kind::Record(_)
            | Kind::Field(..)
            | Kind::Ctor { .. }
            | Kind::Lam(..)
            | Kind::App(..)
            | Kind::Call { .. }
            | Kind::Caller
            | Kind::Key(_)
            | Kind::KeyOf(..)
            | Kind::Deref { .. }
            | Kind::Tag(_)
            | Kind::Try(_)
            | Kind::Rel(..)
            | Kind::Transition(..)
            | Kind::EmptySet
            | Kind::Singleton(_)
            | Kind::IsEmpty(_)
            | Kind::Agg { .. }
            | Kind::Group { .. }
            | Kind::Top { .. }
            | Kind::Fresh(..)
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
            | Kind::Transact(..) => None,
        }
    }

    /// Returns whether `t`, a term of type `Bool`, is the literal `value`.
    fn is_bool(&self, t: Tm, value: bool) -> bool {
        matches!(self.terms.kind(t), Kind::Ctor { ctor: c, fields } if fields.is_empty() && *c == ctor::bool(value))
    }
}

fn ctor_of(terms: &Terms, t: Tm) -> Option<u32> {
    match terms.kind(t) {
        Kind::Ctor { ctor, .. } => Some(*ctor),
        _ => None,
    }
}

/// Returns what makes the operands of `k` not positive, for the forms that are never positive.
#[deny(
    clippy::wildcard_enum_match_arm,
    clippy::match_wildcard_for_single_variants
)]
fn reason(terms: &Terms, k: &Kind) -> Option<&'static str> {
    match k {
        Kind::IsEmpty(_) => Some("the argument of `empty`"),
        Kind::If(_, a, b) => (ctor_of(terms, *a) == Some(ctor::FALSE)
            && ctor_of(terms, *b) == Some(ctor::TRUE))
        .then_some("the operand of `not`"),
        Kind::Agg { .. } => Some("the range or body of an aggregate"),
        Kind::Group { .. } => Some("the source or key of `group`"),
        Kind::Top { .. } => Some("the source or key of `top`"),
        Kind::Record(_) | Kind::Ctor { .. } => Some("a field of a record or constructor value"),
        Kind::Prim(..) => Some("an argument of a primitive"),
        Kind::Call { .. } | Kind::App(..) | Kind::Lam(..) => Some("a function body or argument"),
        Kind::Var(_)
        | Kind::Const(_)
        | Kind::Field(..)
        | Kind::Case { .. }
        | Kind::Let(..)
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
        | Kind::Exists(_)
        | Kind::Fix(..)
        | Kind::Fresh(..)
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
        | Kind::Transact(..) => None,
    }
}

/// How a value of a `fix` body depends on the relations of the `fix`'s own stratum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Taint {
    None,
    /// Computed from an element of such a relation.
    Derived,
    /// An element of such a relation itself.
    Elem,
}

/// Returns whether the `fix` at `t` is bounded: every value it adds is built from finitely many values present before it runs.
///
/// A construction counts only where its result can become an element of a relation of the stratum; one used only in a condition, a comparison or an aggregate key does not.
pub fn bounded(terms: &Terms, t: Tm) -> bool {
    let Kind::Fix(binds, _) = terms.kind(t) else {
        return true;
    };
    let mut b = Bounds {
        terms,
        vars: binds.iter().map(|(x, _)| (*x, Taint::Elem)).collect(),
        memo: HashMap::new(),
        origins: HashMap::new(),
        flowed: HashSet::new(),
        ok: true,
    };
    for (_, e) in binds {
        b.flow(*e);
    }
    b.ok
}

struct Bounds<'t> {
    terms: &'t Terms,
    vars: HashMap<Var, Taint>,
    memo: HashMap<Tm, Taint>,
    /// The terms a variable's value comes from: a `let`'s bound value, a generator's source, a scrutinee, an inner `fix`'s bodies.
    origins: HashMap<Var, Vec<Tm>>,
    flowed: HashSet<Tm>,
    ok: bool,
}

impl Bounds<'_> {
    /// Returns the taint of the value of `t`, or of its elements when `t` is a set.
    fn taint(&mut self, t: Tm) -> Taint {
        if let Some(&r) = self.memo.get(&t) {
            return r;
        }
        let r = self.compute(t);
        self.memo.insert(t, r);
        r
    }

    #[deny(
        clippy::wildcard_enum_match_arm,
        clippy::match_wildcard_for_single_variants
    )]
    fn compute(&mut self, t: Tm) -> Taint {
        let terms = self.terms;
        match terms.kind(t) {
            Kind::Var(v) => self.vars.get(v).copied().unwrap_or(Taint::None),
            Kind::Singleton(x) | Kind::Restrict(x, _) | Kind::Only(x) => self.taint(*x),
            Kind::At { point, body, .. } => {
                self.taint(*point);
                self.taint(*body)
            }
            Kind::Union(a, b) => self.taint(*a).max(self.taint(*b)),
            Kind::For(y, s, b) | Kind::Let(y, s, b) => {
                let ts = self.taint(*s);
                self.vars.insert(*y, ts);
                self.taint(*b)
            }
            Kind::If(c, a, b) => {
                self.taint(*c);
                self.taint(*a).max(self.taint(*b))
            }
            Kind::Case { scrutinee, arms } => {
                let ts = self.taint(*scrutinee);
                let mut r = Taint::None;
                for (_, v, e) in arms {
                    self.vars.insert(*v, ts);
                    r = r.max(self.taint(*e));
                }
                r
            }
            Kind::Top {
                count,
                keys,
                var,
                source,
            } => {
                self.taint(*count);
                let ts = self.taint(*source);
                self.vars.insert(*var, ts);
                for (k, _) in keys {
                    self.taint(*k);
                }
                ts
            }
            Kind::Fix(binds, body) => {
                for (x, _) in binds {
                    self.vars.insert(*x, Taint::Elem);
                }
                for (_, e) in binds {
                    self.taint(*e);
                }
                self.taint(*body)
            }
            Kind::Agg {
                var, source, body, ..
            } => {
                let ts = self.taint(*source);
                self.vars.insert(*var, ts);
                derived(ts.max(self.taint(*body)))
            }
            Kind::Group { var, source, by } => {
                let ts = self.taint(*source);
                self.vars.insert(*var, ts);
                derived(ts.max(self.taint(*by)))
            }
            Kind::Const(_)
            | Kind::Prim(..)
            | Kind::Record(_)
            | Kind::Field(..)
            | Kind::Ctor { .. }
            | Kind::Lam(..)
            | Kind::App(..)
            | Kind::Call { .. }
            | Kind::Caller
            | Kind::Key(_)
            | Kind::KeyOf(..)
            | Kind::Deref { .. }
            | Kind::Tag(_)
            | Kind::Try(_)
            | Kind::Rel(..)
            | Kind::Transition(..)
            | Kind::EmptySet
            | Kind::IsEmpty(_)
            | Kind::Exists(_)
            | Kind::Fresh(..)
            | Kind::Placeholder { .. }
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
            | Kind::Transact(..) => {
                let mut r = Taint::None;
                for c in terms.children(t) {
                    r = r.max(self.taint(c));
                }
                derived(r)
            }
        }
    }

    /// Visit `t`, whose value or elements can become elements of a relation of the stratum, recording any construction over a tainted value.
    #[deny(
        clippy::wildcard_enum_match_arm,
        clippy::match_wildcard_for_single_variants
    )]
    fn flow(&mut self, t: Tm) {
        if !self.flowed.insert(t) {
            return;
        }
        self.taint(t);
        let terms = self.terms;
        match terms.kind(t) {
            Kind::Var(v) => {
                for o in self.origins.get(v).cloned().unwrap_or_default() {
                    self.flow(o);
                }
            }
            Kind::Union(a, b) | Kind::If(_, a, b) => {
                self.flow(*a);
                self.flow(*b);
            }
            Kind::For(y, s, b) | Kind::Let(y, s, b) => {
                self.origins.insert(*y, vec![*s]);
                self.flow(*b);
            }
            Kind::Case { scrutinee, arms } => {
                for (_, v, e) in arms {
                    self.origins.insert(*v, vec![*scrutinee]);
                    self.flow(*e);
                }
            }
            Kind::Singleton(x)
            | Kind::Restrict(x, _)
            | Kind::Only(x)
            | Kind::Field(x, _)
            | Kind::Key(x)
            | Kind::KeyOf(_, x)
            | Kind::Top { source: x, .. }
            | Kind::Present { source: x, .. }
            | Kind::At { body: x, .. } => self.flow(*x),
            Kind::Fix(binds, body) => {
                let bodies: Vec<Tm> = binds.iter().map(|(_, e)| *e).collect();
                for (x, _) in binds {
                    self.origins.insert(*x, bodies.clone());
                }
                for e in bodies {
                    self.flow(e);
                }
                self.flow(*body);
            }
            Kind::Record(fs) | Kind::Ctor { fields: fs, .. } => {
                for (_, f) in fs {
                    if self.taint(*f) == Taint::Elem {
                        self.ok = false;
                    }
                    self.flow(*f);
                }
            }
            Kind::Prim(p, xs) => {
                if constructing(p.clone()) && xs.iter().any(|x| self.taint(*x) != Taint::None) {
                    self.ok = false;
                }
            }
            Kind::Try(x) => {
                if self.taint(*x) == Taint::Elem {
                    self.ok = false;
                }
                self.flow(*x);
            }
            Kind::Agg { .. }
            | Kind::Group { .. }
            | Kind::Call { .. }
            | Kind::App(..)
            | Kind::Placeholder { .. } => {
                if self.taint(t) != Taint::None {
                    self.ok = false;
                }
            }
            Kind::Fresh(..) => self.ok = false,
            Kind::Const(_)
            | Kind::Lam(..)
            | Kind::Caller
            | Kind::Deref { .. }
            | Kind::Tag(_)
            | Kind::Rel(..)
            | Kind::Transition(..)
            | Kind::EmptySet
            | Kind::IsEmpty(_)
            | Kind::Exists(_)
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
            | Kind::Transact(..) => {}
        }
    }
}

fn derived(t: Taint) -> Taint {
    t.min(Taint::Derived)
}

/// Returns whether `p` may return a value that is none of its arguments and no part of one.
fn constructing(p: Prim) -> bool {
    use Prim::*;
    !matches!(
        p,
        Eq | Ne
            | Lt
            | Le
            | Gt
            | Ge
            | StartsWith
            | EndsWith
            | Contains
            | Matches
            | Overlaps
            | Covers
            | Start
            | End
            | Unwrap
            | KeyValue
    )
}
