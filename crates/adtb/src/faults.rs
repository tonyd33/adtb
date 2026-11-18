//! Failures a test injects at a step, by the place in the engine that runs it.

#[cfg(not(test))]
use crate::storage::PageError;

/// A step that may fail before or after the main update of its change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Step {
    /// Count one more version pointing at large values, before the update that writes it.
    Retain,
    /// Clear the map bit of a heap page a change wrote.
    ClearBit,
    /// Set the directory entry of the heap page that took a version.
    Reflect,
    /// Write a change's staged heap pages in one update.
    Write,
    /// Free every page a schema change left unreached.
    FreeLeaked,
    /// End the change in progress the catalog root records.
    EndChange,
    /// Settle the directory entries above a changed one.
    Settle,
    /// Count one version fewer pointing at a large value.
    Release,
}

/// Returns whether a test failed this run of `step` at its caller's place.
#[cfg(not(test))]
#[track_caller]
pub fn struck(_: Step) -> bool {
    false
}

/// Returns the failure a test gave this run of `step` at its caller's place.
#[cfg(not(test))]
#[track_caller]
pub fn injected(_: Step) -> Option<PageError> {
    None
}

#[cfg(test)]
pub use armed::{injected, struck};

#[cfg(test)]
#[path = "faults_tests.rs"]
pub mod armed;
