//! What tests observe of a running engine, and the controls that drive it where no public call
//! reaches. Every answer is a value built at the call; no answer borrows from the engine.
//! [`Plant`] lists every change a test makes to the engine's state beneath its public calls.
//! Every observation but [`Probe::page`], [`Probe::damage_in_log`], [`Probe::logged_commits`]
//! and [`Probe::safe_snapshots`] leaves the engine as it found it, reading memory only.
//! [`Probe::page`] reads through the pager as any reader does. The other three read the retained
//! log as `verify` does: they pin it, open its segments and take the simulator's fault draws.
//! [`Probe::safe_snapshots`] writes the log first.

mod control;
mod layout;
mod object;
mod plant;

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::Database;
use crate::btree;
use crate::exec;
use crate::plan;
use crate::runtime::{Runtime, ThreadId};
use crate::write;

pub use crate::catalog::chained as chained_hash;
pub use crate::heap::{Edit, Error as HeapError, Flags, Header, Laid, Slot};
pub use crate::mvcc::{Gate, Limits, Snapshot, Status, Xid};
pub use crate::plan::{
    Check as PlanCheck, Checked as PlansChecked, Rewrites, checked as plans_checked, with_rewrites,
};
pub use crate::steps::{Change, Checkpoint, Map, Reindex, Retire, Step, StepFn, Vacuum};
pub use crate::storage::policy::Holder;
pub use crate::storage::{
    Damage, LogFinding, LogProblem, Lsn, PageError, PageKind, Pid, Position as LogPosition,
    Quarantine, Shape, manifest_hash,
};
pub use crate::vacuum::Charged;
pub use crate::write::panic_text;
pub use adtb_store::testing::{Probe, Records, TxnProbe};
pub use control::{PageHold, VacuumHold};
pub use object::{CommitLogPages, Object};
pub use plant::{Plant, Planted, Site};

/// Read and seal the header every page of the file begins with.
pub mod page {
    pub use crate::storage::{Crc32c, FIRST_EDITABLE, HEADER_LEN, kind, lsn, owner, seal};
}

/// Read, write and encode the entries of a dump.
pub mod dump {
    pub use crate::catalog::arrows::{BATCH_LIMIT, file_schema, read_rows, write_rows};
    pub use crate::storage::{read_dump, write_dump};
}

/// Name the files a create leaves beside the database it makes until it finishes.
pub mod create {
    pub use crate::storage::{aside, creating, marker, marker_bytes};
}

/// The settings of an open only tests give. Each changes scheduling, never what a call returns
/// on success.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Knobs {
    no_background_vacuum: bool,
    segment_size: Option<u64>,
    limits: Option<Limits>,
    morsel_rows: Option<usize>,
    range_pages: Option<usize>,
    spill_bytes: Option<usize>,
}

impl Knobs {
    /// Returns the knobs that start no background vacuum worker.
    pub fn without_background_vacuum() -> Knobs {
        Knobs {
            no_background_vacuum: true,
            ..Knobs::default()
        }
    }

    /// Returns these knobs creating a file whose log segments are `bytes` each. An open of an
    /// existing file keeps the size it was created with.
    ///
    /// Precondition: `bytes` is a positive multiple of the log block, 512 bytes, or open fails
    /// with an I/O error of kind `InvalidInput`.
    pub fn with_segment_size(self, bytes: u64) -> Knobs {
        Knobs {
            segment_size: Some(bytes),
            ..self
        }
    }

    /// Returns these knobs promoting the serializer's read locks at `limits`.
    pub fn with_limits(self, limits: Limits) -> Knobs {
        Knobs {
            limits: Some(limits),
            ..self
        }
    }

    /// Returns these knobs running each statement in morsels of `morsel_rows` rows, or of the engine's
    /// size with `None`.
    pub fn with_morsel_rows(self, morsel_rows: Option<usize>) -> Knobs {
        Knobs {
            morsel_rows,
            ..self
        }
    }

    /// Returns these knobs scanning a heap in parallel tasks of `range_pages` pages each, or of the
    /// engine's size with `None`.
    pub fn with_range_pages(self, range_pages: Option<usize>) -> Knobs {
        Knobs {
            range_pages,
            ..self
        }
    }

    /// Returns these knobs bounding the spill file of each statement at `spill_bytes`, or at the
    /// engine's bound with `None`.
    pub fn with_spill_bytes(self, spill_bytes: Option<usize>) -> Knobs {
        Knobs {
            spill_bytes,
            ..self
        }
    }

    pub(crate) fn exec(&self, defaults: exec::Options) -> exec::Options {
        exec::Options {
            morsel_rows: self.morsel_rows.unwrap_or(defaults.morsel_rows),
            range_pages: self.range_pages.unwrap_or(defaults.range_pages),
            spill_bytes: self.spill_bytes.unwrap_or(defaults.spill_bytes),
            ..defaults
        }
    }

    pub(crate) fn background_vacuum(&self) -> bool {
        !self.no_background_vacuum
    }

    pub(crate) fn segment_size(&self) -> Option<u64> {
        self.segment_size
    }

    /// Returns the thresholds at which the serializer's read locks promote.
    pub fn limits(&self) -> Limits {
        self.limits.unwrap_or_default()
    }
}

/// Why a control of this module did nothing.
#[derive(Debug, PartialEq)]
pub enum Failed {
    /// The database has no such structure.
    Absent(Object),
    /// The object has no listing of the pages asked for.
    Unlisted(Object),
    /// No mark names the object apart from the database's.
    Shared(Object),
    /// The database has no relation of the name given.
    /// The object is no relation's key index.
    NotKey(Object),
    /// The change found nothing to change.
    Unchanged,
    Page(PageError),
    Mark(crate::storage::MarkFailure),
    Tree(btree::Error),
    Heap(crate::heap::Error),
    Orphan(crate::heap::OrphanEntry),
    Write(write::Error),
    Txn(crate::mvcc::Error),
}

impl From<PageError> for Failed {
    fn from(e: PageError) -> Failed {
        Failed::Page(e)
    }
}

impl From<crate::storage::MarkFailure> for Failed {
    fn from(e: crate::storage::MarkFailure) -> Failed {
        Failed::Mark(e)
    }
}

impl From<btree::Error> for Failed {
    fn from(e: btree::Error) -> Failed {
        Failed::Tree(e)
    }
}

impl From<crate::heap::OrphanEntry> for Failed {
    fn from(e: crate::heap::OrphanEntry) -> Failed {
        Failed::Orphan(e)
    }
}

impl From<crate::heap::Error> for Failed {
    fn from(e: crate::heap::Error) -> Failed {
        Failed::Heap(e)
    }
}

/// The observations of one open database.
#[derive(Debug)]
pub struct Testing<'a, R: Runtime> {
    db: &'a Database<R>,
}

impl<R: Runtime> Clone for Testing<'_, R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R: Runtime> Copy for Testing<'_, R> {}

impl<'a, R: Runtime> Testing<'a, R> {
    pub(crate) fn of(db: &'a Database<R>) -> Testing<'a, R> {
        Testing { db }
    }

    /// Returns the probe over the database's pager.
    pub fn probe(self) -> Probe<'a, R> {
        Probe::of(self.db.tested_pager())
    }

    /// Returns the probe over the database's transactions.
    pub fn txn_probe(self) -> TxnProbe<'a, R> {
        TxnProbe::of(self.db.tested_db().txns())
    }

    /// Returns the id the next transaction gets.
    pub fn next_xid(self) -> Xid {
        self.txn_probe().next_xid()
    }

    /// Returns the damage that quarantined the database through its commit log, if any.
    pub fn commit_log_damage(self) -> Option<PageError> {
        self.txn_probe().commit_log_damage()
    }

    /// Returns the steps the vacuum pacer charged since its budget last changed.
    pub fn charged(self) -> Charged {
        let rt = self.db.tested_runtime();
        self.db.tested_db().pacer().charged(rt)
    }

    /// Returns how many pauses calls have made at a rebuild's gate or for the vacuum lock.
    pub fn rebuild_waits(self) -> u64 {
        self.db.tested_db().rebuild_waits()
    }

    /// Returns whether a thread waits for the vacuum lock or a rebuild's gate while `holder` holds
    /// it, or for a transaction begun on `holder` to end.
    pub fn waits_on(self, holder: ThreadId) -> bool {
        self.db.tested_db().waits_on(holder)
    }

    /// Vacuum relation `relation`, by qualified name. Returns what the vacuum did in all.
    ///
    /// Panics:
    /// - No relation is named `relation`.
    pub fn vacuum(self, relation: &str) -> Result<crate::vacuum::Report, write::Error> {
        let db = self.db.tested_db();
        let rel =
            (db.rel_named(relation)).unwrap_or_else(|| panic!("no relation is named {relation}"));
        Ok(db.vacuum(rel, write::HoldWait::unbounded())?.report)
    }

    /// Returns how many times a call asked a background vacuum to give up the vacuum lock.
    pub fn vacuum_yields(self) -> u64 {
        self.db.tested_db().vacuum_yields()
    }

    /// Returns how many transactions the serializer still tracks, running or kept for a running
    /// one's conflicts.
    pub fn tracked(self) -> usize {
        self.db.tested_db().tracked()
    }

    /// Returns how many read-only transactions keep a snapshot for their safe snapshot marker.
    pub fn readers(self) -> usize {
        self.db.tested_db().readers_held()
    }

    /// Returns the structures the catalog names for the relations of the modules the engine
    /// declares when `engine`, else of every other module, one line each.
    pub fn catalog(self, engine: bool) -> Result<Vec<String>, Failed> {
        Ok(self
            .db
            .tested_catalog()
            .probed(self.db.tested_pager(), engine)?)
    }

    /// Returns the SIREAD locks `txn` holds, each named by relation and constructor, sorted;
    /// none once it has ended.
    ///
    /// Precondition: `txn` is a transaction of this database.
    pub fn locks(self, txn: &crate::AdHocTransaction<'_, R>) -> Vec<String> {
        let Some(tx) = txn.tested_txn() else {
            return Vec::new();
        };
        let catalog = self.db.tested_catalog();
        let held = self.db.tested_db().locks(tx);
        let mut out: Vec<String> = held.iter().map(|l| catalog.lock_named(l)).collect();
        out.sort();
        out
    }

    /// Returns the transactions running: begun, and neither recorded ended nor kept unresolved.
    pub fn running(self) -> BTreeSet<Xid> {
        self.db.tested_db().txns().running()
    }

    /// Returns the transactions a waiter may wait on, started and not yet ended.
    pub fn waits_running(self) -> BTreeSet<Xid> {
        self.db.tested_db().waits_running()
    }

    /// Returns the transactions some waiter waits on to end.
    pub fn waited_on(self) -> BTreeSet<Xid> {
        self.db.tested_db().waited_on()
    }

    /// Returns how many transactions have ended by being dropped since the count was last
    /// taken, and starts the count again. A close asserts the count is zero.
    pub fn take_ends_by_drop(self) -> usize {
        self.db.tested_db().take_ends_by_drop()
    }

    /// Returns the snapshot `txn` reads, `None` once it has ended.
    pub fn snapshot(self, txn: &crate::AdHocTransaction<'_, R>) -> Option<Snapshot> {
        txn.tested_snapshot().cloned()
    }

    /// Returns the executor's store as the statements of `txn` read it: at its snapshot, under
    /// its entry in the serializer. `None` once `txn` has ended.
    ///
    /// Preconditions:
    /// - `txn` is a transaction of this database, or this panics;
    /// - the caller only reads through the store, and drops it before `txn` ends.
    pub fn store(self, txn: &crate::AdHocTransaction<'_, R>) -> Option<Arc<exec::Store<R>>> {
        self.db.tested_store(txn)
    }

    /// Returns the planner's description of every relation the database runs.
    pub fn plan_catalog(self) -> plan::Catalog {
        self.db.tested_db().planned().clone()
    }

    /// Returns the reindex job threads not yet joined.
    pub fn reindex_threads(self) -> Vec<ThreadId> {
        self.db.reindex_threads()
    }

    /// Returns the next page of the background scrub's pass as last saved in the file: the page,
    /// `None` between passes, and `None` of the outer option when nothing has been saved.
    pub fn scrub(self) -> Option<Option<Pid>> {
        self.db.saved_scrub_page()
    }

    /// Returns how many rows of `sys::verify_findings` the engine holds in memory.
    pub fn held_findings(self) -> usize {
        self.db.held_findings()
    }
}
