use std::collections::BTreeSet;

use crate::check::Program;
use crate::kernel::{Kind, Terms, Tm};
use crate::schema::Schema;
use crate::types::RelId;

use crate::span::{Diag, Span};

/// Refuse a program write to an engine-defined relation.
pub fn writable_relation(schema: &Schema, target: RelId) -> Result<(), Diag> {
    let rel = schema.relation(target);
    let module = schema.module(rel.module);
    if module.engine {
        Err(Diag::denied(
            Span::default(),
            format!(
                "programs cannot write system relation `{}::{}`",
                module.name, rel.name
            ),
        ))
    } else {
        Ok(())
    }
}

pub(super) fn program(
    schema: &Schema,
    program: &Program,
    terms: &Terms,
    roots: impl IntoIterator<Item = Tm>,
    span: Span,
) -> Result<(), Diag> {
    let mut pending: Vec<_> = roots.into_iter().map(|root| (false, root)).collect();
    let mut visited = BTreeSet::new();
    while let Some((function, term)) = pending.pop() {
        if !visited.insert((function, term)) {
            continue;
        }
        let arena = if function { &program.terms } else { terms };
        let kind = arena.kind(term);
        if let Some(target) = written(kind) {
            writable_relation(schema, target).map_err(|mut diag| {
                diag.span = span;
                diag
            })?;
        }
        if let Kind::Call { func, .. } = kind
            && let Some(body) = program.functions[func.0 as usize]
        {
            pending.push((true, body));
        }
        pending.extend(
            arena
                .children(term)
                .into_iter()
                .map(|child| (function, child)),
        );
    }
    Ok(())
}

fn written(kind: &Kind) -> Option<RelId> {
    match kind {
        Kind::Insert(rel, _)
        | Kind::Delete(rel, _)
        | Kind::Replace(rel, _, _)
        | Kind::Upsert { rel, .. }
        | Kind::Rejects { into: rel, .. } => Some(*rel),
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
        | Kind::Call { .. }
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
        | Kind::Fresh(..)
        | Kind::Only(_)
        | Kind::At { .. }
        | Kind::Present { .. }
        | Kind::Reject(_)
        | Kind::Unreachable
        | Kind::Skip
        | Kind::Par(_)
        | Kind::Fail(_)
        | Kind::LetBang(..)
        | Kind::Block(..)
        | Kind::Transact(..) => None,
    }
}
