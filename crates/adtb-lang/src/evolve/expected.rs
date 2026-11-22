use std::ops::Deref;

use super::*;
use crate::schema::{Adder, AheadState};

/// An operator of a migration with its position among the migration's operators, or an
/// operator outside every migration.
#[derive(Clone, Copy, Debug)]
pub struct Listed<'a> {
    at: Option<usize>,
    op: &'a Operator,
}

impl<'a> Listed<'a> {
    /// Returns each operator of `ops` with its position.
    pub fn all(ops: &'a [Operator]) -> impl Iterator<Item = Listed<'a>> {
        ops.iter()
            .enumerate()
            .map(|(k, op)| Listed { at: Some(k), op })
    }

    /// Returns operator `k` of `ops` with its position.
    pub fn nth(ops: &'a [Operator], k: usize) -> Listed<'a> {
        Listed {
            at: Some(k),
            op: &ops[k],
        }
    }

    /// Returns the operator's position in its migration, `None` for one of no migration.
    pub fn at(&self) -> Option<usize> {
        self.at
    }

    /// Returns `op`, an operator of no migration, which adds no relation one expects.
    pub fn alone(op: &'a Operator) -> Listed<'a> {
        Listed { at: None, op }
    }
}

impl Deref for Listed<'_> {
    type Target = Operator;

    fn deref(&self) -> &Operator {
        self.op
    }
}

/// A relation variable an operator of the migration being applied adds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expected {
    module: String,
    /// Its name as the adding operator spells it.
    relation: String,
    /// Whether the migration also adds the relation variable's type.
    typed: bool,
    by: Adder,
    state: Added,
}

/// How far the operator that adds an expected relation variable has gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Added {
    Not,
    Applying,
    /// Added, as the relation variable of this id.
    As(CatRelId),
}

impl Expected {
    /// Returns relation variable `relation` of module `module`, which `by` adds, `typed` when the
    /// migration also adds its type.
    pub fn new(module: String, relation: String, typed: bool, by: Adder) -> Expected {
        Expected {
            module,
            relation,
            typed,
            by,
            state: Added::Not,
        }
    }

    /// Returns relation variable `relation` of module `module` as [`Expected::new`] does, added
    /// as `added` says: not yet when `None`, being added when `Some(None)`, and added as the
    /// relation variable of id `r` when `Some(Some(r))`.
    #[cfg(feature = "compare-migration-model")]
    pub(super) fn added(
        module: String,
        relation: String,
        typed: bool,
        by: Adder,
        added: Option<Option<u64>>,
    ) -> Expected {
        Expected {
            state: match added {
                None => Added::Not,
                Some(None) => Added::Applying,
                Some(Some(r)) => Added::As(CatRelId::from_raw(r)),
            },
            ..Expected::new(module, relation, typed, by)
        }
    }

    /// Returns its module, its name as the adding operator spells it, whether the migration also
    /// adds its type, and its adder.
    #[cfg(feature = "compare-migration-model")]
    pub(super) fn parts(&self) -> (&str, &str, bool, Adder) {
        (&self.module, &self.relation, self.typed, self.by)
    }
}

impl Source {
    /// Returns the relation variables the builder takes as ones a later operator adds, by their
    /// names in this source.
    pub(super) fn ahead_inputs(&self) -> Vec<schema::Ahead> {
        self.ahead
            .iter()
            .filter_map(|e| {
                let (module, relation, state) = match (e.state, e.by) {
                    (Added::Not, _) => (e.module.clone(), e.relation.clone(), AheadState::Pending),
                    (Added::Applying, _) => (
                        e.module.clone(),
                        e.relation.clone(),
                        AheadState::UnlessDeclared,
                    ),
                    (Added::As(id), _) if e.typed => {
                        let (module, relation) = self.relation_named(id)?;
                        (module, relation, AheadState::Declared)
                    }
                    (Added::As(_), _) => return None,
                };
                Some(schema::Ahead::new(module, relation, e.typed, state, e.by))
            })
            .collect()
    }

    /// Mark the relation variable operator `op` adds as being added.
    pub(super) fn adding(&mut self, op: Listed<'_>) {
        for e in self.expected_by(op) {
            e.state = Added::Applying;
        }
    }

    /// Mark the relation variable operator `op` of `module` added as added, with its id.
    pub(super) fn added(&mut self, module: &str, op: Listed<'_>) {
        let OperatorKind::AddRelation(r) = &op.kind else {
            return;
        };
        let id = self
            .relation_key_id(module, &r.name.name.text)
            .expect("an applied `add relation` gives its relation variable an id");
        for e in self.expected_by(op) {
            e.state = Added::As(id);
        }
    }

    /// Mark the relation variable `name` of `module` that `by`, a step of a revert, restored as
    /// added, with its id.
    pub(super) fn restored(&mut self, by: Adder, module: &str, name: &str) {
        let id = self
            .relation_key_id(module, name)
            .expect("a restored relation variable has an id");
        for e in self.ahead.iter_mut().filter(|e| e.by == by) {
            e.state = Added::As(id);
        }
    }

    fn expected_by(&mut self, op: Listed<'_>) -> impl Iterator<Item = &mut Expected> {
        let by = op
            .at
            .filter(|_| matches!(op.kind, OperatorKind::AddRelation(_)))
            .map(Adder::Operator);
        self.ahead.iter_mut().filter(move |e| Some(e.by) == by)
    }
}
