//! Points in the engine's work at which a test can stop the thread that reaches them.

/// A point in the engine's work that a test's step hook observes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Step {
    /// A serializable read of a version through its TID read it and has yet to lock it.
    VersionRead,
    /// A serializable scan of a heap that did not stop early read its pages and has yet to end
    /// its scan.
    ScanRead,
    /// A serializable scan of a heap that stopped early read its pages and has yet to end its
    /// scan.
    ScanStopped,
    /// A statement's update, delete or retag of a version changed its page.
    WriteChanged(Change),
    /// A statement's update, delete or retag of a version checked the SIREAD locks on it.
    WriteChecked,
    /// A statement's insert of a version changed its page.
    WriteInserted,
    /// A failed statement's take-back is to undo its next change.
    TakeBack,
}

/// What a statement's write did to a version it superseded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Delete,
    Update {
        /// The successor is on the version's page and has no index entries.
        heap_only: bool,
    },
    Retag,
}

/// Called with each step on the thread that reaches it, holding no latch or engine lock.
#[cfg(any(test, feature = "testing"))]
pub type StepHook = std::sync::Arc<dyn Fn(Step) + Send + Sync>;

/// The step hook a test set.
#[derive(Default)]
pub struct Steps {
    #[cfg(any(test, feature = "testing"))]
    hook: std::sync::Mutex<Option<StepHook>>,
}

impl std::fmt::Debug for Steps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Steps")
    }
}

#[cfg(any(test, feature = "testing"))]
impl Steps {
    /// Call the hook with `step`.
    ///
    /// Precondition: the calling thread holds no latch or engine lock.
    pub(crate) fn at(&self, step: Step) {
        let hook = self
            .hook
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(hook) = hook {
            hook(step);
        }
    }

    pub(crate) fn set(&self, hook: StepHook) {
        *self
            .hook
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(hook);
    }
}

#[cfg(not(any(test, feature = "testing")))]
impl Steps {
    pub(crate) fn at(&self, _: Step) {}
}
