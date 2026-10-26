use std::collections::{HashMap, HashSet};

use crate::kernel::{Kind, Prim, Terms, Tm, Var};

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
        match (terms.kind(t), pos) {
            (Kind::Var(v), Pos::Not(why)) if self.xs.contains(v) => Err(format!(
                "`{}` occurs in a body of its own `fix` at a position that is not positive ({why}); negation and aggregation reach only relations of a lower stratum",
                terms.var_info(*v).name
            )),
            (Kind::Var(_), _) => Ok(()),
            (Kind::Union(a, b) | Kind::For(_, a, b), Pos::Positive) => {
                self.walk(*a, pos)?;
                self.walk(*b, pos)
            }
            (Kind::If(c, a, b), Pos::Positive) => {
                let cond = if matches!(terms.kind(*b), Kind::EmptySet) {
                    Pos::Condition
                } else {
                    Pos::Not("a condition whose `else` is not empty")
                };
                self.walk(*c, cond)?;
                self.walk(*a, pos)?;
                self.walk(*b, pos)
            }
            (Kind::Case { scrutinee, arms }, Pos::Positive) => {
                self.walk(*scrutinee, Pos::Not("a scrutinee"))?;
                for (_, _, e) in arms {
                    self.walk(*e, pos)?;
                }
                Ok(())
            }
            (Kind::Let(_, a, b), Pos::Positive) => {
                self.walk(*a, Pos::Not("the bound expression of a `let`"))?;
                self.walk(*b, pos)
            }
            (Kind::Fix(binds, body), Pos::Positive) => {
                for (_, e) in binds {
                    self.walk(*e, pos)?;
                }
                self.walk(*body, pos)
            }
            (Kind::Restrict(x, _), Pos::Positive) => self.walk(*x, pos),
            (Kind::Exists(x), Pos::Condition) => self.walk(*x, Pos::Positive),
            (Kind::If(p, q, r), Pos::Condition)
                if self.is_bool(*r, false) || self.is_bool(*q, true) =>
            {
                self.walk(*p, pos)?;
                self.walk(*q, pos)?;
                self.walk(*r, pos)
            }
            (k, pos) => {
                let why = match (reason(terms, k), pos) {
                    (Some(why), _) | (None, Pos::Not(why)) => why,
                    (None, _) => match k {
                        Kind::Exists(_) => "an `exists` that is not a positive condition",
                        Kind::Case { .. } | Kind::If(..) => "a condition that is not positive",
                        _ => "a position that is not positive",
                    },
                };
                for c in terms.children(t) {
                    self.walk(c, Pos::Not(why))?;
                }
                Ok(())
            }
        }
    }

    /// Returns whether `t`, a term of type `Bool`, is the literal `value`.
    fn is_bool(&self, t: Tm, value: bool) -> bool {
        matches!(self.terms.kind(t), Kind::Ctor { ctor, fields } if fields.is_empty() && *ctor == u32::from(value))
    }
}

/// Returns what makes the operands of `k` not positive, for the forms that are never positive.
fn reason(terms: &Terms, k: &Kind) -> Option<&'static str> {
    Some(match k {
        Kind::IsEmpty(_) => "the argument of `empty`",
        Kind::If(_, a, b)
            if matches!(terms.kind(*a), Kind::Ctor { ctor: 0, .. })
                && matches!(terms.kind(*b), Kind::Ctor { ctor: 1, .. }) =>
        {
            "the operand of `not`"
        }
        Kind::Agg { .. } => "the range or body of an aggregate",
        Kind::Group { .. } => "the source or key of `group`",
        Kind::Top { .. } => "the source or key of `top`",
        Kind::Record(_) | Kind::Ctor { .. } => "a field of a record or constructor value",
        Kind::Prim(..) => "an argument of a primitive",
        Kind::Call { .. } | Kind::App(..) | Kind::Lam(..) => "a function body or argument",
        _ => return None,
    })
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
            _ => {
                let mut r = Taint::None;
                for c in terms.children(t) {
                    r = r.max(self.taint(c));
                }
                derived(r)
            }
        }
    }

    /// Visits `t`, whose value or elements can become elements of a relation of the stratum, recording any construction over a tainted value.
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
            Kind::Singleton(x) | Kind::Restrict(x, _) | Kind::Only(x) => self.flow(*x),
            Kind::Top { source, .. } => self.flow(*source),
            Kind::At { body, .. } => self.flow(*body),
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
                if constructing(*p) && xs.iter().any(|x| self.taint(*x) != Taint::None) {
                    self.ok = false;
                }
            }
            Kind::Agg { .. } | Kind::Group { .. } | Kind::Call { .. }
                if self.taint(t) != Taint::None =>
            {
                self.ok = false;
            }
            _ => {}
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
