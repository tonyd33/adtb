use std::collections::HashSet;

use crate::check::{Adhoc, Program, Query, Txn};
use crate::kernel::{Kind, Terms, Tm};
use crate::norm::{self, Root};
use crate::schema::{DefaultValue, Schema};
use crate::span::{Diag, FileDiag, FileId, Span};

mod aggregate;
mod fix;
mod totality;

#[cfg(test)]
mod tests;

pub use aggregate::ranges_over_bindings;
pub use fix::{bounded, stratified};
pub use totality::Totality;

/// Applies every acceptance rule stated over the normal form to the terms reachable from `roots`.
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
            let c = &schema.constraints[i];
            (c.file, c.query.span)
        }
        Root::Check(i) => {
            let c = schema
                .checks
                .iter()
                .filter(|c| c.relation.0 != u32::MAX)
                .nth(i)
                .expect("one check per checked clause");
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
        Root::Item(_) | Root::Query => unreachable!("a schema has no ad hoc items or queries"),
    }
}

/// Normalises a checked schema's bodies and applies the acceptance rules over their normal forms.
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
        Err(e) => return vec![site(e.root, e.message)],
    };
    let mut roots: Vec<(Root, Tm)> = Vec::new();
    for (i, f) in n.functions.iter().enumerate() {
        if let Some((_, body)) = f {
            roots.push((Root::Function(crate::types::FnId(i as u32)), *body));
        }
    }
    roots.extend(
        n.constraints
            .iter()
            .enumerate()
            .map(|(i, t)| (Root::Constraint(i), *t)),
    );
    roots.extend(
        n.checks
            .iter()
            .enumerate()
            .map(|(i, t)| (Root::Check(i), *t)),
    );
    roots.extend(n.domains.iter().map(|(id, (_, t))| (Root::Domain(*id), *t)));
    roots.extend(n.retags.iter().map(|(r, (_, t))| (Root::Retag(*r), *t)));
    roots.extend(
        n.defaults
            .iter()
            .map(|(&(r, i), t)| (Root::Default(r, i), *t)),
    );
    roots.extend(
        n.computed
            .iter()
            .map(|(&(r, i), (_, t))| (Root::Computed(r, i), *t)),
    );
    roots.extend(
        n.index_fields
            .iter()
            .map(|(&(r, i, j), (_, t))| (Root::IndexField(r, i, j), *t)),
    );
    roots
        .into_iter()
        .filter_map(|(root, t)| accept(&n.terms, &[t]).err().map(|m| site(root, m)))
        .collect()
}

/// Normalises checked ad hoc text and applies the acceptance rules over its items' normal forms, `spans` giving each item's span.
pub(crate) fn adhoc_diags(schema: &Schema, p: &Program, a: &Adhoc, spans: &[Span]) -> Vec<Diag> {
    let n = match norm::adhoc(schema, p, a) {
        Ok(n) => n,
        Err(e) => {
            let Root::Item(i) = e.root else {
                unreachable!("ad hoc errors are at items")
            };
            return vec![Diag::new(spans[i], e.message)];
        }
    };
    n.items
        .iter()
        .zip(spans)
        .filter_map(|(t, span)| accept(&n.terms, &[*t]).err().map(|m| Diag::new(*span, m)))
        .collect()
}

/// Normalises a checked registered query and applies the acceptance rules over its normal form.
pub(crate) fn query_diag(schema: &Schema, p: &Program, q: &Query, span: Span) -> Result<(), Diag> {
    let n = norm::query(schema, p, q).map_err(|e| Diag::new(span, e.message))?;
    accept(&n.terms, &[n.body]).map_err(|m| Diag::new(span, m))
}

/// Normalises a checked registered transaction and applies the acceptance rules over its normal form.
pub(crate) fn txn_diag(schema: &Schema, p: &Program, t: &Txn, span: Span) -> Result<(), Diag> {
    let n = norm::txn(schema, p, t).map_err(|e| Diag::new(span, e.message))?;
    accept(&n.terms, &[n.body]).map_err(|m| Diag::new(span, m))
}
