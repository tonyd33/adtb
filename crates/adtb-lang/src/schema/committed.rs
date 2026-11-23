use std::ops::Deref;

use super::{Schema, Temp};
use crate::types::{ModuleId, RelId, RelRef};

/// A schema no migration is being applied to, whose objects outside the temp module have catalog
/// ids.
#[derive(Clone, Debug)]
pub struct Committed(Schema);

impl Committed {
    /// Returns `schema` when no migration is being applied to it and it identifies its objects:
    /// it takes no relation as added later, resolves no selection when one ends, and gives every
    /// type, constructor, field, relation variable, relation key and index outside the temp module
    /// a catalog id.
    pub fn new(schema: Schema) -> Option<Committed> {
        (schema.ahead.is_empty() && schema.late_sels.is_empty() && identified(&schema))
            .then_some(Committed(schema))
    }

    /// Returns the schema with each of `temps` appended to the relations of module
    /// [`crate::check::TEMP_MODULE`], an anonymous record row declared as a record type of that
    /// module; returns the relation of each.
    pub(crate) fn with_temps(&self, temps: &[Temp]) -> (Committed, Vec<RelId>) {
        let (schema, rels) = self.0.with_temps(temps);
        (Committed(schema), rels)
    }

    /// Returns the declared relation `rel` names.
    ///
    /// Preconditions: `rel` is of this schema.
    pub fn declared(&self, rel: RelRef) -> RelId {
        match rel {
            RelRef::Declared(id) => id,
            RelRef::Ahead(a) => panic!(
                "a key of a committed schema names one of its relations: {a:?} is of another schema"
            ),
        }
    }
}

/// Returns whether every type, constructor, field, relation variable, relation key and index
/// of a module of `schema` other than the temp module has a catalog id.
fn identified(schema: &Schema) -> bool {
    let ids = schema.ids();
    let held = |m: ModuleId| schema.module(m).name != crate::check::TEMP_MODULE;
    schema.types.iter().filter(|td| held(td.module)).all(|td| {
        ids.type_id(td.id()).is_some()
            && td.ctor_refs().all(|c| ids.ctor_id(c).is_some())
            && td.field_refs().all(|(f, _)| ids.field_id(f).is_some())
    }) && (0..schema.relations.len() as u32)
        .map(RelId)
        .filter(|&r| held(schema.relation(r).module))
        .all(|r| {
            ids.relation_id(r).is_some()
                && ids.key_id(r).is_some()
                && (0..schema.relation(r).indexes.len()).all(|at| ids.index_id(r, at).is_some())
        })
}

impl Deref for Committed {
    type Target = Schema;

    fn deref(&self) -> &Schema {
        &self.0
    }
}

impl Schema {
    /// Returns the first type, constructor, field, relation variable, function, aggregate,
    /// constraint or check this schema identifies that `now` does not identify at the same
    /// position by the same id.
    pub(crate) fn first_moved(&self, now: &Schema) -> Option<crate::ids::Placed> {
        self.ids().first_moved(now.ids())
    }
}
