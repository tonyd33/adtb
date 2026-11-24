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
    /// The damage mapper, outside a commit, reached a point of its mapping.
    Map(Map),
    /// A commit's damage mapper reached a point of its mapping, holding the serialization lock.
    CommitMap(Map),
    /// A checkpoint reached a point of its work.
    Checkpoint(Checkpoint),
    /// A vacuum reached one of its pace points, holding the vacuum lock.
    Vacuum(Vacuum),
    /// A rebuild of an index began a phase, holding the vacuum lock, and the index's write lock
    /// at [`Reindex::Sweep`]. An offline rebuild also holds the writes to its relation.
    Reindex(Reindex),
    /// The retirement of the pages of an index a rebuild replaced reached a point of its work,
    /// holding the vacuum lock, and the index's write lock at [`Retire::Held`].
    Retire(Retire),
    /// The checkpoint of a recycle of the log reached a point of its work, holding the recycle
    /// lock.
    Recycle(Checkpoint),
    /// A descent of a B-tree read a child's page id from its parent and has yet to read the
    /// child, holding the lock its search runs under, if any: the vacuum lock, the bundle writer
    /// lock, an index latch, the serialization lock with the readers lock as a commit records
    /// safe snapshot markers, or the transaction state lock as a commit records its time. A
    /// descent of the record of the window may hold the latch of a heap page whose versions it
    /// judges.
    Descended,
    /// The end of a transaction is to record its commit or abort, holding the transaction state
    /// lock, and the serialization lock when it commits.
    Recording,
}

/// A point of the damage mapper's work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Map {
    /// It took the records to map and has yet to walk their holders.
    Begun,
    /// It walked the holders of one record's page and found them.
    Walked,
    /// Its walk of the holders of one record's page was incomplete and found none.
    Incomplete,
    /// It set the marks and has yet to record the records mapped.
    Marked,
}

/// A point of a checkpoint's work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Checkpoint {
    /// It logged its record and folded the damage records, and has yet to force the log.
    Folded,
    /// It forced the log and has yet to point the file header at its record.
    Forced,
}

/// A pace point of a vacuum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vacuum {
    /// It is to scan a heap page for dead versions.
    Scan,
    /// It is to copy the versions of a heap page the retention window keeps to history.
    Keep,
    /// It is to prune a heap page.
    Prune,
    /// It cleaned an index leaf.
    Leaf,
    /// It swept a store chunk or page.
    Sweep,
    /// It forgot commit times or markers before the retention window.
    Expire,
}

/// A phase of a rebuild of an index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reindex {
    /// It is to scan the heaps into runs of entries.
    Scan,
    /// It is to build the new tree from the runs.
    Build,
    /// It is to catch up with the writes made since its scan.
    CatchUp,
    /// It is to take the index's write lock, catch up a last time and swap the new tree in.
    Swap,
    /// It swapped the new tree in and is to look for pages of the old tree its walk missed.
    Sweep,
    /// It is to free the pages of the old tree.
    Reclaim,
}

impl Reindex {
    /// Returns the name a job's row shows for this phase.
    pub fn name(self) -> &'static str {
        match self {
            Reindex::Scan => "scan",
            Reindex::Build => "build",
            Reindex::CatchUp => "catch up",
            Reindex::Swap => "swap",
            Reindex::Sweep => "sweep",
            Reindex::Reclaim => "reclaim",
        }
    }

    /// Returns the fraction of the rebuild done when this phase begins.
    pub fn done(self) -> f64 {
        match self {
            Reindex::Scan => 0.1,
            Reindex::Build => 0.4,
            Reindex::CatchUp => 0.6,
            Reindex::Swap => 0.8,
            Reindex::Sweep => 0.85,
            Reindex::Reclaim => 0.9,
        }
    }
}

/// A point of the retirement of the pages of a replaced tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Retire {
    /// The pager holds the pages as retired, and the swap that replaces their tree is to run.
    Held,
    /// The sound pages still allocated are marked deleted and the damaged ones scrapped, and no
    /// sound page is yet freed.
    Deleted,
    /// It freed one page.
    Freed,
}

/// A step function of the engine, by the boundaries it names.
#[cfg(any(test, feature = "testing"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepFn {
    Read,
    Write,
    TakeBack,
    Map,
    CommitMap,
    Checkpoint,
    Vacuum,
    Reindex,
    Retire,
    Recycle,
    Descend,
    Record,
}

#[cfg(any(test, feature = "testing"))]
impl Step {
    /// Returns the step function that names this boundary.
    pub fn function(self) -> StepFn {
        match self {
            Step::VersionRead | Step::ScanRead | Step::ScanStopped => StepFn::Read,
            Step::WriteChanged(_) | Step::WriteChecked | Step::WriteInserted => StepFn::Write,
            Step::TakeBack => StepFn::TakeBack,
            Step::Map(_) => StepFn::Map,
            Step::CommitMap(_) => StepFn::CommitMap,
            Step::Checkpoint(_) => StepFn::Checkpoint,
            Step::Vacuum(_) => StepFn::Vacuum,
            Step::Reindex(_) => StepFn::Reindex,
            Step::Retire(_) => StepFn::Retire,
            Step::Recycle(_) => StepFn::Recycle,
            Step::Descended => StepFn::Descend,
            Step::Recording => StepFn::Record,
        }
    }
}

#[cfg(any(test, feature = "testing"))]
impl Step {
    /// Returns the ranks of the engine locks a thread reaching this step may hold. A statement
    /// runs under the vacuum lock when a setting change runs it, and under the bundle writer
    /// lock when a registration does.
    fn holds(self) -> &'static [adtb_sys::lock::Rank] {
        use crate::ranks::{BUNDLE_WRITER, INDEX_LATCH, READERS, SERIALIZER, STATE, VACUUM};
        match self {
            Step::VersionRead
            | Step::ScanRead
            | Step::ScanStopped
            | Step::WriteChanged(_)
            | Step::WriteChecked
            | Step::WriteInserted
            | Step::TakeBack => &[VACUUM, BUNDLE_WRITER],
            Step::Checkpoint(_) | Step::Recycle(_) => &[],
            Step::CommitMap(_) => &[SERIALIZER],
            Step::Map(_) | Step::Vacuum(_) | Step::Retire(Retire::Deleted | Retire::Freed) => {
                &[VACUUM]
            }
            Step::Reindex(Reindex::Sweep) | Step::Retire(Retire::Held) => &[VACUUM, INDEX_LATCH],
            Step::Reindex(_) => &[VACUUM],
            Step::Descended | Step::Recording => &[
                VACUUM,
                BUNDLE_WRITER,
                INDEX_LATCH,
                SERIALIZER,
                READERS,
                STATE,
            ],
        }
    }

    /// Returns whether a thread reaching this step may hold a page latch: a descent of the
    /// record of the window may run while the latch of a heap page whose versions it judges is
    /// held.
    fn may_latch(self) -> bool {
        self == Step::Descended
    }
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

/// Called with each step on the thread that reaches it, holding no latch but one
/// [`Step::Descended`] names, and no engine lock but those its step names.
#[cfg(any(test, feature = "testing"))]
pub type StepHook = std::sync::Arc<dyn Fn(Step) + Send + Sync>;

/// What a test set on one database: its step hook, and a value limit below the format's.
#[derive(Default)]
pub struct Steps {
    #[cfg(any(test, feature = "testing"))]
    hook: std::sync::Mutex<Option<StepHook>>,
    /// The lowered value limit, 0 while none is set.
    #[cfg(any(test, feature = "testing"))]
    value_limit: std::sync::atomic::AtomicU64,
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
    /// Panic, in debug builds and under `latch-checks`, if the calling thread holds a page latch,
    /// or an engine lock `step` does not name.
    pub fn at(&self, step: Step) {
        let latched = adtb_sys::pages::latched_by_this_thread();
        assert!(
            latched == 0 || step.may_latch(),
            "{step:?} is reached while this thread latches {latched} pages"
        );
        if let Some(held) = adtb_sys::lock::held_beside(step.holds()) {
            panic!("{step:?} is reached holding {held}");
        }
        let hook = self
            .hook
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(hook) = hook {
            hook(step);
        }
    }

    /// Lower the longest value a heap of this database stores to `limit` bytes.
    ///
    /// Preconditions: `limit` is positive, and no heap of the database is open yet.
    pub fn lower_value_limit(&self, limit: u64) {
        self.value_limit
            .store(limit, std::sync::atomic::Ordering::Relaxed);
    }

    /// Returns the value limit a test lowered, if one did.
    pub fn value_limit(&self) -> Option<u64> {
        Some(self.value_limit.load(std::sync::atomic::Ordering::Relaxed)).filter(|&l| l > 0)
    }

    pub fn set(&self, hook: StepHook) {
        *self
            .hook
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(hook);
    }
}

#[cfg(not(any(test, feature = "testing")))]
impl Steps {
    pub fn at(&self, _: Step) {}

    pub fn value_limit(&self) -> Option<u64> {
        None
    }
}
