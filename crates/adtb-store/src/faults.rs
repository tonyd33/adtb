//! Failures a test injects at a step, by the place in the engine that runs it.

#[cfg(not(any(test, feature = "testing")))]
use crate::runtime::Runtime;
#[cfg(not(any(test, feature = "testing")))]
use crate::storage::{PageError, Pager, Pid};

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
    /// Read a page a schema change left unreached, to learn its owner.
    ReadLeaked,
    /// End the change in progress the catalog root records.
    EndChange,
    /// Settle the directory entries above a changed one.
    Settle,
    /// Count one version fewer pointing at a large value.
    Release,
    /// Run a step of an open after the migrations it applied committed.
    Opening,
}

impl Step {
    /// Every step, in the order of their declaration.
    pub const EVERY: [Step; 10] = [
        Step::Retain,
        Step::ClearBit,
        Step::Reflect,
        Step::Write,
        Step::FreeLeaked,
        Step::ReadLeaked,
        Step::EndChange,
        Step::Settle,
        Step::Release,
        Step::Opening,
    ];
}

/// Returns whether a test failed this run of `step` at its caller's place.
#[cfg(not(any(test, feature = "testing")))]
#[track_caller]
pub fn struck(_: Step) -> bool {
    false
}

/// Returns the failure a test gave this run of `step` at its caller's place.
#[cfg(not(any(test, feature = "testing")))]
#[track_caller]
pub fn injected(_: Step) -> Option<PageError> {
    None
}

/// Returns the damage a test gave `pid` of `pager` at this run of `step` at its caller's place.
#[cfg(not(any(test, feature = "testing")))]
#[track_caller]
pub fn damaged<R: Runtime>(_: Step, _: &Pager<R>, _: Pid) -> Option<PageError> {
    None
}

#[cfg(any(test, feature = "testing"))]
pub use armed::{damaged, injected, struck};

#[cfg(any(test, feature = "testing"))]
#[path = "faults_tests.rs"]
pub mod armed;
