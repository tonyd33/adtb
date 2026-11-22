/// A constraint, or a check when `check`, that a migration keeps although its operators drop it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    pub module: String,
    pub name: String,
    pub check: bool,
}
