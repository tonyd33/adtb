//! Observe a pager and a set of transactions for tests, reading memory only but where an
//! observation says otherwise.

use std::collections::{BTreeMap, BTreeSet};

use crate::mvcc::{Snapshot, Transactions, Writers, Xid};
use crate::runtime::Runtime;
use crate::storage::{
    Bitmapped, Damage, LogFinding, Lsn, PageError, Pager, Pid, Position, Quarantine,
};

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

    /// Returns every object marked quarantined, in memory or in the file, without the database
    /// held for this open only.
    pub fn marks_not_held(self) -> BTreeSet<Quarantine> {
        self.pager.quarantined_not_held()
    }

    /// Returns whether `object` is quarantined, by its own mark or the database's.
    pub fn marked(self, object: Quarantine) -> bool {
        self.pager.is_quarantined(object)
    }

    /// Returns whether the allocation bitmap calls `pid` free.
    ///
    /// Fail if its bitmap page is damaged or was lost.
    pub fn is_free(self, pid: Pid) -> Result<bool, PageError> {
        match self.pager.bitmapped(pid) {
            Bitmapped::Free => Ok(true),
            Bitmapped::Allocated => Ok(false),
            Bitmapped::Unknown(e) => Err(e),
        }
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

    /// Returns whether a read or latch of `page` has met its refusal since the database opened.
    pub fn refusal_met(self, page: Pid) -> bool {
        self.pager.refusal_met(page)
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
        self.txns.pager().whole_cause()
    }
}

/// Returns the writers of a version `xmin` inserted and `xmax` deleted, none when `xmax` is 0.
pub fn writers(xmin: u64, xmax: u64) -> Writers {
    let inserted = Writers::inserted(Xid(xmin));
    match xmax {
        0 => inserted,
        x => inserted.superseded_by(Xid(x)),
    }
}
