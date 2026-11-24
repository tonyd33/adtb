use crate::ids::CatConstraintId;

/// A constraint or check, by its catalog id, that a migration keeps although its operators drop
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kept(pub CatConstraintId);
