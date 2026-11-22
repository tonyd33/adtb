use std::ops::Deref;

use super::*;
use crate::schema::{Adder, AheadKind, AheadState};

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

/// A relation variable an operator of the migration being applied adds, or a type or function
/// a refused operator of it would have declared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expected {
    module: String,
    /// Its name as the adding operator spells it.
    relation: String,
    /// Whether the migration also adds the relation variable's type.
    typed: bool,
    by: Adder,
    state: Added,
    kind: AheadKind,
}

/// How far the operator that adds an expected relation variable has gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Added {
    Not,
    Applying,
    /// Refused: its name stays bound, poisoned, to the end of the migration.
    Refused,
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
            kind: AheadKind::Relation,
        }
    }

    /// Returns the type or function `name` of module `module`, which the refused operator `by`
    /// would have declared.
    pub(super) fn refused(module: String, name: String, kind: AheadKind, by: Adder) -> Expected {
        Expected {
            state: Added::Refused,
            kind,
            ..Expected::new(module, name, false, by)
        }
    }

    /// Returns relation variable `relation` of module `module` as [`Expected::new`] does, its
    /// adding operator gone as far as `state`.
    #[cfg(feature = "compare-migration-model")]
    pub(super) fn added(
        module: String,
        relation: String,
        typed: bool,
        by: Adder,
        state: Added,
        kind: AheadKind,
    ) -> Expected {
        Expected {
            state,
            kind,
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
                    (Added::Refused, _) if e.kind != AheadKind::Relation => {
                        return Some(schema::Ahead::refused(
                            e.module.clone(),
                            e.relation.clone(),
                            e.kind,
                            e.by,
                        ));
                    }
                    (Added::Refused, _) => {
                        (e.module.clone(), e.relation.clone(), AheadState::Refused)
                    }
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

    /// Poison the name of the relation variable, type or function operator `op` of a migration
    /// of `module` would have declared.
    pub(super) fn refused(&mut self, module: &str, op: Listed<'_>) {
        for e in self.expected_by(op) {
            e.state = Added::Refused;
        }
        let (Some(at), Some((name, kind))) = (op.at, poisons(&op.kind)) else {
            return;
        };
        self.ahead.push(Expected::refused(
            module.to_string(),
            name.to_string(),
            kind,
            Adder::Operator(at),
        ));
    }

    /// Keep only the relation variables whose adding operator was refused.
    pub(super) fn keep_refused(&mut self) {
        self.ahead.retain(|e| e.state == Added::Refused);
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

/// Returns the name and kind of the type or function `op` declares.
pub(super) fn poisons(op: &OperatorKind) -> Option<(&str, AheadKind)> {
    match op {
        OperatorKind::AddType(t) => Some((&t.name.text, AheadKind::Type)),
        OperatorKind::DefineFn(f) => Some((&f.name.text, AheadKind::Function)),
        _ => None,
    }
}
