//! Admit kernel terms: stratification, boundedness, totality, aggregates and volatile reads.

use std::collections::HashSet;

use crate::check::{Adhoc, Program, Query, Txn};
use crate::kernel::{Kind, Terms, Tm};
use crate::norm::{self, Root};
use crate::schema::{DefaultValue, Schema};
use crate::span::{Diag, FileDiag, FileId, Span};

mod aggregate;
mod fix;
mod system;
mod totality;
mod volatile;

#[cfg(test)]
pub(crate) mod tests;
#[cfg(test)]
mod totality_tests;

pub use aggregate::ranges_over_bindings;
pub use fix::{bounded, stratified};
pub use system::writable_relation;
pub use totality::Totality;
pub use volatile::{reads_volatile, write_depends_on_volatile};

/// Apply every acceptance rule stated over the normal form to the terms reachable from `roots`.
///
/// Preconditions:
/// - `roots` are normal forms in `terms`.
pub fn accept(terms: &Terms, roots: &[Tm]) -> Result<(), String> {
    let mut seen = HashSet::new();
    let mut stack: Vec<Tm> = roots.to_vec();
    while let Some(t) = stack.pop() {
        if !seen.insert(t) {
            continue;
        }
        match terms.kind(t) {
            Kind::Fix(..) => stratified(terms, t)?,
            Kind::Agg { .. } => ranges_over_bindings(terms, t)?,
            _ => {}
        }
        stack.extend(terms.children(t));
    }
    Ok(())
}

fn root_site(schema: &Schema, root: Root) -> (FileId, Span) {
    match root {
        Root::Function(f) => {
            let d = schema.function(f);
            (d.file, d.span)
        }
        Root::Constraint(i) => {
            let c = schema.constraint(i);
            (c.file, c.query.span)
        }
        Root::Check(i) => {
            let c = schema.check(i);
            (c.file, c.predicate.span)
        }
        Root::Domain(t) => {
            let d = schema.ty(t);
            (d.file, d.span)
        }
        Root::Retag(r) => {
            let d = schema.relation(r);
            (d.file, d.span)
        }
        Root::Default(r, i) => {
            let d = schema.relation(r);
            match &d.defaults[i].value {
                DefaultValue::Expr(e) => (d.file, e.span),
                _ => (d.file, d.span),
            }
        }
        Root::Computed(r, i) => {
            let d = schema.relation(r);
            (d.file, d.computed[i].expr.span)
        }
        Root::IndexField(r, i, j) => {
            let d = schema.relation(r);
            (d.file, d.indexes[i].fields[j].span)
        }
        Root::Aggregate(i) => {
            let d = &schema.aggregates[i];
            (d.file, d.decl.span)
        }
        Root::Item(_) | Root::Command(_) | Root::Query => {
            unreachable!("a schema has no ad hoc items or queries")
        }
    }
}

/// Normalise a checked schema's bodies and apply the acceptance rules over their normal forms.
pub(crate) fn schema_diags(schema: &Schema, p: &Program) -> Vec<FileDiag> {
    let site = |root: Root, message: String| {
        let (file, span) = root_site(schema, root);
        FileDiag {
            file,
            diag: Diag::new(span, message),
        }
    };
    let n = match norm::program(schema, p) {
        Ok(n) => n,
        Err(e) => {
            let (file, span) = root_site(schema, e.root);
            return vec![FileDiag {
                file,
                diag: diag(span, e),
            }];
        }
    };
    n.bodies()
        .into_iter()
        .filter_map(|(root, t, _)| {
            accept(n.terms(), &[t])
                .and_then(|()| volatile_free(schema, n.terms(), root, t))
                .err()
                .map(|m| site(root, m))
        })
        .collect()
}

/// Refuse a schema object that must not read a volatile relation but does.
fn volatile_free(schema: &Schema, terms: &Terms, root: Root, t: Tm) -> Result<(), String> {
    let what = match root {
        Root::Function(f) if schema.function(f).stored.is_some() => "a stored view",
        Root::Function(_)
        | Root::Aggregate(_)
        | Root::Domain(_)
        | Root::Item(_)
        | Root::Command(_)
        | Root::Query => {
            return Ok(());
        }
        Root::Constraint(_) => "a constraint",
        Root::Check(_) => "a row check",
        Root::Retag(_) => "a delete action",
        Root::Default(..) => "a default",
        Root::Computed(..) => "a computed field",
        Root::IndexField(..) => "an index expression",
    };
    if reads_volatile(schema, terms, t) {
        Err(format!("{what} must not read a volatile relation"))
    } else {
        Ok(())
    }
}

const VOLATILE_WRITE: &str = "a write must not depend on a volatile relation: not in its target, its predicate, its values, or a value or condition it depends on";

/// Normalise checked ad hoc text and apply the acceptance rules over the normal forms of its
/// items and its commands' terms, `spans` giving each item's span.
pub(crate) fn adhoc_diags(schema: &Schema, p: &Program, a: &Adhoc, spans: &[Span]) -> Vec<Diag> {
    let items = a.items.iter().zip(spans).map(|(t, span)| (*t, *span));
    let commands = a
        .commands
        .iter()
        .flat_map(|c| c.command.terms().into_iter().map(|t| (t, c.span)));
    let denied: Vec<_> = items
        .chain(commands)
        .filter_map(|(term, span)| system::program(schema, p, &a.terms, [term], span).err())
        .collect();
    if !denied.is_empty() {
        return denied;
    }
    let n = match norm::adhoc(schema, p, a) {
        Ok(n) => n,
        Err(e) => {
            let span = match e.root {
                Root::Item(i) => spans[i],
                Root::Command(i) => a.commands[i].span,
                _ => unreachable!("ad hoc errors are at items or commands"),
            };
            return vec![diag(span, e)];
        }
    };
    let accepted = |t: Tm| {
        accept(n.terms(), &[t]).and_then(|()| {
            if write_depends_on_volatile(schema, n.terms(), t) {
                Err(VOLATILE_WRITE.to_string())
            } else {
                Ok(())
            }
        })
    };
    let items = n.items().iter().zip(spans).map(|(t, span)| (*t, *span));
    let commands = n
        .commands()
        .iter()
        .flat_map(|c| c.command.terms().into_iter().map(|t| (t, c.span)));
    items
        .chain(commands)
        .filter_map(|(t, span)| accepted(t).err().map(|m| Diag::new(span, m)))
        .collect()
}

/// Returns the diagnostic at `span` of normalisation failing with `e`.
fn diag(span: Span, e: norm::Error) -> Diag {
    if e.broken().is_some() {
        Diag::internal(span, e.message)
    } else {
        Diag::new(span, e.message)
    }
}

/// Normalise a checked registered query and apply the acceptance rules over its normal form.
pub(crate) fn query_diag(schema: &Schema, p: &Program, q: &Query, span: Span) -> Result<(), Diag> {
    system::program(schema, p, &q.terms, [q.term], span)?;
    let n = norm::query(schema, p, q).map_err(|e| diag(span, e))?;
    accept(n.terms(), &[n.body()]).map_err(|m| Diag::new(span, m))
}

/// Normalise a checked registered transaction and apply the acceptance rules over its normal form.
pub(crate) fn txn_diag(schema: &Schema, p: &Program, t: &Txn, span: Span) -> Result<(), Diag> {
    system::program(schema, p, &t.terms, [t.term], span)?;
    let n = norm::txn(schema, p, t).map_err(|e| diag(span, e))?;
    accept(n.terms(), &[n.body()]).map_err(|m| Diag::new(span, m))?;
    if write_depends_on_volatile(schema, n.terms(), n.body()) {
        return Err(Diag::new(span, VOLATILE_WRITE));
    }
    Ok(())
}
