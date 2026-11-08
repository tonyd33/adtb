//! What tests observe of a running engine, and the controls that drive it where no public call
//! reaches. Every answer is a value built at the call; no answer borrows from the engine.
//! [`Plant`] lists every change a test makes to the engine's state beneath its public calls.
//! Every observation but [`Probe::page`], [`Probe::damage_in_log`], [`Probe::logged_commits`]
//! and [`Probe::safe_snapshots`] leaves the engine as it found it, reading memory only.
//! [`Probe::page`] reads through the pager as any reader does. The other three read the retained
//! log as `verify` does: they pin it, open its segments and take the simulator's fault draws.
//! [`Probe::safe_snapshots`] writes the log first.

mod control;
mod object;
mod plant;

use std::collections::{BTreeMap, BTreeSet};

use crate::Database;
use crate::btree;
use crate::mvcc::{Limits, Snapshot, Transactions, Xid};
use crate::runtime::Runtime;
use crate::storage::{Damage, LogFinding, Lsn, PageError, Pager, Pid, Position, Quarantine};
use crate::vacuum::Charged;
use crate::write;

pub use control::{PageHold, VacuumHold};
pub use object::{CommitLogPages, Object};
pub use plant::{Plant, Planted, Site};

/// The settings of an open only tests give. Each changes scheduling, never what a call returns
/// on success.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Knobs {
    no_background_vacuum: bool,
    segment_size: Option<u64>,
    limits: Option<Limits>,
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

    pub(crate) fn background_vacuum(&self) -> bool {
        !self.no_background_vacuum
    }

    pub(crate) fn segment_size(&self) -> Option<u64> {
        self.segment_size
    }

    pub(crate) fn limits(&self) -> Limits {
        self.limits.unwrap_or_default()
    }
}

/// Why a control of this module did nothing.
#[derive(Debug)]
pub enum Failed {
    /// The database has no such structure.
    Absent(Object),
    /// The object has no listing of the pages asked for.
    Unlisted(Object),
    /// The database has no relation of the name given.
    /// The object is no relation's key index.
    NotKey(Object),
    /// The change found nothing to change.
    Unchanged,
    Page(PageError),
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

    /// Returns how many times a call asked a background vacuum to give up the vacuum lock.
    pub fn vacuum_yields(self) -> u64 {
        self.db.tested_db().vacuum_yields()
    }

    /// Returns how many transactions the serializer still tracks, running or kept for a running
    /// one's conflicts.
    pub fn tracked(self) -> usize {
        self.db.tested_db().tracked()
    }

    /// Returns the snapshot `txn` reads, `None` once it has ended.
    pub fn snapshot(self, txn: &crate::AdHocTransaction<'_, R>) -> Option<Snapshot> {
        txn.tested_snapshot().cloned()
    }

    /// Returns how many reindex job threads are not yet joined.
    pub fn reindex_threads(self) -> usize {
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

/// The damage recorded and not cleared at one instant.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Records {
    pages: BTreeMap<Pid, Damage>,
    log: BTreeSet<LogFinding>,
    full: bool,
}

impl Records {
    /// Returns each recorded page with its damage.
    pub fn pages(&self) -> &BTreeMap<Pid, Damage> {
        &self.pages
    }

    /// Returns the stale blocks recovery found past the end of the log.
    pub fn log(&self) -> &BTreeSet<LogFinding> {
        &self.log
    }

    /// Returns whether the damage list ran out of room, so the whole file counts as damaged.
    pub fn full(&self) -> bool {
        self.full
    }
}

/// The observations of one pager.
#[derive(Debug)]
pub struct Probe<'a, R: Runtime> {
    pager: &'a Pager<R>,
}

impl<R: Runtime> Clone for Probe<'_, R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R: Runtime> Copy for Probe<'_, R> {}

impl<'a, R: Runtime> Probe<'a, R> {
    pub fn of(pager: &'a Pager<R>) -> Probe<'a, R> {
        Probe { pager }
    }

    /// Returns every object marked quarantined, in memory or in the file.
    pub fn marks(self) -> BTreeSet<Quarantine> {
        self.pager.quarantined()
    }

    /// Returns whether `object` is quarantined, by its own mark or the database's.
    pub fn marked(self, object: Quarantine) -> bool {
        self.pager.is_quarantined(object)
    }

    /// Returns the pages refused this open, each with what is wrong with it.
    pub fn refusals(self) -> BTreeMap<Pid, Damage> {
        self.pager.damaged()
    }

    /// Returns every damage recorded and not cleared: the list's, the log's since and this
    /// open's, in one snapshot.
    pub fn records(self) -> Records {
        let r = self.pager.recorded();
        Records {
            pages: r.pages,
            log: r.log,
            full: r.full,
        }
    }

    /// Returns each damage record in the retained log, in log order, none when its segments
    /// cannot be listed or opened.
    pub fn damage_in_log(self) -> Vec<(Pid, Damage)> {
        self.pager.logged_damage()
    }

    /// Returns the transaction of every commit record in the retained log, none when its
    /// segments cannot be listed or opened.
    pub fn logged_commits(self) -> BTreeSet<u64> {
        self.pager.logged_commits()
    }

    /// Returns each safe snapshot marker in the retained log, in log order, after writing the
    /// log: the commit position it covers and its snapshot. Nothing when its segments cannot be
    /// listed or opened.
    ///
    /// Panic at a marker or a batch of the log that does not decode.
    pub fn safe_snapshots(self) -> Vec<(Position, Snapshot)> {
        self.pager.flush_log();
        self.pager
            .logged_safe_snapshots()
            .into_iter()
            .map(|(at, bytes)| {
                let snapshot = Snapshot::decode(&bytes).expect("a marker's snapshot decodes");
                (at, snapshot)
            })
            .collect()
    }

    /// Returns the LSN the next record appended takes.
    pub fn appended(self) -> Lsn {
        self.pager.appended()
    }

    /// Returns the LSN below which every record is durable.
    pub fn durable(self) -> Lsn {
        self.pager.durable()
    }

    /// Returns the LSN of the checkpoint record the file header names.
    pub fn checkpoint(self) -> Lsn {
        self.pager.checkpoint_lsn()
    }

    /// Returns each dirty page with the LSN of the change that first dirtied it.
    pub fn dirty(self) -> BTreeMap<Pid, Lsn> {
        self.pager.dirty_pages()
    }

    /// Returns the bytes of `page` as a reader sees them, refusing it as any read does.
    pub fn page(self, page: Pid) -> Result<Vec<u8>, PageError> {
        Ok(self.pager.read(page)?.to_vec())
    }

    pub fn page_size(self) -> usize {
        self.pager.page_size()
    }

    /// Returns the objects the damage of `page` was mapped to, `None` while it is not mapped.
    pub fn mapped_to(self, page: Pid) -> Option<BTreeSet<Quarantine>> {
        self.pager.mapped_to(page)
    }

    /// Returns whether `at` lies on the history of this file, as a restore to it checks.
    pub fn on_history(self, at: Position) -> bool {
        self.pager.on_history(at)
    }

    /// Returns how many damaged pages this open has walked the objects for.
    pub fn walks(self) -> u64 {
        self.pager.walked()
    }
}

/// The observations of one set of transactions.
#[derive(Debug)]
pub struct TxnProbe<'a, R: Runtime> {
    txns: &'a Transactions<R>,
}

impl<'a, R: Runtime> TxnProbe<'a, R> {
    pub fn of(txns: &'a Transactions<R>) -> TxnProbe<'a, R> {
        TxnProbe { txns }
    }

    /// Returns the id the next transaction gets.
    pub fn next_xid(&self) -> Xid {
        self.txns.next()
    }

    /// Returns the damage that quarantined the database through its commit log, if any.
    pub fn commit_log_damage(&self) -> Option<PageError> {
        self.txns.quarantined()
    }
}
