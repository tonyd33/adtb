use std::collections::BTreeSet;

use super::{Ahead, Schema, SchemaDefs, Scope, TypeDefKind, TypeName};
use crate::ast;
use crate::resolve::Unbound;
use crate::types::ModuleId;
use crate::types::{AheadId, RelId, RelRef, Ty, TypeId};

/// What adds a relation variable the migration being applied expects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Adder {
    /// The operator at this position of the migration.
    Operator(usize),
    /// The inverse at this index of the step at this position of a revert.
    Inverse(usize, usize),
}

/// Whether the operator that adds an expected relation variable has applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AheadState {
    /// It has not: a key of the relation names it as one a later operator adds, unless a value
    /// of its name is declared.
    Pending,
    /// It has: the relation is declared under the name the entry gives.
    Declared,
    /// It is being applied: it has once a value of the entry's name is declared.
    UnlessDeclared,
    /// It was refused: the name is poisoned, and a lookup reaching it reports nothing.
    Refused,
}

/// What an [`Ahead`] entry names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AheadKind {
    Relation,
    /// A type; its entry is always [`AheadState::Refused`].
    Type,
    /// A function; its entry is always [`AheadState::Refused`].
    Function,
}

impl Ahead {
    /// Returns the relation variable `relation` of module `module`, which `by` adds, `typed`
    /// when the migration also adds its type.
    pub fn new(
        module: String,
        relation: String,
        typed: bool,
        state: AheadState,
        by: Adder,
    ) -> Ahead {
        Ahead {
            module,
            relation,
            typed,
            state,
            by,
            kind: AheadKind::Relation,
        }
    }

    /// Returns the type or function `name` of module `module`, which the operator `by` would
    /// have declared had the migration not refused it.
    pub fn refused(module: String, name: String, kind: AheadKind, by: Adder) -> Ahead {
        Ahead {
            kind,
            ..Ahead::new(module, name, false, AheadState::Refused, by)
        }
    }
}

impl Schema {
    /// Returns the type `name` names as [`Schema::lookup_type_in`] does.
    ///
    /// Fail with `Unbound::Poisoned` when it finds none and names a type a refused operator left
    /// poisoned.
    pub(crate) fn find_type_in(
        &self,
        module: ModuleId,
        name: &ast::QName,
        scope: Option<&Scope>,
    ) -> Result<TypeName, Unbound> {
        self.lookup_type_in(module, name, scope).map_err(|e| {
            let owner = match &name.module {
                Some(m) => self.module_named(&m.text),
                None => Some(module),
            };
            match owner {
                Some(m) if self.poisoned_type(m, &name.name.text) => Unbound::Poisoned,
                _ => Unbound::Missing(e),
            }
        })
    }

    /// Returns what adds relation variable `id`.
    pub fn ahead_adder(&self, id: AheadId) -> Adder {
        self.ahead[id.index()].2
    }

    /// Returns the refusal of a relation a later operator of the migration adds when a value of
    /// `ty` can hold a key of it.
    pub fn holds_no_ahead_key(&self, ty: &Ty) -> Result<(), String> {
        match self.ahead_held(ty, &mut BTreeSet::new(), &mut BTreeSet::new()) {
            Some(a) => Err(self.added_later(a)),
            None => Ok(()),
        }
    }

    /// Returns a relation a later operator of the migration adds that a key a value of `ty` can
    /// hold names, through the types and relations not in `defs` and `rels`, adding those it
    /// passes. A tag holds only a constructor's name.
    fn ahead_held(
        &self,
        ty: &Ty,
        defs: &mut BTreeSet<TypeId>,
        rels: &mut BTreeSet<RelId>,
    ) -> Option<AheadId> {
        let mut inner = |t: &Ty| self.ahead_held(t, defs, rels);
        match ty {
            Ty::Key {
                rel: RelRef::Ahead(a),
                ..
            } => Some(*a),
            Ty::Key {
                rel: RelRef::Declared(rel),
                ..
            } => {
                if !rels.insert(*rel) {
                    return None;
                }
                let r = self.relation(*rel);
                let elem = self.ty(r.elem_def);
                let keys: Vec<Ty> = r
                    .key
                    .iter()
                    .filter_map(|k| elem.field_named(0, k).map(|f| f.ty.subst(r.elem_args())))
                    .collect();
                keys.iter().find_map(|t| self.ahead_held(t, defs, rels))
            }
            Ty::Data { def, args, .. } => {
                if let Some(a) = args.iter().find_map(&mut inner) {
                    return Some(a);
                }
                if !defs.insert(*def) {
                    return None;
                }
                match &self.ty(*def).kind {
                    TypeDefKind::Sum { header, ctors, .. } => header
                        .iter()
                        .chain(ctors.iter().flat_map(|c| &c.fields))
                        .find_map(|f| self.ahead_held(&f.ty, defs, rels)),
                    TypeDefKind::Newtype { base, .. } => self.ahead_held(base, defs, rels),
                }
            }
            Ty::Set(t) | Ty::NonEmpty(t) => inner(t),
            Ty::Group(a, b) | Ty::Fn(a, b) => inner(a).or_else(|| inner(b)),
            Ty::Record(fs) => fs.iter().find_map(|(_, t)| inner(t)),
            Ty::Sum { ctors, .. } => ctors
                .iter()
                .find_map(|(_, fs)| fs.iter().find_map(|(_, t)| inner(t))),
            Ty::Tag(_) | Ty::Base(_) | Ty::Param(_) | Ty::Unit | Ty::Change => None,
        }
    }
}

impl SchemaDefs {
    /// Returns whether `name` of `module` names a relation variable or a function a refused
    /// operator of the migration being applied would have declared.
    pub fn poisoned(&self, module: ModuleId, name: &str) -> bool {
        self.poisoned.contains(&(module, name.to_string()))
    }

    /// Returns whether `name` of `module` names a type a refused operator of the migration being
    /// applied would have declared.
    pub fn poisoned_type(&self, module: ModuleId, name: &str) -> bool {
        self.poisoned_types.contains(&(module, name.to_string()))
    }
}
