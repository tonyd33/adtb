//! Assign and hold the catalog ids of a schema's objects.

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{self, Decl, RelationClause};
use crate::schema::{CtorRef, FieldRef, Schema, TypeDefKind, ValueDef};
use crate::span::{FileId, Span};
use crate::types::*;

#[cfg(test)]
mod tests;

mod fixed;
pub use fixed::{FixedType, RELATIONS, TYPES};

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
    type_of: Reverse<CatTypeId, TypeId>,
    ctor_of: Reverse<CatCtorId, CtorRef>,
    relation_of: Reverse<CatRelId, RelId>,
    field_of: Reverse<CatFieldId, FieldRef>,
    set_of: Reverse<CatSetId, (RelId, String)>,
    function_of: Reverse<CatFunctionId, FnId>,
    aggregate_of: Reverse<CatFunctionId, usize>,
    index_of: Reverse<CatIndexId, (RelId, usize)>,
    constraint_of: Reverse<CatConstraintId, usize>,
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
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// One object of a user module, in the order a migration declaring the module files in order creates it.
#[derive(Clone, Debug)]
pub struct Object {
    space: Space,
    slot: Slot,
    /// The object's name, stable while no operator renames or rewrites it.
    path: String,
}

impl Object {
    pub fn space(&self) -> Space {
        self.space
    }

    pub fn slot(&self) -> &Slot {
        &self.slot
    }

    pub fn path(&self) -> &str {
        &self.path
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

/// The objects that hold each id of one map of [`CatalogIds`].
type Reverse<I, K> = BTreeMap<I, BTreeSet<K>>;

/// Insert `id` at `key` of `forward` and keep `reverse` its inverse, forgetting the id `key` had.
fn put<K: Ord + Clone, I: Ord + Copy>(
    forward: &mut BTreeMap<K, I>,
    reverse: &mut Reverse<I, K>,
    key: K,
    id: I,
) {
    take(forward, reverse, &key);
    forward.insert(key.clone(), id);
    reverse.entry(id).or_default().insert(key);
}

/// Remove `key` from `forward` and from the holders of its id in `reverse`.
fn take<K: Ord, I: Ord>(forward: &mut BTreeMap<K, I>, reverse: &mut Reverse<I, K>, key: &K) {
    let Some(old) = forward.remove(key) else {
        return;
    };
    if let Some(keys) = reverse.get_mut(&old) {
        keys.remove(key);
        if keys.is_empty() {
            reverse.remove(&old);
        }
    }
}

/// Returns the objects holding `id`, least first.
///
/// Several objects hold one id only in a schema built while the log has no id for some object,
/// whose id then comes from declaration order.
fn holders<I: Ord, K: Ord>(reverse: &Reverse<I, K>, id: I) -> impl DoubleEndedIterator<Item = &K> {
    reverse.get(&id).into_iter().flatten()
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

    /// Returns the id of the constraint at position `at` of `Schema::constraints`.
    pub fn constraint_id(&self, at: usize) -> Option<CatConstraintId> {
        self.constraints.get(&at).copied()
    }

    /// Returns the id of the check at position `at` of `Schema::checks`.
    pub fn check_id(&self, at: usize) -> Option<CatConstraintId> {
        self.checks.get(&at).copied()
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

    /// Returns the types whose id is `id`, least first.
    pub fn type_of(&self, id: CatTypeId) -> impl DoubleEndedIterator<Item = TypeId> + '_ {
        holders(&self.type_of, id).copied()
    }

    /// Returns the constructors whose id is `id`, least first.
    pub fn ctor_of(&self, id: CatCtorId) -> impl DoubleEndedIterator<Item = CtorRef> + '_ {
        holders(&self.ctor_of, id).copied()
    }

    /// Returns the relation variables whose id is `id`, least first.
    pub fn relation_of(&self, id: CatRelId) -> impl DoubleEndedIterator<Item = RelId> + '_ {
        holders(&self.relation_of, id).copied()
    }

    /// Returns the fields whose id is `id`, least first.
    pub(crate) fn field_of(
        &self,
        id: CatFieldId,
    ) -> impl DoubleEndedIterator<Item = FieldRef> + '_ {
        holders(&self.field_of, id).copied()
    }

    /// Returns the named sets, as relation and name, whose id is `id`, least first.
    pub fn set_of(&self, id: CatSetId) -> impl DoubleEndedIterator<Item = (RelId, &str)> + '_ {
        holders(&self.set_of, id).map(|(r, n)| (*r, n.as_str()))
    }

    /// Returns the functions whose id is `id`, least first.
    pub fn function_of(&self, id: CatFunctionId) -> impl DoubleEndedIterator<Item = FnId> + '_ {
        holders(&self.function_of, id).copied()
    }

    /// Returns the uniques and indexes, as relation and position among `RelationDef::indexes`, whose id is `id`, least first.
    pub fn index_of(&self, id: CatIndexId) -> impl DoubleEndedIterator<Item = (RelId, usize)> + '_ {
        holders(&self.index_of, id).copied()
    }

    /// Returns the constraints, as position among `Schema::constraints`, whose id is `id`, least first.
    pub fn constraint_of(
        &self,
        id: CatConstraintId,
    ) -> impl DoubleEndedIterator<Item = usize> + '_ {
        holders(&self.constraint_of, id).copied()
    }

    /// Returns the aggregates, as position among `Schema::aggregates`, whose id is `id`, least first.
    pub fn aggregate_of(&self, id: CatFunctionId) -> impl DoubleEndedIterator<Item = usize> + '_ {
        holders(&self.aggregate_of, id).copied()
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

    pub fn constraint_ids(&self) -> impl Iterator<Item = (usize, CatConstraintId)> + '_ {
        self.constraints.iter().map(|(&i, &id)| (i, id))
    }

    pub fn check_ids(&self) -> impl Iterator<Item = (usize, CatConstraintId)> + '_ {
        self.checks.iter().map(|(&i, &id)| (i, id))
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
    pub fn set(&mut self, slot: &Slot, id: u64) {
        match slot.clone() {
            Slot::Type(t) => put(&mut self.types, &mut self.type_of, t, CatTypeId(id)),
            Slot::Ctor(c) => put(&mut self.ctors, &mut self.ctor_of, c, CatCtorId(id)),
            Slot::Relation(r) => put(&mut self.relations, &mut self.relation_of, r, CatRelId(id)),
            Slot::Field(f) => put(&mut self.fields, &mut self.field_of, f, CatFieldId(id)),
            Slot::Set(r, s) => put(&mut self.sets, &mut self.set_of, (r, s), CatSetId(id)),
            Slot::Function(f) => {
                put(
                    &mut self.functions,
                    &mut self.function_of,
                    f,
                    CatFunctionId(id),
                );
            }
            Slot::Key(r) => {
                self.keys.insert(r, CatIndexId(id));
            }
            Slot::Index(r, i) => put(
                &mut self.indexes,
                &mut self.index_of,
                (r, i),
                CatIndexId(id),
            ),
            Slot::Implied(r, f) => {
                self.implied.insert((r, f), CatIndexId(id));
            }
            Slot::Generated(r) => {
                self.generated.insert(r, CatDefaultId(id));
            }
            Slot::Default(r, i, f) => {
                self.defaults.insert((r, i, f), CatDefaultId(id));
            }
            Slot::Computed(r, i) => {
                self.computed.insert((r, i), CatComputedId(id));
            }
            Slot::Aggregate(i) => {
                put(
                    &mut self.aggregates,
                    &mut self.aggregate_of,
                    i,
                    CatFunctionId(id),
                );
            }
            Slot::Constraint(i) => {
                put(
                    &mut self.constraints,
                    &mut self.constraint_of,
                    i,
                    CatConstraintId(id),
                );
            }
            Slot::Check(i) => {
                self.checks.insert(i, CatConstraintId(id));
            }
            Slot::View(f) => {
                self.views.insert(f, CatRelId(id));
            }
            Slot::ViewKey(f) => {
                self.view_keys.insert(f, CatIndexId(id));
            }
            Slot::ViewIndex(f, i) => {
                self.view_indexes.insert((f, i), CatIndexId(id));
            }
            Slot::ViewType(f) => {
                self.view_types.insert(f, CatTypeId(id));
            }
            Slot::ViewCtor(f) => {
                self.view_ctors.insert(f, CatCtorId(id));
            }
            Slot::ViewField(f, n) => {
                self.view_fields.insert((f, n), CatFieldId(id));
            }
        }
    }

    /// Remove the id of the object at `slot`.
    pub fn forget(&mut self, slot: &Slot) {
        match slot.clone() {
            Slot::Type(t) => take(&mut self.types, &mut self.type_of, &t),
            Slot::Ctor(c) => take(&mut self.ctors, &mut self.ctor_of, &c),
            Slot::Relation(r) => take(&mut self.relations, &mut self.relation_of, &r),
            Slot::Field(f) => take(&mut self.fields, &mut self.field_of, &f),
            Slot::Set(r, s) => take(&mut self.sets, &mut self.set_of, &(r, s)),
            Slot::Function(f) => take(&mut self.functions, &mut self.function_of, &f),
            Slot::Key(r) => drop(self.keys.remove(&r)),
            Slot::Index(r, i) => take(&mut self.indexes, &mut self.index_of, &(r, i)),
            Slot::Implied(r, f) => drop(self.implied.remove(&(r, f))),
            Slot::Generated(r) => drop(self.generated.remove(&r)),
            Slot::Default(r, i, f) => drop(self.defaults.remove(&(r, i, f))),
            Slot::Computed(r, i) => drop(self.computed.remove(&(r, i))),
            Slot::Aggregate(i) => take(&mut self.aggregates, &mut self.aggregate_of, &i),
            Slot::Constraint(i) => take(&mut self.constraints, &mut self.constraint_of, &i),
            Slot::Check(i) => drop(self.checks.remove(&i)),
            Slot::View(f) => drop(self.views.remove(&f)),
            Slot::ViewKey(f) => drop(self.view_keys.remove(&f)),
            Slot::ViewIndex(f, i) => drop(self.view_indexes.remove(&(f, i))),
            Slot::ViewType(f) => drop(self.view_types.remove(&f)),
            Slot::ViewCtor(f) => drop(self.view_ctors.remove(&f)),
            Slot::ViewField(f, n) => drop(self.view_fields.remove(&(f, n))),
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

/// Every object the user modules of `files` declare, in creation order, `text` giving the source
/// text of a span of a file for the paths of uniques and indexes.
pub fn objects(
    schema: &Schema,
    files: &[(FileId, ast::SchemaFile)],
    text: &dyn Fn(FileId, Span) -> String,
) -> Vec<Object> {
    let mut out = Vec::new();
    let mut push = |space: Space, slot: Slot, path: String| out.push(Object { space, slot, path });
    for (file, sf) in files {
        if file.is_engine() {
            continue;
        }
        let Some(mid) = schema.module_named(&sf.module.text) else {
            continue;
        };
        let m = &sf.module.text;
        let values = &schema.module(mid).values;
        for d in &sf.decls {
            match d {
                Decl::Type(t) => type_objects(&mut push, schema, mid, m, t),
                Decl::Relation(r) => {
                    if let Some(&ValueDef::Relation(rel)) = values.get(&r.name.name.text) {
                        relation_objects(&mut push, schema, *file, text, m, rel, r);
                    }
                }
                Decl::Set(s) => {
                    if let Some(&ValueDef::Relation(rel)) = values.get(&s.relation.text) {
                        push(
                            Space::Set,
                            Slot::Set(rel, s.name.text.clone()),
                            format!("set {m}::{}.{}", s.relation.text, s.name.text),
                        );
                    }
                }
                Decl::Fn(f) => {
                    if let Some(&ValueDef::Function(id)) = values.get(&f.name.text) {
                        push(
                            Space::Function,
                            Slot::Function(id),
                            format!("fn {m}::{}", f.name.text),
                        );
                    }
                }
                Decl::Aggregate(a) => {
                    if let Some(&ValueDef::Aggregate(i)) = values.get(&a.name.text) {
                        push(
                            Space::Function,
                            Slot::Aggregate(i as usize),
                            format!("fn {m}::{}", a.name.text),
                        );
                    }
                }
                Decl::Constraint(c) => {
                    if let Some(&ValueDef::Constraint(i)) = values.get(&c.name.text) {
                        push(
                            Space::Constraint,
                            Slot::Constraint(i as usize),
                            format!("constraint {m}::{}", c.name.text),
                        );
                    }
                }
                Decl::Check(c) => {
                    if let Some(&ValueDef::Check(i)) = values.get(&c.name.text) {
                        push(
                            Space::Constraint,
                            Slot::Check(i as usize),
                            format!("constraint {m}::{}", c.name.text),
                        );
                    }
                }
                Decl::Store(s) => {
                    let Some(&ValueDef::Function(f)) = values.get(&s.view.name.text) else {
                        continue;
                    };
                    let vn = &s.view.name.text;
                    push(Space::Shared, Slot::View(f), format!("relation {m}::{vn}"));
                    if matches!(
                        &schema.functions[f.0 as usize].result,
                        Ty::Set(elem) if !matches!(**elem, Ty::Data { .. })
                    ) {
                        let fields = anonymous_row(files, m, vn);
                        push(Space::Shared, Slot::ViewType(f), format!("type {m}::{vn}"));
                        push(
                            Space::Shared,
                            Slot::ViewCtor(f),
                            format!("ctor {m}::{vn}.{vn}"),
                        );
                        for name in fields {
                            push(
                                Space::Field,
                                Slot::ViewField(f, name.clone()),
                                format!("field {m}::{vn}.{vn}.{name}"),
                            );
                        }
                    }
                    let name = format!("{m}::{vn}");
                    clause_objects(
                        &mut push,
                        schema,
                        *file,
                        text,
                        &s.clauses,
                        &name,
                        Owner::View(f),
                    );
                }
            }
        }
    }
    out
}

fn type_objects(
    push: &mut dyn FnMut(Space, Slot, String),
    schema: &Schema,
    mid: ModuleId,
    m: &str,
    t: &ast::TypeDecl,
) {
    let def = if t.name.text == "Principal" {
        Some(schema.principal)
    } else {
        schema.module(mid).types.get(&t.name.text).copied()
    };
    let Some(def) = def else { return };
    let td = schema.ty(def);
    let tn = &td.name;
    push(Space::Shared, Slot::Type(def), format!("type {m}::{tn}"));
    let newtype = matches!(td.kind, TypeDefKind::Newtype { .. });
    for c in td.ctor_refs() {
        let cn = if newtype { tn } else { &td.ctor_def(c).name };
        push(Space::Shared, Slot::Ctor(c), format!("ctor {m}::{tn}.{cn}"));
    }
    for (f, d) in td.field_refs() {
        let at = f.ctor().map_or(String::new(), |c| {
            format!("{}.", td.ctors()[c as usize].name)
        });
        push(
            Space::Field,
            Slot::Field(f),
            format!("field {m}::{tn}.{at}{}", d.name),
        );
    }
}

fn relation_objects(
    push: &mut dyn FnMut(Space, Slot, String),
    schema: &Schema,
    file: FileId,
    text: &dyn Fn(FileId, Span) -> String,
    m: &str,
    rel: RelId,
    r: &ast::RelationDecl,
) {
    let rn = &r.name.name.text;
    push(
        Space::Shared,
        Slot::Relation(rel),
        format!("relation {m}::{rn}"),
    );
    let name = format!("{m}::{rn}");
    clause_objects(
        push,
        schema,
        file,
        text,
        &r.clauses,
        &name,
        Owner::Relation(rel),
    );
    let def = schema.ty(schema.relation(rel).elem_def);
    for i in schema.implied_indexes(rel) {
        let at = i.ctor.map_or(String::new(), |c| {
            format!("{}.", def.ctors()[c as usize].name)
        });
        push(
            Space::Index,
            Slot::Implied(rel, i.field_ref()),
            format!("index {name} implied by {at}{}", i.field),
        );
    }
}

/// Returns the fields of the rows of view `view` of module `module`, as its result type writes
/// them: none unless its element type is an anonymous record.
fn anonymous_row(files: &[(FileId, ast::SchemaFile)], module: &str, view: &str) -> Vec<String> {
    let result = files
        .iter()
        .filter(|(_, sf)| sf.module.text == module)
        .flat_map(|(_, sf)| &sf.decls)
        .find_map(|d| match d {
            Decl::Fn(f) if f.name.text == view => Some(&f.result),
            _ => None,
        });
    let Some(ast::Type {
        kind: ast::TypeKind::Named { args, .. },
        ..
    }) = result
    else {
        return Vec::new();
    };
    let [ast::TypeArg::Type(elem)] = args.as_slice() else {
        return Vec::new();
    };
    match &elem.kind {
        ast::TypeKind::Record(fs) => fs.iter().map(|f| f.name.text.clone()).collect(),
        _ => Vec::new(),
    }
}

/// Returns the fields of relation `r` a default of `field`, of constructor `ctor` when one is
/// named, fills: the header's field, else the field of each constructor with it in declaration
/// order.
fn default_fields(
    schema: &Schema,
    r: RelId,
    ctor: Option<&ast::Ident>,
    field: &str,
) -> Vec<FieldRef> {
    let def = schema.relation(r).elem_def;
    if def.0 == u32::MAX {
        return Vec::new();
    }
    let td = schema.ty(def);
    if let Some(f) = td.field(None, field) {
        return vec![f];
    }
    td.ctor_refs()
        .filter(|&c| ctor.is_none_or(|n| n.text == td.ctor_def(c).name))
        .filter_map(|c| td.field(Some(c), field))
        .collect()
}

/// What declares a run of clauses.
#[derive(Clone, Copy)]
enum Owner {
    Relation(RelId),
    View(FnId),
}

/// The objects a relation's or a stored view's clauses create, in written order.
fn clause_objects(
    push: &mut dyn FnMut(Space, Slot, String),
    schema: &Schema,
    file: FileId,
    text: &dyn Fn(FileId, Span) -> String,
    clauses: &[RelationClause],
    name: &str,
    owner: Owner,
) {
    let (mut ix, mut dx, mut cx) = (0, 0, 0);
    for c in clauses {
        match (c, owner) {
            (RelationClause::Key(_, generate), _) => {
                let key = match owner {
                    Owner::Relation(r) => Slot::Key(r),
                    Owner::View(f) => Slot::ViewKey(f),
                };
                push(Space::Index, key, format!("index {name} key"));
                if let (Some(_), Owner::Relation(r)) = (generate, owner) {
                    push(
                        Space::Default,
                        Slot::Generated(r),
                        format!("default {name} key"),
                    );
                }
            }
            (
                RelationClause::Unique(fs, scope)
                | RelationClause::Index {
                    fields: fs, scope, ..
                },
                _,
            ) => {
                let kind = match c {
                    RelationClause::Unique(..) => "unique",
                    RelationClause::Index { interval: true, .. } => "interval index",
                    _ => "index",
                };
                let fields: Vec<String> = fs.iter().map(|e| text(file, e.span)).collect();
                let scope = match scope {
                    None => "*".to_string(),
                    Some(ast::CtorScope::One(c)) => c.text.clone(),
                    Some(ast::CtorScope::Many(cs)) => cs
                        .iter()
                        .map(|c| c.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" | "),
                };
                let slot = match owner {
                    Owner::Relation(r) => Slot::Index(r, ix),
                    Owner::View(f) => Slot::ViewIndex(f, ix),
                };
                push(
                    Space::Index,
                    slot,
                    format!("index {name} {kind} ({}) in {scope}", fields.join(", ")),
                );
                ix += 1;
            }
            (RelationClause::Default { ctor, field, .. }, Owner::Relation(r)) => {
                for f in default_fields(schema, r, ctor.as_ref(), &field.text) {
                    let at = f.ctor().map_or(String::new(), |k| {
                        let td = schema.ty(schema.relation(r).elem_def);
                        format!("{}.", td.ctors()[k as usize].name)
                    });
                    push(
                        Space::Default,
                        Slot::Default(r, dx, f),
                        format!("default {name} {at}{}", field.text),
                    );
                }
                dx += 1;
            }
            (RelationClause::Computed(n, _, _), Owner::Relation(r)) => {
                push(
                    Space::Computed,
                    Slot::Computed(r, cx),
                    format!("computed {name}.{}", n.text),
                );
                cx += 1;
            }
            _ => {}
        }
    }
}
