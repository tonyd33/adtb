use std::collections::BTreeSet;

use crate::kernel::{Kind, Terms, Tm, Var};
use crate::schema::Schema;

/// Returns whether `t` reads a volatile relation.
pub fn reads_volatile(schema: &Schema, terms: &Terms, t: Tm) -> bool {
    match terms.kind(t) {
        Kind::Rel(r, _) | Kind::Transition(_, r, _) if schema.relation(*r).volatile => true,
        _ => terms
            .children(t)
            .into_iter()
            .any(|c| reads_volatile(schema, terms, c)),
    }
}

fn is_write(k: &Kind) -> bool {
    matches!(
        k,
        Kind::Insert(..)
            | Kind::Delete(..)
            | Kind::Replace(..)
            | Kind::Upsert { .. }
            | Kind::Rejects { .. }
            | Kind::Fail(_)
    )
}

fn has_write(terms: &Terms, t: Tm) -> bool {
    is_write(terms.kind(t)) || terms.children(t).into_iter().any(|c| has_write(terms, c))
}

fn mentions(terms: &Terms, t: Tm, vars: &BTreeSet<Var>) -> bool {
    match terms.kind(t) {
        Kind::Var(v) => vars.contains(v),
        _ => terms
            .children(t)
            .into_iter()
            .any(|c| mentions(terms, c, vars)),
    }
}

/// Returns whether a write in `t` depends on a volatile read: its target, predicate or values read one, or a
/// condition, a source it ranges over or a `let` whose value reaches it does.
///
/// Preconditions:
/// - `t` is a normal form.
pub fn write_depends_on_volatile(schema: &Schema, terms: &Terms, t: Tm) -> bool {
    walk(schema, terms, t, &mut BTreeSet::new())
}

fn walk(schema: &Schema, terms: &Terms, t: Tm, tainted: &mut BTreeSet<Var>) -> bool {
    let dirty = |x: Tm, tainted: &BTreeSet<Var>| {
        reads_volatile(schema, terms, x) || mentions(terms, x, tainted)
    };
    let kind = terms.kind(t).clone();
    if is_write(&kind) {
        return dirty(t, tainted);
    }
    match kind {
        Kind::Let(x, e, body) | Kind::For(x, e, body) => {
            if dirty(e, tainted) {
                if has_write(terms, body) && matches!(terms.kind(t), Kind::For(..)) {
                    return true;
                }
                tainted.insert(x);
            }
            walk(schema, terms, e, tainted) || walk(schema, terms, body, tainted)
        }
        Kind::If(c, a, b) => {
            (dirty(c, tainted) && (has_write(terms, a) || has_write(terms, b)))
                || walk(schema, terms, a, tainted)
                || walk(schema, terms, b, tainted)
        }
        Kind::Case { scrutinee, arms } => {
            let by_volatile = dirty(scrutinee, tainted);
            if by_volatile && arms.iter().any(|(_, _, b)| has_write(terms, *b)) {
                return true;
            }
            arms.iter().any(|(_, v, b)| {
                if by_volatile {
                    tainted.insert(*v);
                }
                walk(schema, terms, *b, tainted)
            })
        }
        _ => terms
            .children(t)
            .into_iter()
            .any(|c| walk(schema, terms, c, tainted)),
    }
}
