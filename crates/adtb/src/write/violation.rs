use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::types::{DenialIdx, RelId, TypeId};

use crate::exec::Val;
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

/// The violations a statement found, each with the name its refusal prints.
#[derive(Debug, Default)]
pub struct Violations {
    /// The keys rows of each relation share.
    keys: BTreeMap<RelId, (String, BTreeSet<Val>)>,
    /// The report of each other violation.
    others: BTreeMap<Violator, (String, BTreeSet<Val>)>,
}

impl Violations {
    pub fn new() -> Violations {
        Violations::default()
    }
}

pub fn violate_key(v: &mut Violations, name: &str, rel: RelId, key: Val) {
    let (_, keys) = v
        .keys
        .entry(rel)
        .or_insert_with(|| (name.to_string(), BTreeSet::new()));
    keys.insert(key);
}

pub fn violate(v: &mut Violations, of: Violator, name: &str, rows: impl IntoIterator<Item = Val>) {
    let rows: BTreeSet<Val> = rows.into_iter().collect();
    if rows.is_empty() {
        return;
    }
    let (_, report) = v
        .others
        .entry(of)
        .or_insert_with(|| (name.to_string(), BTreeSet::new()));
    report.extend(rows);
}

/// Returns the pairs of distinct rows of every group with more than one, as a unique's report.
pub fn pairs<'a>(groups: impl IntoIterator<Item = &'a BTreeMap<Val, Val>>) -> BTreeSet<Val> {
    let mut out = BTreeSet::new();
    for g in groups {
        for x in g.values() {
            for y in g.values() {
                if x != y {
                    out.insert(Val::record(vec![x.clone(), y.clone()]));
                }
            }
        }
    }
    out
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
