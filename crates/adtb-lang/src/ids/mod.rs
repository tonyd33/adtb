//! Assign and hold the catalog ids of a schema's objects.

use std::collections::BTreeMap;

use crate::ast;
use crate::schema::{CtorRef, FieldRef, Schema, TypeDefKind};
use crate::span::FileId;
use crate::types::*;

#[cfg(test)]
mod tests;

mod fixed;
mod key;
mod named;
mod objects;
mod reach;
pub use fixed::{FixedType, RELATIONS, TYPES};
pub use key::{KeyKind, ObjectKey, Owner, Path, PathStep, forget};
pub use named::{is_named, named_ids};
pub use objects::{Object, assign, objects};
pub use reach::Replacement;

macro_rules! catalog_id {
    ($($(#[$doc:meta])* $name:ident),* $(,)?) => {$(
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            /// Returns the id whose stored form is `raw`.
            pub const fn from_raw(raw: u64) -> Self {
                Self(raw)
            }

            /// Returns the id's stored form.
            pub const fn raw(self) -> u64 {
                self.0
            }
        }

        impl From<$name> for u64 {
            fn from(id: $name) -> u64 {
                id.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    )*};
}

catalog_id!(
    /// The catalog id of a type.
    CatTypeId,
    /// The catalog id of a constructor.
    CatCtorId,
    /// The catalog id of a relation variable, a stored view's included.
    CatRelId,
    /// The catalog id of a field.
    CatFieldId,
    /// The catalog id of a named constructor set.
    CatSetId,
    /// The catalog id of a key index, a unique or an index.
    CatIndexId,
    /// The catalog id of a default.
    CatDefaultId,
    /// The catalog id of a computed field.
    CatComputedId,
    /// The catalog id of a function or an aggregate.
    CatFunctionId,
    /// The catalog id of a constraint or a check.
    CatConstraintId,
);

/// The id of a temp relation of ad hoc text, from a space of its own: never stored, and never
/// a catalog id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TempRelId(u32);

impl TempRelId {
    /// Returns the id after `last`, the first when `None`.
    pub(crate) fn after(last: Option<TempRelId>) -> TempRelId {
        TempRelId(last.map_or(0, |t| t.0 + 1))
    }
}

/// The catalog ids of a schema's objects, by the handle of each in the schema.
///
/// Every object of a user module has an id, and every object of an engine module its fixed id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatalogIds {
    types: BTreeMap<TypeId, CatTypeId>,
    ctors: BTreeMap<CtorRef, CatCtorId>,
    relations: BTreeMap<RelId, CatRelId>,
    fields: BTreeMap<FieldRef, CatFieldId>,
    sets: BTreeMap<(RelId, String), CatSetId>,
    /// The index of each relation's key.
    keys: BTreeMap<RelId, CatIndexId>,
    /// Uniques and indexes, by relation and position among `RelationDef::indexes`.
    indexes: BTreeMap<(RelId, usize), CatIndexId>,
    /// The index each reference field implies, by relation and field.
    implied: BTreeMap<(RelId, FieldRef), CatIndexId>,
    /// The default a key's `generate` gives.
    generated: BTreeMap<RelId, CatDefaultId>,
    /// Defaults, one for each field a default clause fills, by relation, position among
    /// `RelationDef::defaults`, and the field.
    defaults: BTreeMap<(RelId, usize, FieldRef), CatDefaultId>,
    /// Computed fields, by relation and position among `RelationDef::computed`.
    computed: BTreeMap<(RelId, usize), CatComputedId>,
    functions: BTreeMap<FnId, CatFunctionId>,
    /// Aggregates, by position among `Schema::aggregates`.
    aggregates: BTreeMap<usize, CatFunctionId>,
    /// Constraints, by position among `Schema::constraints`.
    constraints: BTreeMap<usize, CatConstraintId>,
    /// Checks, by position among `Schema::checks`.
    checks: BTreeMap<usize, CatConstraintId>,
    /// The relation variable of each stored view.
    views: BTreeMap<FnId, CatRelId>,
    /// The key index of each stored view.
    view_keys: BTreeMap<FnId, CatIndexId>,
    /// A stored view's uniques and indexes, by position among its clauses of those kinds.
    view_indexes: BTreeMap<(FnId, usize), CatIndexId>,
    /// The type of each stored view's rows, for a view whose element type is not a declared type.
    view_types: BTreeMap<FnId, CatTypeId>,
    /// The one constructor of each stored view's row type.
    view_ctors: BTreeMap<FnId, CatCtorId>,
    /// The fields of a stored view's row constructor, by view and name.
    view_fields: BTreeMap<(FnId, String), CatFieldId>,
    /// The object holding each id of each space.
    held: BTreeMap<(Space, u64), Slot>,
}

/// An id space of catalog objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Space {
    /// Types, constructors and relation variables.
    Shared,
    Field,
    Set,
    Index,
    Constraint,
    Function,
    Default,
    Computed,
}

/// Where an object's id goes in [`CatalogIds`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Slot {
    Type(TypeId),
    Ctor(CtorRef),
    Relation(RelId),
    Field(FieldRef),
    Set(RelId, String),
    Key(RelId),
    Index(RelId, usize),
    Implied(RelId, FieldRef),
    Generated(RelId),
    Default(RelId, usize, FieldRef),
    Computed(RelId, usize),
    Function(FnId),
    Aggregate(usize),
    Constraint(usize),
    Check(usize),
    View(FnId),
    ViewKey(FnId),
    ViewIndex(FnId, usize),
    ViewType(FnId),
    ViewCtor(FnId),
    ViewField(FnId, String),
}

impl Slot {
    pub fn space(&self) -> Space {
        match self {
            Slot::Type(_)
            | Slot::Ctor(_)
            | Slot::Relation(_)
            | Slot::View(_)
            | Slot::ViewType(_)
            | Slot::ViewCtor(_) => Space::Shared,
            Slot::Field(_) | Slot::ViewField(..) => Space::Field,
            Slot::Set(..) => Space::Set,
            Slot::Key(_)
            | Slot::Index(..)
            | Slot::Implied(..)
            | Slot::ViewKey(_)
            | Slot::ViewIndex(..) => Space::Index,
            Slot::Generated(_) | Slot::Default(..) => Space::Default,
            Slot::Computed(..) => Space::Computed,
            Slot::Function(_) | Slot::Aggregate(_) => Space::Function,
            Slot::Constraint(_) | Slot::Check(_) => Space::Constraint,
        }
    }
}

/// Returns the id a lookup found for an object of a built schema.
///
/// Preconditions: `id` is `Some`; every object of a built schema outside the temp module has a
/// catalog id.
#[track_caller]
pub fn held_id<T>(id: Option<T>) -> T {
    id.expect("every object of a built schema outside the temp module has a catalog id")
}

impl CatalogIds {
    pub fn type_id(&self, def: TypeId) -> Option<CatTypeId> {
        self.types.get(&def).copied()
    }

    pub fn ctor_id(&self, c: CtorRef) -> Option<CatCtorId> {
        self.ctors.get(&c).copied()
    }

    pub fn relation_id(&self, rel: RelId) -> Option<CatRelId> {
        self.relations.get(&rel).copied()
    }

    pub fn field_id(&self, f: FieldRef) -> Option<CatFieldId> {
        self.fields.get(&f).copied()
    }

    pub fn set_id(&self, rel: RelId, name: &str) -> Option<CatSetId> {
        self.sets.get(&(rel, name.to_string())).copied()
    }

    pub fn key_id(&self, rel: RelId) -> Option<CatIndexId> {
        self.keys.get(&rel).copied()
    }

    /// Returns the id of the unique or index at position `at` of `RelationDef::indexes`.
    pub fn index_id(&self, rel: RelId, at: usize) -> Option<CatIndexId> {
        self.indexes.get(&(rel, at)).copied()
    }

    /// Returns the id of the index reference field `f` of `rel` implies.
    pub fn implied_id(&self, rel: RelId, f: FieldRef) -> Option<CatIndexId> {
        self.implied.get(&(rel, f)).copied()
    }

    pub fn generated_id(&self, rel: RelId) -> Option<CatDefaultId> {
        self.generated.get(&rel).copied()
    }

    /// Returns the defaults the default clause at position `at` of `RelationDef::defaults` gives,
    /// by the field each fills.
    pub fn clause_default_ids(
        &self,
        rel: RelId,
        at: usize,
    ) -> impl Iterator<Item = (FieldRef, CatDefaultId)> + '_ {
        self.defaults
            .iter()
            .filter(move |((r, a, _), _)| (*r, *a) == (rel, at))
            .map(|(&(_, _, f), &id)| (f, id))
    }

    pub fn computed_id(&self, rel: RelId, at: usize) -> Option<CatComputedId> {
        self.computed.get(&(rel, at)).copied()
    }

    pub fn function_id(&self, f: FnId) -> Option<CatFunctionId> {
        self.functions.get(&f).copied()
    }

    /// Returns the id of the aggregate at position `at` of `Schema::aggregates`.
    pub fn aggregate_id(&self, at: usize) -> Option<CatFunctionId> {
        self.aggregates.get(&at).copied()
    }

    pub fn constraint_id(&self, c: ConstraintIdx) -> Option<CatConstraintId> {
        self.constraints.get(&c.index()).copied()
    }

    pub fn check_id(&self, c: CheckIdx) -> Option<CatConstraintId> {
        self.checks.get(&c.index()).copied()
    }

    /// Returns the id of the relation variable stored view `f` holds.
    pub fn view_id(&self, f: FnId) -> Option<CatRelId> {
        self.views.get(&f).copied()
    }

    /// Returns the id of the row type of stored view `f`, a view whose element type is not a
    /// declared type.
    pub fn view_type_id(&self, f: FnId) -> Option<CatTypeId> {
        self.view_types.get(&f).copied()
    }

    pub fn view_key_id(&self, f: FnId) -> Option<CatIndexId> {
        self.view_keys.get(&f).copied()
    }

    /// Returns the id of the one constructor of the row type of stored view `f`.
    pub fn view_ctor_id(&self, f: FnId) -> Option<CatCtorId> {
        self.view_ctors.get(&f).copied()
    }

    /// Returns the id of field `name` of the row constructor of stored view `f`.
    pub fn view_field_id(&self, f: FnId, name: &str) -> Option<CatFieldId> {
        self.view_fields.get(&(f, name.to_string())).copied()
    }

    /// Returns the object of `space` whose id is `id`.
    fn holder(&self, space: Space, id: u64) -> Option<&Slot> {
        self.held.get(&(space, id))
    }

    /// Returns the type whose id is `id`.
    pub fn type_of(&self, id: CatTypeId) -> Option<TypeId> {
        match self.holder(Space::Shared, id.0)? {
            Slot::Type(t) => Some(*t),
            _ => None,
        }
    }

    /// Returns the constructor whose id is `id`.
    pub fn ctor_of(&self, id: CatCtorId) -> Option<CtorRef> {
        match self.holder(Space::Shared, id.0)? {
            Slot::Ctor(c) => Some(*c),
            _ => None,
        }
    }

    /// Returns the relation variable whose id is `id`.
    pub fn relation_of(&self, id: CatRelId) -> Option<RelId> {
        match self.holder(Space::Shared, id.0)? {
            Slot::Relation(r) => Some(*r),
            _ => None,
        }
    }

    /// Returns the field whose id is `id`.
    pub fn field_of(&self, id: CatFieldId) -> Option<FieldRef> {
        match self.holder(Space::Field, id.0)? {
            Slot::Field(f) => Some(*f),
            _ => None,
        }
    }

    /// Returns the named set, as relation and name, whose id is `id`.
    pub fn set_of(&self, id: CatSetId) -> Option<(RelId, &str)> {
        match self.holder(Space::Set, id.0)? {
            Slot::Set(r, n) => Some((*r, n.as_str())),
            _ => None,
        }
    }

    /// Returns the function whose id is `id`.
    pub fn function_of(&self, id: CatFunctionId) -> Option<FnId> {
        match self.holder(Space::Function, id.0)? {
            Slot::Function(f) => Some(*f),
            _ => None,
        }
    }

    /// Returns the unique or index, as relation and position among `RelationDef::indexes`, whose
    /// id is `id`.
    pub fn index_of(&self, id: CatIndexId) -> Option<(RelId, usize)> {
        match self.holder(Space::Index, id.0)? {
            Slot::Index(r, at) => Some((*r, *at)),
            _ => None,
        }
    }

    /// Returns the constraint whose id is `id`.
    pub fn constraint_of(&self, id: CatConstraintId) -> Option<ConstraintIdx> {
        match self.holder(Space::Constraint, id.0)? {
            Slot::Constraint(at) => Some(ConstraintIdx::at(*at)),
            _ => None,
        }
    }

    /// Returns the aggregate, as position among `Schema::aggregates`, whose id is `id`.
    pub fn aggregate_of(&self, id: CatFunctionId) -> Option<usize> {
        match self.holder(Space::Function, id.0)? {
            Slot::Aggregate(at) => Some(*at),
            _ => None,
        }
    }

    pub fn type_ids(&self) -> impl Iterator<Item = (TypeId, CatTypeId)> + '_ {
        self.types.iter().map(|(&t, &id)| (t, id))
    }

    pub fn ctor_ids(&self) -> impl Iterator<Item = (CtorRef, CatCtorId)> + '_ {
        self.ctors.iter().map(|(&c, &id)| (c, id))
    }

    /// Returns each constructor of `def` by index.
    pub fn type_ctor_ids(&self, def: TypeId) -> impl Iterator<Item = (CtorRef, CatCtorId)> + '_ {
        self.ctors
            .iter()
            .skip_while(move |(c, _)| c.def() < def)
            .take_while(move |(c, _)| c.def() == def)
            .map(|(&c, &id)| (c, id))
    }

    /// Returns each field of `def`, the header's first, then each constructor's own by position.
    pub fn type_field_ids(&self, def: TypeId) -> impl Iterator<Item = (FieldRef, CatFieldId)> + '_ {
        self.fields
            .iter()
            .skip_while(move |(f, _)| f.def() < def)
            .take_while(move |(f, _)| f.def() == def)
            .map(|(&f, &id)| (f, id))
    }

    pub fn relation_ids(&self) -> impl Iterator<Item = (RelId, CatRelId)> + '_ {
        self.relations.iter().map(|(&r, &id)| (r, id))
    }

    pub fn field_ids(&self) -> impl Iterator<Item = (FieldRef, CatFieldId)> + '_ {
        self.fields.iter().map(|(&f, &id)| (f, id))
    }

    pub fn set_ids(&self) -> impl Iterator<Item = (RelId, &str, CatSetId)> + '_ {
        self.sets.iter().map(|((r, n), &id)| (*r, n.as_str(), id))
    }

    pub fn key_ids(&self) -> impl Iterator<Item = (RelId, CatIndexId)> + '_ {
        self.keys.iter().map(|(&r, &id)| (r, id))
    }

    /// Returns each unique and index by relation and position among `RelationDef::indexes`.
    pub fn index_ids(&self) -> impl Iterator<Item = ((RelId, usize), CatIndexId)> + '_ {
        self.indexes.iter().map(|(&i, &id)| (i, id))
    }

    /// Returns each unique and index of `rel` by position among `RelationDef::indexes`.
    pub fn relation_index_ids(&self, rel: RelId) -> impl Iterator<Item = (usize, CatIndexId)> + '_ {
        self.indexes
            .range((rel, 0)..=(rel, usize::MAX))
            .map(|(&(_, at), &id)| (at, id))
    }

    pub fn implied_ids(&self) -> impl Iterator<Item = (RelId, FieldRef, CatIndexId)> + '_ {
        self.implied.iter().map(|(&(r, f), &id)| (r, f, id))
    }

    pub fn generated_ids(&self) -> impl Iterator<Item = (RelId, CatDefaultId)> + '_ {
        self.generated.iter().map(|(&r, &id)| (r, id))
    }

    /// Returns each default by relation, position among `RelationDef::defaults` and the field it
    /// fills.
    pub fn default_ids(
        &self,
    ) -> impl Iterator<Item = ((RelId, usize, FieldRef), CatDefaultId)> + '_ {
        self.defaults.iter().map(|(&d, &id)| (d, id))
    }

    pub fn computed_ids(&self) -> impl Iterator<Item = ((RelId, usize), CatComputedId)> + '_ {
        self.computed.iter().map(|(&c, &id)| (c, id))
    }

    pub fn function_ids(&self) -> impl Iterator<Item = (FnId, CatFunctionId)> + '_ {
        self.functions.iter().map(|(&f, &id)| (f, id))
    }

    pub fn aggregate_ids(&self) -> impl Iterator<Item = (usize, CatFunctionId)> + '_ {
        self.aggregates.iter().map(|(&i, &id)| (i, id))
    }

    pub fn constraint_ids(&self) -> impl Iterator<Item = (ConstraintIdx, CatConstraintId)> + '_ {
        self.constraints
            .iter()
            .map(|(&i, &id)| (ConstraintIdx::at(i), id))
    }

    pub fn check_ids(&self) -> impl Iterator<Item = (CheckIdx, CatConstraintId)> + '_ {
        self.checks.iter().map(|(&i, &id)| (CheckIdx::at(i), id))
    }

    pub fn view_field_ids(&self) -> impl Iterator<Item = (FnId, &str, CatFieldId)> + '_ {
        self.view_fields
            .iter()
            .map(|((f, n), &id)| (*f, n.as_str(), id))
    }

    pub fn view_ids(&self) -> impl Iterator<Item = (FnId, CatRelId)> + '_ {
        self.views.iter().map(|(&f, &id)| (f, id))
    }

    /// Give the object at `slot` the id whose stored form is `id`.
    ///
    /// Preconditions: no other object of `slot`'s space holds `id`.
    pub fn set(&mut self, slot: &Slot, id: u64) {
        self.forget(slot);
        if let Some(other) = self.held.insert((slot.space(), id), slot.clone()) {
            panic!(
                "no two objects of a schema share a catalog id: {other:?} and {slot:?} hold {id}"
            );
        }
        match slot.clone() {
            Slot::Type(t) => drop(self.types.insert(t, CatTypeId(id))),
            Slot::Ctor(c) => drop(self.ctors.insert(c, CatCtorId(id))),
            Slot::Relation(r) => drop(self.relations.insert(r, CatRelId(id))),
            Slot::Field(f) => drop(self.fields.insert(f, CatFieldId(id))),
            Slot::Set(r, s) => drop(self.sets.insert((r, s), CatSetId(id))),
            Slot::Function(f) => drop(self.functions.insert(f, CatFunctionId(id))),
            Slot::Key(r) => drop(self.keys.insert(r, CatIndexId(id))),
            Slot::Index(r, i) => drop(self.indexes.insert((r, i), CatIndexId(id))),
            Slot::Implied(r, f) => drop(self.implied.insert((r, f), CatIndexId(id))),
            Slot::Generated(r) => drop(self.generated.insert(r, CatDefaultId(id))),
            Slot::Default(r, i, f) => drop(self.defaults.insert((r, i, f), CatDefaultId(id))),
            Slot::Computed(r, i) => drop(self.computed.insert((r, i), CatComputedId(id))),
            Slot::Aggregate(i) => drop(self.aggregates.insert(i, CatFunctionId(id))),
            Slot::Constraint(i) => drop(self.constraints.insert(i, CatConstraintId(id))),
            Slot::Check(i) => drop(self.checks.insert(i, CatConstraintId(id))),
            Slot::View(f) => drop(self.views.insert(f, CatRelId(id))),
            Slot::ViewKey(f) => drop(self.view_keys.insert(f, CatIndexId(id))),
            Slot::ViewIndex(f, i) => drop(self.view_indexes.insert((f, i), CatIndexId(id))),
            Slot::ViewType(f) => drop(self.view_types.insert(f, CatTypeId(id))),
            Slot::ViewCtor(f) => drop(self.view_ctors.insert(f, CatCtorId(id))),
            Slot::ViewField(f, n) => drop(self.view_fields.insert((f, n), CatFieldId(id))),
        }
    }

    /// Remove the id of the object at `slot`.
    pub fn forget(&mut self, slot: &Slot) {
        let old = match slot.clone() {
            Slot::Type(t) => self.types.remove(&t).map(u64::from),
            Slot::Ctor(c) => self.ctors.remove(&c).map(u64::from),
            Slot::Relation(r) => self.relations.remove(&r).map(u64::from),
            Slot::Field(f) => self.fields.remove(&f).map(u64::from),
            Slot::Set(r, s) => self.sets.remove(&(r, s)).map(u64::from),
            Slot::Function(f) => self.functions.remove(&f).map(u64::from),
            Slot::Key(r) => self.keys.remove(&r).map(u64::from),
            Slot::Index(r, i) => self.indexes.remove(&(r, i)).map(u64::from),
            Slot::Implied(r, f) => self.implied.remove(&(r, f)).map(u64::from),
            Slot::Generated(r) => self.generated.remove(&r).map(u64::from),
            Slot::Default(r, i, f) => self.defaults.remove(&(r, i, f)).map(u64::from),
            Slot::Computed(r, i) => self.computed.remove(&(r, i)).map(u64::from),
            Slot::Aggregate(i) => self.aggregates.remove(&i).map(u64::from),
            Slot::Constraint(i) => self.constraints.remove(&i).map(u64::from),
            Slot::Check(i) => self.checks.remove(&i).map(u64::from),
            Slot::View(f) => self.views.remove(&f).map(u64::from),
            Slot::ViewKey(f) => self.view_keys.remove(&f).map(u64::from),
            Slot::ViewIndex(f, i) => self.view_indexes.remove(&(f, i)).map(u64::from),
            Slot::ViewType(f) => self.view_types.remove(&f).map(u64::from),
            Slot::ViewCtor(f) => self.view_ctors.remove(&f).map(u64::from),
            Slot::ViewField(f, n) => self.view_fields.remove(&(f, n)).map(u64::from),
        };
        if let Some(old) = old {
            self.held.remove(&(slot.space(), old));
        }
    }
}

/// The first id the log gives in every space.
pub const FIRST_LOG_ID: u64 = 1024;

/// Returns the ids of a schema built from `files`: the engine's fixed ones, and for the user modules
/// those one migration declaring every declaration of `files` in order, as `add` operators, gives.
pub fn declared(schema: &Schema, files: &[(FileId, ast::SchemaFile)]) -> CatalogIds {
    let mut ids = engine(schema);
    let mut next: BTreeMap<Space, u64> = BTreeMap::new();
    for o in objects(schema, files, &|_, _| String::new()) {
        let n = next.entry(o.space()).or_insert(FIRST_LOG_ID);
        ids.set(o.slot(), *n);
        *n += 1;
    }
    ids
}

fn engine(schema: &Schema) -> CatalogIds {
    let mut ids = CatalogIds::default();
    for (i, t) in schema.types.iter().enumerate() {
        let m = &schema.module(t.module).name;
        if !schema.module(t.module).engine {
            continue;
        }
        let Some((_, _, id, ctors)) = TYPES.iter().find(|(tm, tn, _, _)| tm == m && *tn == t.name)
        else {
            continue;
        };
        let def = TypeId(i as u32);
        ids.set(&Slot::Type(def), *id);
        let newtype = matches!(t.kind, TypeDefKind::Newtype { .. });
        for (c, cid) in *ctors {
            let at = if newtype {
                t.ctor_at(0)
            } else {
                t.ctor_named(c)
            };
            if let Some(at) = at {
                ids.set(&Slot::Ctor(at), *cid);
            }
        }
    }
    for (i, r) in schema.relations.iter().enumerate() {
        let m = &schema.module(r.module).name;
        if let Some((_, _, id)) = RELATIONS
            .iter()
            .find(|(rm, rn, _)| rm == m && *rn == r.name)
        {
            ids.set(&Slot::Relation(RelId(i as u32)), *id);
        }
    }
    let mut types: Vec<(CatTypeId, TypeId)> = ids.type_ids().map(|(t, id)| (id, t)).collect();
    types.sort();
    let mut field = 0;
    let mut added = Vec::new();
    for (_, def) in types {
        let td = schema.ty(def);
        for (f, _) in td.field_refs() {
            let ctor = f.ctor().and_then(|c| td.ctor_at(c)).map(|c| {
                ids.ctor_id(c)
                    .expect("every constructor of a fixed engine type has a fixed id")
            });
            // Constructors from 471 extend the original engine field-id allocation.
            if let Some(ctor) = ctor.filter(|ctor| ctor.raw() >= 471) {
                added.push((ctor, Slot::Field(f)));
                continue;
            }
            ids.set(&Slot::Field(f), field);
            field += 1;
        }
    }
    added.sort_by_key(|(ctor, _)| *ctor);
    for (_, slot) in added {
        ids.set(&slot, field);
        field += 1;
    }
    let mut relations: Vec<(CatRelId, RelId)> = ids.relation_ids().map(|(r, id)| (id, r)).collect();
    relations.sort();
    let mut index = 0;
    for (_, r) in relations {
        ids.set(&Slot::Key(r), index);
        index += 1;
        for i in 0..schema.relation(r).indexes.len() {
            ids.set(&Slot::Index(r, i), index);
            index += 1;
        }
    }
    ids
}
