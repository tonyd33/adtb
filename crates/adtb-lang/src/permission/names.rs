//! What a role that does not read the schema catalog relations may name in its text.

use std::collections::{BTreeMap, BTreeSet};

use crate::schema::{Schema, TypeDefKind, ValueDef};
use crate::types::{CtorSet, FnId, RelId, Ty, TypeId};

use super::{EffectiveGrants, Right};

/// The user declarations of a schema a role may name. The prelude and the engine's modules state
/// nothing of a schema and stay nameable.
#[derive(Clone, Debug, Default)]
pub struct Nameable {
    relations: BTreeSet<RelId>,
    /// Relations a field of a nameable type references, which only a parameter's key type names,
    /// each with the constructor selections those fields state.
    referenced: BTreeMap<RelId, BTreeSet<CtorSet>>,
    types: BTreeSet<TypeId>,
    functions: BTreeSet<FnId>,
}

impl Nameable {
    /// Returns what `grants` and the `functions` called by grant let a role name: each relation it
    /// holds any grant on, and the types and constructors those relations reach.
    pub fn of(schema: &Schema, grants: &EffectiveGrants, functions: BTreeSet<FnId>) -> Nameable {
        let mut out = Nameable {
            functions,
            ..Nameable::default()
        };
        for grant in &grants.relations {
            let relation = match &grant.right {
                Right::Read { target, .. }
                | Right::Insert(target)
                | Right::Update { target, .. }
                | Right::Delete(target) => target.relation,
                Right::Retag { relation, .. } => *relation,
            };
            if let Some(rel) = schema.ids.relation_of(relation)
                && out.relations.insert(rel)
            {
                out.reach(schema, schema.relation(rel).elem_def);
            }
        }
        out
    }

    /// Returns these names with `relations` also nameable.
    #[must_use]
    pub fn naming(&self, relations: impl IntoIterator<Item = RelId>) -> Nameable {
        let mut out = self.clone();
        out.relations.extend(relations);
        out
    }

    fn reach(&mut self, schema: &Schema, def: TypeId) {
        if !self.types.insert(def) {
            return;
        }
        let ty = schema.ty(def);
        let fields = ty.header().iter().map(|f| &f.ty);
        let fields = fields.chain(
            ty.ctors()
                .iter()
                .flat_map(|c| c.fields.iter().map(|f| &f.ty)),
        );
        let mut found: Vec<&Ty> = fields.collect();
        if let TypeDefKind::Newtype { base, .. } = &ty.kind {
            found.push(base);
        }
        for t in found {
            self.reach_ty(schema, t);
        }
    }

    fn reach_ty(&mut self, schema: &Schema, ty: &Ty) {
        match ty {
            Ty::Data { def, args, .. } => {
                self.reach(schema, *def);
                for a in args {
                    self.reach_ty(schema, a);
                }
            }
            Ty::Key { rel, ctors } => {
                let selections = self.referenced.entry(*rel).or_default();
                selections.extend(ctors.clone());
            }
            Ty::Set(t) | Ty::NonEmpty(t) | Ty::Tag(t) => self.reach_ty(schema, t),
            Ty::Group(a, b) | Ty::Fn(a, b) => {
                self.reach_ty(schema, a);
                self.reach_ty(schema, b);
            }
            Ty::Record(fields) => {
                for (_, t) in fields {
                    self.reach_ty(schema, t);
                }
            }
            Ty::Sum { ctors, .. } => {
                for (_, fields) in ctors {
                    for (_, t) in fields {
                        self.reach_ty(schema, t);
                    }
                }
            }
            Ty::Base(_) | Ty::Param(_) | Ty::Unit | Ty::Change => {}
        }
    }

    /// Returns `schema` with every other user declaration out of its names, its ids unchanged.
    pub fn view(&self, schema: &Schema) -> Schema {
        let mut out = schema.clone();
        for module in &mut out.modules {
            if module.engine {
                continue;
            }
            module.types.retain(|_, t| self.types.contains(t));
            module.values.retain(|_, v| match v {
                ValueDef::Relation(r) => self.relations.contains(r),
                ValueDef::Function(f) => self.functions.contains(f),
                ValueDef::Constraint(_) | ValueDef::Check(_) | ValueDef::Aggregate(_) => false,
            });
            module.hidden = module.types.is_empty() && module.values.is_empty();
        }
        out
    }

    /// Returns whether a parameter of a role may have type `ty`, which `schema` declares.
    pub fn may_type(&self, schema: &Schema, ty: &Ty) -> bool {
        let open = |module| schema.module(module).engine;
        match ty {
            Ty::Data { def, args, .. } => {
                (self.types.contains(def) || open(schema.ty(*def).module))
                    && args.iter().all(|t| self.may_type(schema, t))
            }
            Ty::Key { rel, ctors } => {
                self.relations.contains(rel)
                    || open(schema.relation(*rel).module)
                    || self.referenced.get(rel).is_some_and(|selections| {
                        ctors.as_ref().is_none_or(|c| selections.contains(c))
                    })
            }
            Ty::Set(t) | Ty::NonEmpty(t) | Ty::Tag(t) => self.may_type(schema, t),
            Ty::Group(a, b) | Ty::Fn(a, b) => self.may_type(schema, a) && self.may_type(schema, b),
            Ty::Record(fields) => fields.iter().all(|(_, t)| self.may_type(schema, t)),
            Ty::Sum { ctors, .. } => ctors
                .iter()
                .all(|(_, fields)| fields.iter().all(|(_, t)| self.may_type(schema, t))),
            Ty::Base(_) | Ty::Param(_) | Ty::Unit | Ty::Change => true,
        }
    }
}
