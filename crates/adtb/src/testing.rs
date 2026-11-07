//! What tests observe of a running engine: marks, damage evidence, log positions, maintenance
//! counters, the scrub cursor and held findings. Every answer is a value built at the call; no
//! answer borrows from the engine. Every observation but [`Probe::damage_in_log`] and
//! [`Probe::logged_commits`] reads memory only and changes nothing. Those two read the retained
//! log as `verify` does: they pin it, open its segments and take the simulator's fault draws.

use std::collections::{BTreeMap, BTreeSet};

use crate::Database;
use crate::mvcc::{Transactions, Xid};
use crate::runtime::Runtime;
use crate::storage::{Damage, LogFinding, Lsn, PageError, Pager, Pid, Quarantine};
use crate::vacuum::Charged;

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
        TxnProbe::of(self.db.tested_write_path().txns())
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
        self.db.tested_write_path().pacer().charged(rt)
    }

    /// Returns how many pauses calls have made at a rebuild's gate or for the vacuum lock.
    pub fn rebuild_waits(self) -> u64 {
        self.db.tested_write_path().rebuild_waits()
    }

    /// Returns how many times a call asked a background vacuum to give up the vacuum lock.
    pub fn vacuum_yields(self) -> u64 {
        self.db.tested_write_path().vacuum_yields()
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
