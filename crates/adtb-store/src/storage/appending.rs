//! The witness that the log and meta locks are held together.

use super::holding::Holding;
use super::pager::{Appended, Meta};

/// The witness of an append: the log and meta locks held together.
pub(super) struct Appending(());

impl Appending {
    /// Returns the witness of `log` and `meta` held together.
    pub(super) fn of(_log: &Holding<'_, Appended>, _meta: &Holding<'_, Meta>) -> Appending {
        Appending(())
    }
}

#[cfg(test)]
pub(super) fn appending_for_test() -> Appending {
    Appending(())
}
