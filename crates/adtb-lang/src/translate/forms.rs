//! What the form of a term at the pinned version says of the rows it holds.

#![deny(
    clippy::wildcard_enum_match_arm,
    clippy::match_wildcard_for_single_variants
)]

use std::collections::HashSet;

use crate::kernel::{Kind, Search, Terms, Tm, Var};

/// Returns whether `t` reads a row a conflict probe returned, `probed` the variables bound to one.
pub(super) fn reads_probed(terms: &Terms, probed: &HashSet<Var>, t: Tm) -> bool {
    match terms.kind(t) {
        Kind::Var(v) => probed.contains(v),
        Kind::Field(x, _) => reads_probed(terms, probed, *x),
        Kind::Const(..)
        | Kind::Prim(..)
        | Kind::Record(..)
        | Kind::Ctor { .. }
        | Kind::Case { .. }
        | Kind::If(..)
        | Kind::Let(..)
        | Kind::Lam(..)
        | Kind::App(..)
        | Kind::Call { .. }
        | Kind::Caller
        | Kind::Key(..)
        | Kind::KeyOf(..)
        | Kind::Deref { .. }
        | Kind::Tag(..)
        | Kind::Try(..)
        | Kind::Rel(..)
        | Kind::Transition(..)
        | Kind::Restrict(..)
        | Kind::EmptySet
        | Kind::Singleton(..)
        | Kind::Union(..)
        | Kind::For(..)
        | Kind::IsEmpty(..)
        | Kind::Exists(..)
        | Kind::Agg { .. }
        | Kind::Group { .. }
        | Kind::Top { .. }
        | Kind::Fix(..)
        | Kind::Fresh(..)
        | Kind::Only(..)
        | Kind::At { .. }
        | Kind::Present { .. }
        | Kind::Reject(..)
        | Kind::Unreachable
        | Kind::Skip
        | Kind::Insert(..)
        | Kind::Delete(..)
        | Kind::Replace(..)
        | Kind::Upsert { .. }
        | Kind::Rejects { .. }
        | Kind::Par(..)
        | Kind::Fail(..)
        | Kind::LetBang(..)
        | Kind::Block(..)
        | Kind::Transact(..) => false,
    }
}

/// Returns the probed row a set of rows `t` updates or deletes holds at most, as `update old`
/// makes it.
pub(super) fn probed_rows(terms: &Terms, probed: &HashSet<Var>, t: Tm) -> Option<Var> {
    match terms.kind(t) {
        Kind::Singleton(x) => match terms.kind(*x) {
            Kind::Var(v) if probed.contains(v) => Some(*v),
            Kind::Var(..)
            | Kind::Const(..)
            | Kind::Prim(..)
            | Kind::Record(..)
            | Kind::Field(..)
            | Kind::Ctor { .. }
            | Kind::Case { .. }
            | Kind::If(..)
            | Kind::Let(..)
            | Kind::Lam(..)
            | Kind::App(..)
            | Kind::Call { .. }
            | Kind::Caller
            | Kind::Key(..)
            | Kind::KeyOf(..)
            | Kind::Deref { .. }
            | Kind::Tag(..)
            | Kind::Try(..)
            | Kind::Rel(..)
            | Kind::Transition(..)
            | Kind::Restrict(..)
            | Kind::EmptySet
            | Kind::Singleton(..)
            | Kind::Union(..)
            | Kind::For(..)
            | Kind::IsEmpty(..)
            | Kind::Exists(..)
            | Kind::Agg { .. }
            | Kind::Group { .. }
            | Kind::Top { .. }
            | Kind::Fix(..)
            | Kind::Fresh(..)
            | Kind::Only(..)
            | Kind::At { .. }
            | Kind::Present { .. }
            | Kind::Reject(..)
            | Kind::Unreachable
            | Kind::Skip
            | Kind::Insert(..)
            | Kind::Delete(..)
            | Kind::Replace(..)
            | Kind::Upsert { .. }
            | Kind::Rejects { .. }
            | Kind::Par(..)
            | Kind::Fail(..)
            | Kind::LetBang(..)
            | Kind::Block(..)
            | Kind::Transact(..) => None,
        },
        Kind::If(_, rows, _) => probed_rows(terms, probed, *rows),
        Kind::Var(..)
        | Kind::Const(..)
        | Kind::Prim(..)
        | Kind::Record(..)
        | Kind::Field(..)
        | Kind::Ctor { .. }
        | Kind::Case { .. }
        | Kind::Let(..)
        | Kind::Lam(..)
        | Kind::App(..)
        | Kind::Call { .. }
        | Kind::Caller
        | Kind::Key(..)
        | Kind::KeyOf(..)
        | Kind::Deref { .. }
        | Kind::Tag(..)
        | Kind::Try(..)
        | Kind::Rel(..)
        | Kind::Transition(..)
        | Kind::Restrict(..)
        | Kind::EmptySet
        | Kind::Union(..)
        | Kind::For(..)
        | Kind::IsEmpty(..)
        | Kind::Exists(..)
        | Kind::Agg { .. }
        | Kind::Group { .. }
        | Kind::Top { .. }
        | Kind::Fix(..)
        | Kind::Fresh(..)
        | Kind::Only(..)
        | Kind::At { .. }
        | Kind::Present { .. }
        | Kind::Reject(..)
        | Kind::Unreachable
        | Kind::Skip
        | Kind::Insert(..)
        | Kind::Delete(..)
        | Kind::Replace(..)
        | Kind::Upsert { .. }
        | Kind::Rejects { .. }
        | Kind::Par(..)
        | Kind::Fail(..)
        | Kind::LetBang(..)
        | Kind::Block(..)
        | Kind::Transact(..) => None,
    }
}

/// Returns whether `set` holds only rows read from one relation: the relation, a constructor
/// relation of it, or a comprehension over one that yields its bound row.
pub(super) fn rows_of(terms: &Terms, set: Tm) -> bool {
    match terms.kind(set) {
        Kind::Rel(..) | Kind::Transition(..) => true,
        Kind::Restrict(x, _) => rows_of(terms, *x),
        Kind::For(v, source, body) => rows_of(terms, *source) && yields(terms, *body, *v),
        Kind::Var(..)
        | Kind::Const(..)
        | Kind::Prim(..)
        | Kind::Record(..)
        | Kind::Field(..)
        | Kind::Ctor { .. }
        | Kind::Case { .. }
        | Kind::If(..)
        | Kind::Let(..)
        | Kind::Lam(..)
        | Kind::App(..)
        | Kind::Call { .. }
        | Kind::Caller
        | Kind::Key(..)
        | Kind::KeyOf(..)
        | Kind::Deref { .. }
        | Kind::Tag(..)
        | Kind::Try(..)
        | Kind::EmptySet
        | Kind::Singleton(..)
        | Kind::Union(..)
        | Kind::IsEmpty(..)
        | Kind::Exists(..)
        | Kind::Agg { .. }
        | Kind::Group { .. }
        | Kind::Top { .. }
        | Kind::Fix(..)
        | Kind::Fresh(..)
        | Kind::Only(..)
        | Kind::At { .. }
        | Kind::Present { .. }
        | Kind::Reject(..)
        | Kind::Unreachable
        | Kind::Skip
        | Kind::Insert(..)
        | Kind::Delete(..)
        | Kind::Replace(..)
        | Kind::Upsert { .. }
        | Kind::Rejects { .. }
        | Kind::Par(..)
        | Kind::Fail(..)
        | Kind::LetBang(..)
        | Kind::Block(..)
        | Kind::Transact(..) => false,
    }
}

/// Returns whether every element of `set` is the value of `v`.
pub(super) fn yields(terms: &Terms, set: Tm, v: Var) -> bool {
    !terms.search(set, |t| match terms.kind(t) {
        Kind::EmptySet => Search::Into(Vec::new()),
        Kind::Singleton(x) if matches!(terms.kind(*x), Kind::Var(y) if *y == v) => {
            Search::Into(Vec::new())
        }
        Kind::If(_, a, b) => Search::Into(vec![*a, *b]),
        Kind::Let(_, _, body) | Kind::For(_, _, body) => Search::Into(vec![*body]),
        Kind::Case { arms, .. } => Search::Into(arms.iter().map(|&(_, _, body)| body).collect()),
        Kind::Var(..)
        | Kind::Const(..)
        | Kind::Prim(..)
        | Kind::Record(..)
        | Kind::Field(..)
        | Kind::Ctor { .. }
        | Kind::Lam(..)
        | Kind::App(..)
        | Kind::Call { .. }
        | Kind::Caller
        | Kind::Key(..)
        | Kind::KeyOf(..)
        | Kind::Deref { .. }
        | Kind::Tag(..)
        | Kind::Try(..)
        | Kind::Rel(..)
        | Kind::Transition(..)
        | Kind::Restrict(..)
        | Kind::Singleton(..)
        | Kind::Union(..)
        | Kind::IsEmpty(..)
        | Kind::Exists(..)
        | Kind::Agg { .. }
        | Kind::Group { .. }
        | Kind::Top { .. }
        | Kind::Fix(..)
        | Kind::Fresh(..)
        | Kind::Only(..)
        | Kind::At { .. }
        | Kind::Present { .. }
        | Kind::Reject(..)
        | Kind::Unreachable
        | Kind::Skip
        | Kind::Insert(..)
        | Kind::Delete(..)
        | Kind::Replace(..)
        | Kind::Upsert { .. }
        | Kind::Rejects { .. }
        | Kind::Par(..)
        | Kind::Fail(..)
        | Kind::LetBang(..)
        | Kind::Block(..)
        | Kind::Transact(..) => Search::Found,
    })
}
