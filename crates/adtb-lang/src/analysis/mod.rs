//! Admit kernel terms: stratification, boundedness, totality, aggregates and volatile reads.

use std::collections::HashSet;

use crate::capability::{self, Judge, Reach, RootKind};
use crate::check::{Adhoc, AsSchema, Compiled, Query, Txn};
use crate::kernel::{Kind, Terms, Tm};
use crate::norm::{self, Root};
use crate::schema::{DefaultValue, Schema};
use crate::span::{Diag, DiagKind, FileDiag, FileId, Span};

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
pub use totality::{Totality, literal_count};
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
pub(crate) fn schema_diags(c: &Compiled<impl AsSchema>) -> Vec<FileDiag> {
    let (schema, p) = (c.schema(), c.program());
    let site = |root: Root, message: String| {
        let (file, span) = root_site(schema, root);
        FileDiag {
            file,
            diag: Diag::new(span, message),
        }
    };
    let judge = Judge::new(c);
    let refused: Vec<FileDiag> = p
        .bodies()
        .into_iter()
        .filter_map(|(root, t, _)| {
            let r = judge.judge(capability::kind_of(schema, root), &p.terms, t);
            r.err().map(|r| site(root, r.to_string()))
        })
        .collect();
    if !refused.is_empty() {
        return refused;
    }
    let n = match norm::program(c) {
        Ok(n) => n,
        Err(e) => {
            let (file, span) = root_site(schema, e.root);
            return vec![FileDiag {
                file,
                diag: diag(schema, span, e),
            }];
        }
    };
    n.bodies()
        .into_iter()
        .filter_map(|(root, t, _)| accept(n.terms(), &[t]).err().map(|m| site(root, m)))
        .collect()
}

const VOLATILE_WRITE: &str = "a write must not depend on a volatile relation: not in its target, its predicate, its values, or a value or condition it depends on";

/// Normalise checked ad hoc text and apply the acceptance rules over the normal forms of its
/// items and its commands' terms, `spans` giving each item's span.
pub(crate) fn adhoc_diags(c: &Compiled<impl AsSchema>, a: &Adhoc, spans: &[Span]) -> Vec<Diag> {
    let schema = c.schema();
    let items = a.items.iter().zip(spans).map(|(t, span)| (*t, *span));
    let commands = a
        .commands
        .iter()
        .flat_map(|c| c.command.terms().into_iter().map(|t| (t, c.span)));
    let denied: Vec<_> = items
        .chain(commands)
        .filter_map(|(term, span)| system::program(c, &a.terms, [term], span).err())
        .collect();
    if !denied.is_empty() {
        return denied;
    }
    let judge = Judge::new(c);
    let items = a.items.iter().zip(spans).map(|(t, span)| (*t, *span));
    let commands = a
        .commands
        .iter()
        .flat_map(|c| c.command.terms().into_iter().map(|t| (t, c.span)));
    let refused: Vec<Diag> = items
        .chain(commands)
        .filter_map(|(t, span)| {
            let r = judged(schema, &judge, a.kind(), &a.terms, t);
            r.err().map(|m| Diag::new(span, m))
        })
        .collect();
    if !refused.is_empty() {
        return refused;
    }
    let n = match norm::adhoc_over(c, a) {
        Ok(n) => n,
        Err(e) => {
            let span = match e.root {
                Root::Item(i) => spans[i],
                Root::Command(i) => a.commands[i].span,
                _ => unreachable!("ad hoc errors are at items or commands"),
            };
            return vec![diag(schema, span, e)];
        }
    };
    let accepted = |t: Tm| accept(n.terms(), &[t]);
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

/// Returns the diagnostic at `span` of normalisation over `schema` failing with `e`.
fn diag(schema: &Schema, span: Span, e: norm::Error) -> Diag {
    match e.cause {
        norm::Cause::Invalid(..) => Diag::internal(span, e.to_string()),
        norm::Cause::Stuck(_) => Diag::new(span, e.to_string()),
        norm::Cause::TooLarge => Diag {
            span,
            message: e.to_string(),
            kind: DiagKind::PastLimits {
                object: e.root.object(schema),
            },
        },
    }
}

/// Refuse `t`, a checked body of kind `root`, when it reaches what the entry of `root` forbids or a
/// write in it depends on a volatile read, dead terms included.
fn judged(
    schema: &Schema,
    judge: &Judge,
    root: RootKind,
    terms: &Terms,
    t: Tm,
) -> Result<(), String> {
    judge.judge(root, terms, t).map_err(|r| r.to_string())?;
    let volatile = &mut |x| judge.reaches(terms, x).contains(&Reach::Volatile);
    if write_depends_on_volatile(schema, terms, t, volatile) {
        return Err(VOLATILE_WRITE.to_string());
    }
    Ok(())
}

/// Normalise a checked registered query and apply the acceptance rules over its normal form.
pub(crate) fn query_diag(c: &Compiled<impl AsSchema>, q: &Query, span: Span) -> Result<(), Diag> {
    let schema = c.schema();
    system::program(c, &q.terms, [q.term], span)?;
    judged(schema, &Judge::new(c), RootKind::Query, &q.terms, q.term)
        .map_err(|m| Diag::new(span, m))?;
    let n = norm::registered(c, &q.terms, q.term, RootKind::Query)
        .map_err(|e| diag(schema, span, e))?;
    accept(n.terms(), &[n.body()]).map_err(|m| Diag::new(span, m))
}

/// Normalise a checked registered transaction and apply the acceptance rules over its normal form.
pub(crate) fn txn_diag(c: &Compiled<impl AsSchema>, t: &Txn, span: Span) -> Result<(), Diag> {
    let schema = c.schema();
    system::program(c, &t.terms, [t.term], span)?;
    judged(schema, &Judge::new(c), RootKind::Query, &t.terms, t.term)
        .map_err(|m| Diag::new(span, m))?;
    let n = norm::registered(c, &t.terms, t.term, RootKind::Query)
        .map_err(|e| diag(schema, span, e))?;
    accept(n.terms(), &[n.body()]).map_err(|m| Diag::new(span, m))
}
