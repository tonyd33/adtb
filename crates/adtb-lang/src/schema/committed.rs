use std::ops::Deref;

use super::Schema;
use crate::types::{RelId, RelRef};

/// A schema no migration is being applied to: it takes no relation as added later.
#[derive(Clone, Debug)]
pub struct Committed(Schema);

impl Committed {
    /// Returns `schema` when no migration is being applied to it: it takes no relation as added
    /// later and resolves no selection when one ends.
    pub fn new(schema: Schema) -> Option<Committed> {
        (schema.ahead.is_empty() && schema.late_sels.is_empty()).then_some(Committed(schema))
    }

    /// Returns `schema`, built with no relation taken as added later.
    pub(super) fn built(schema: Schema) -> Committed {
        Committed(schema)
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

impl Deref for Committed {
    type Target = Schema;

    fn deref(&self) -> &Schema {
        &self.0
    }
}
