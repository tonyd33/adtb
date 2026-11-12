pub use adtb_lang::ids::held_id;
pub use adtb_lang::ids::{
    CatConstraintId, CatCtorId, CatFieldId, CatIndexId, CatRelId, CatSetId, CatTypeId,
};
use adtb_lang::schema::{CtorRef, Schema};
use adtb_lang::types::{RelId, TypeId};

use crate::key::CtorId;

/// Returns the id heaps and keys store constructor `id` under.
///
/// Preconditions: `id` fits 32 bits.
pub fn stored(id: CatCtorId) -> CtorId {
    CtorId(u32::try_from(id.raw()).expect("a constructor id fits 32 bits"))
}

/// Returns the catalog id of relation `r` of `schema`.
pub fn rel_id(schema: &Schema, r: RelId) -> CatRelId {
    held_id(schema.ids.relation_id(r))
}

/// Returns the catalog id of constructor `c` of relation `r` of `schema`.
pub fn ctor_id(schema: &Schema, r: RelId, c: u32) -> CatCtorId {
    type_ctor_id(schema, schema.relation(r).elem_def, c)
}

/// Returns the catalog id of constructor `c` of type `def` of `schema`.
pub fn type_ctor_id(schema: &Schema, def: TypeId, c: u32) -> CatCtorId {
    held_id(
        schema
            .ty(def)
            .ctor_at(c)
            .and_then(|c| schema.ids.ctor_id(c)),
    )
}

pub fn type_id(schema: &Schema, def: TypeId) -> Option<CatTypeId> {
    schema.ids.type_id(def)
}

/// Returns the catalog id of field `name` of constructor `c` of type `def`, a header field's
/// when the header has one by that name.
pub fn field_id(schema: &Schema, def: TypeId, c: u32, name: &str) -> Option<CatFieldId> {
    let td = schema.ty(def);
    schema.ids.field_id(td.field(td.ctor_at(c), name)?)
}

/// Returns the relation of `schema` whose catalog id is `id`.
pub fn relation(schema: &Schema, id: CatRelId) -> Option<RelId> {
    schema.ids.relation_of(id)
}

/// Returns the type of `schema` whose catalog id is `id`.
pub fn type_def(schema: &Schema, id: CatTypeId) -> Option<TypeId> {
    schema.ids.type_of(id)
}

/// Returns the position of the constructor with catalog id `id` of relation `r`.
pub fn ctor_index(schema: &Schema, r: RelId, id: CatCtorId) -> Option<u32> {
    type_ctor_index(schema, schema.relation(r).elem_def, id)
}

/// Returns the position of the constructor with catalog id `id` of type `def`.
pub fn type_ctor_index(schema: &Schema, def: TypeId, id: CatCtorId) -> Option<u32> {
    schema
        .ids
        .ctor_of(id)
        .filter(|c| c.def() == def)
        .map(CtorRef::index)
}
