use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use adtb_lang::types::{DenialIdx, RelId, TypeId};

use crate::exec::{Arena, Footprint, Grant, Val};
use crate::plan::IndexId;

use super::{Error, Rejection};

/// What a statement's write violates, besides the key of a relation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Violator {
    Denial(DenialIdx),
    Unique(IndexId),
    /// A reference field of a relation, by constructor, `None` for a header field, and position.
    Reference(RelId, Option<u32>, usize),
    /// The domain of a newtype.
    Domain(TypeId),
}

/// What a refused statement violated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Violated {
    /// The key of a relation.
    Key(RelId),
    Other(Violator),
}

impl Rejection {
    /// Returns what the statement violated, when it is refused for a violation.
    pub fn violated(&self) -> Option<Violated> {
        match self {
            Rejection::Violated { violator, .. } => Some(Violated::Other(*violator)),
            Rejection::KeyViolation { relation, .. } => Some(Violated::Key(*relation)),
            _ => None,
        }
    }
}

/// The violations a statement found, each with the name its refusal prints, held in a grant.
#[derive(Debug)]
pub struct Violations {
    /// The keys rows of each relation share.
    keys: BTreeMap<RelId, (String, BTreeSet<Val>)>,
    /// The report of each other violation.
    others: BTreeMap<Violator, (String, BTreeSet<Val>)>,
    arena: Arena,
}

impl Violations {
    pub fn new(grant: &Arc<Grant>) -> Violations {
        Violations {
            keys: BTreeMap::new(),
            others: BTreeMap::new(),
            arena: grant.arena(),
        }
    }
}

pub fn violate_key(v: &mut Violations, name: &str, rel: RelId, key: Val) -> Result<(), Error> {
    let fresh = !v.keys.contains_key(&rel);
    v.arena
        .grow(key.footprint() + if fresh { name.len() } else { 0 })?;
    let (_, keys) = v
        .keys
        .entry(rel)
        .or_insert_with(|| (name.to_string(), BTreeSet::new()));
    let n = key.footprint();
    if !keys.insert(key) {
        v.arena.release(n);
    }
    Ok(())
}

pub fn violate(
    v: &mut Violations,
    of: Violator,
    name: &str,
    rows: impl IntoIterator<Item = Val>,
) -> Result<(), Error> {
    let mut rows = rows.into_iter().peekable();
    if rows.peek().is_none() {
        return Ok(());
    }
    if !v.others.contains_key(&of) {
        v.arena.grow(name.len())?;
    }
    let (_, report) = v
        .others
        .entry(of)
        .or_insert_with(|| (name.to_string(), BTreeSet::new()));
    for row in rows {
        let n = row.footprint();
        v.arena.grow(n)?;
        if !report.insert(row) {
            v.arena.release(n);
        }
    }
    Ok(())
}

/// Returns the pairs of distinct rows of every group with more than one, as a unique's report.
pub fn pairs<'a>(
    groups: impl IntoIterator<Item = &'a BTreeMap<Val, Val>>,
) -> impl Iterator<Item = Val> {
    groups.into_iter().flat_map(|g| {
        g.values().flat_map(move |x| {
            g.values()
                .filter(move |y| x != *y)
                .map(move |y| Val::record(vec![x.clone(), y.clone()]))
        })
    })
}

/// Fail with the violation of least name in `viol`, if it holds any.
pub fn refuse(viol: Violations) -> Result<(), Error> {
    let key = viol
        .keys
        .into_iter()
        .min_by(|(_, (a, _)), (_, (b, _))| a.cmp(b));
    let other = viol
        .others
        .into_iter()
        .min_by(|(_, (a, _)), (_, (b, _))| a.cmp(b));
    let rejection = match (key, other) {
        (None, None) => return Ok(()),
        (Some((_, (k, _))), Some((violator, (o, report)))) if o < k => Rejection::Violated {
            violator,
            constraint: o,
            report,
        },
        (Some((relation, (_, keys))), _) => Rejection::KeyViolation {
            relation,
            key: keys.into_iter().next().expect("a key violation has a key"),
        },
        (None, Some((violator, (constraint, report)))) => Rejection::Violated {
            violator,
            constraint,
            report,
        },
    };
    Err(Error::Rejected(rejection))
}
