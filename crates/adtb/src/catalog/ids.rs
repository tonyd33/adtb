use adtb_lang::schema::Schema;
use adtb_lang::types::{RelId, TypeId};

use crate::key::CtorId;

macro_rules! catalog_id {
    ($(#[$doc:meta] $name:ident),*) => {$(
        #[$doc]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u64);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    )*};
}

catalog_id!(
    /// The catalog id of a relation variable.
    CatRelId,
    /// The catalog id of a constructor.
    CatCtorId,
    /// The catalog id of a field.
    CatFieldId,
    /// The catalog id of a type.
    CatTypeId,
    /// The catalog id of a constraint or a check.
    CatConstraintId,
    /// The catalog id of a unique or an index.
    CatIndexId,
    /// The catalog id of a named constructor set.
    CatSetId
);

impl CatCtorId {
    /// Returns the id heaps and keys store the constructor under.
    ///
    /// Panics if the id does not fit 32 bits.
    pub fn stored(self) -> CtorId {
        CtorId(u32::try_from(self.0).expect("a constructor id fits 32 bits"))
    }
}

/// Returns the catalog id of relation `r` of `schema`.
pub fn rel_id(schema: &Schema, r: RelId) -> CatRelId {
    CatRelId(schema.ids.relations[&r])
}

/// Returns the catalog id of constructor `c` of relation `r` of `schema`.
pub fn ctor_id(schema: &Schema, r: RelId, c: u32) -> CatCtorId {
    CatCtorId(schema.ids.ctors[&(schema.relation(r).elem_def, c)])
}

pub fn type_id(schema: &Schema, def: TypeId) -> Option<CatTypeId> {
    schema.ids.types.get(&def).copied().map(CatTypeId)
}

/// Returns the catalog id of field `name` of constructor `c` of type `def`, a header field's
/// when the header has one by that name.
pub fn field_id(schema: &Schema, def: TypeId, c: u32, name: &str) -> Option<CatFieldId> {
    let fields = &schema.ids.fields;
    fields
        .get(&(def, None, name.to_string()))
        .or_else(|| fields.get(&(def, Some(c), name.to_string())))
        .copied()
        .map(CatFieldId)
}

/// Returns the relation of `schema` whose catalog id is `id`.
pub fn relation(schema: &Schema, id: CatRelId) -> Option<RelId> {
    schema.ids.relation(id.0)
}

/// Returns the type of `schema` whose catalog id is `id`.
pub fn type_def(schema: &Schema, id: CatTypeId) -> Option<TypeId> {
    schema
        .ids
        .types
        .iter()
        .find(|&(_, &x)| x == id.0)
        .map(|(&d, _)| d)
}

/// Returns the position of the constructor with catalog id `id` of relation `r`.
pub fn ctor_index(schema: &Schema, r: RelId, id: CatCtorId) -> Option<u32> {
    let def = schema.relation(r).elem_def;
    schema
        .ids
        .ctors
        .iter()
        .find(|&(&(d, _), &x)| d == def && x == id.0)
        .map(|(&(_, k), _)| k)
}
