use std::collections::BTreeSet;

use crate::kernel::{Kind, Terms, Tm, Var};
use crate::schema::Schema;

/// Returns whether `t` reads a volatile relation.
pub fn reads_volatile(schema: &Schema, terms: &Terms, t: Tm) -> bool {
    terms.any(t, |k| {
        matches!(k, Kind::Rel(r, _) | Kind::Transition(_, r, _) if schema.relation(*r).volatile)
    })
}

fn mentions(terms: &Terms, t: Tm, vars: &BTreeSet<Var>) -> bool {
    terms.any(t, |k| matches!(k, Kind::Var(v) if vars.contains(v)))
}

/// Returns whether a write in `t` depends on a volatile read: its target is a volatile relation, its
/// target, predicate or values read one, or a condition, a source it ranges over or a `let` whose
/// value reaches it does.
///
/// Preconditions:
/// - `t` is a normal form.
pub fn write_depends_on_volatile(schema: &Schema, terms: &Terms, t: Tm) -> bool {
    let mut tainted = BTreeSet::new();
    let mut stack = vec![t];
    while let Some(t) = stack.pop() {
        match visit(schema, terms, t, &mut tainted) {
            Visit::Depends => return true,
            Visit::Into(cs) => stack.extend(cs.into_iter().rev()),
        }
    }
    false
}

/// What [`visit`] found at one term.
enum Visit {
    Depends,
    /// The terms to visit next, in order.
    Into(Vec<Tm>),
}

/// Visits `t` before the terms under it, tainting the variables a volatile read reaches.
fn visit(schema: &Schema, terms: &Terms, t: Tm, tainted: &mut BTreeSet<Var>) -> Visit {
    let dirty = |x: Tm, tainted: &BTreeSet<Var>| {
        reads_volatile(schema, terms, x) || mentions(terms, x, tainted)
    };
    let kind = terms.kind(t);
    if kind.is_write() {
        let target = match kind {
            Kind::Insert(r, _) | Kind::Delete(r, _) | Kind::Replace(r, ..) => Some(*r),
            Kind::Upsert { rel, .. } => Some(*rel),
            Kind::Rejects { into, .. } => Some(*into),
            _ => None,
        };
        let volatile = target.is_some_and(|r| schema.relation(r).volatile);
        return if volatile || dirty(t, tainted) {
            Visit::Depends
        } else {
            Visit::Into(Vec::new())
        };
    }
    match kind {
        Kind::Let(x, e, body) | Kind::For(x, e, body) => {
            if dirty(*e, tainted) {
                if terms.writes(*body) && matches!(kind, Kind::For(..)) {
                    return Visit::Depends;
                }
                tainted.insert(*x);
            }
            Visit::Into(vec![*e, *body])
        }
        Kind::If(c, a, b) => {
            if dirty(*c, tainted) && (terms.writes(*a) || terms.writes(*b)) {
                return Visit::Depends;
            }
            Visit::Into(vec![*a, *b])
        }
        Kind::Case { scrutinee, arms } => {
            if dirty(*scrutinee, tainted) {
                if arms.iter().any(|(_, _, b)| terms.writes(*b)) {
                    return Visit::Depends;
                }
                tainted.extend(arms.iter().map(|(_, v, _)| *v));
            }
            Visit::Into(arms.iter().map(|(_, _, b)| *b).collect())
        }
        _ => Visit::Into(terms.children(t)),
    }
}
