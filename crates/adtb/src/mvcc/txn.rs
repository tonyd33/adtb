use std::collections::{BTreeMap, BTreeSet};

use crate::runtime::Runtime;
use crate::storage::{Lsn, PageError, Pager, Pid};

use super::{CommitLog, Holds, Scope, Snapshot, SnapshotId, Status, Xid};

/// The number of ids one xid allocation record reserves.
pub const RESERVATION: u64 = 1024;

/// Returns the reservation that follows one ending at `reserved`, or `None` once ids are spent.
pub(crate) fn extend(reserved: Xid) -> Option<Xid> {
    (reserved.0 < u64::MAX).then(|| Xid(reserved.0.saturating_add(RESERVATION)))
}

/// The transactions of one database: id assignment, the commit log, running transactions and
/// the snapshots that hold back reclamation.
#[derive(Debug)]
pub struct Transactions {
    clog: CommitLog,
    next: Xid,
    running: BTreeSet<Xid>,
    holds: Holds,
    /// The LSN of the latest commit record logged since the commit log was opened.
    last_commit: Option<Lsn>,
    /// The `last_commit` of each held snapshot when it was taken.
    reads: BTreeMap<SnapshotId, Option<Lsn>>,
}

impl Transactions {
    pub fn create<R: Runtime>(pager: &Pager<R>) -> Result<Transactions, PageError> {
        Ok(Transactions::over(CommitLog::create(pager)?))
    }

    /// Open the commit log rooted at `root` in a recovered pager.
    ///
    /// Postconditions:
    ///
    /// - Every id assigned before the crash reads as committed exactly when its commit record was
    ///   recovered, and as aborted otherwise.
    /// - No id assigned before the crash is assigned again.
    pub fn open<R: Runtime>(pager: &Pager<R>, root: Pid) -> Result<Transactions, PageError> {
        Ok(Transactions::over(CommitLog::open(pager, root)?))
    }

    fn over(clog: CommitLog) -> Transactions {
        Transactions {
            next: clog.reserved(),
            clog,
            running: BTreeSet::new(),
            holds: Holds::default(),
            last_commit: None,
            reads: BTreeMap::new(),
        }
    }

    pub fn commit_log(&self) -> &CommitLog {
        &self.clog
    }

    /// Returns the id the next transaction gets.
    pub fn next(&self) -> Xid {
        self.next
    }

    pub fn running(&self) -> &BTreeSet<Xid> {
        &self.running
    }

    /// Start a transaction, first reserving more ids and forcing the log when the reservation is
    /// spent.
    ///
    /// Panics once every 64-bit id is spent.
    pub fn begin<R: Runtime>(&mut self, pager: &Pager<R>) -> Result<Xid, PageError> {
        if self.next == self.clog.reserved() {
            let upto = extend(self.next).expect("transaction ids are exhausted");
            let resolved = self.running.first().copied().unwrap_or(self.next);
            self.clog.reserve(pager, upto, resolved)?;
        }
        let x = self.next;
        self.next = Xid(x.0 + 1);
        self.running.insert(x);
        Ok(x)
    }

    /// Commit `x`: record it in the commit log, then log its commit record. The commit is durable
    /// once the log is durable past the returned LSN.
    ///
    /// Fails, leaving `x` running, if its commit log page is damaged.
    ///
    /// Panics if `x` is not running.
    pub fn commit<R: Runtime>(&mut self, pager: &Pager<R>, x: Xid) -> Result<Lsn, PageError> {
        assert!(self.running.contains(&x), "{x:?} is not running");
        let lsn = self.clog.commit(pager, x)?;
        self.running.remove(&x);
        self.last_commit = Some(lsn);
        Ok(lsn)
    }

    /// Abort `x`: record it in the commit log, then log its abort record.
    ///
    /// Fails, leaving `x` running, if its commit log page is damaged.
    ///
    /// Panics if `x` is not running.
    pub fn abort<R: Runtime>(&mut self, pager: &Pager<R>, x: Xid) -> Result<Lsn, PageError> {
        assert!(self.running.contains(&x), "{x:?} is not running");
        let lsn = self.clog.abort(pager, x)?;
        self.running.remove(&x);
        Ok(lsn)
    }

    /// Returns the commit log status of `x`, an id already assigned.
    pub fn status<R: Runtime>(&self, pager: &Pager<R>, x: Xid) -> Result<Status, PageError> {
        assert!(x < self.next, "{x:?} is not assigned");
        self.clog.status(pager, x)
    }

    /// Take a snapshot of the transactions now committed and hold back the heaps of `scope` for
    /// it until it is released.
    pub fn take(&mut self, scope: Scope) -> (SnapshotId, Snapshot) {
        let in_progress: Vec<Xid> = self.running.iter().copied().collect();
        let xmin = in_progress.first().copied().unwrap_or(self.next);
        let snapshot = Snapshot::new(xmin, self.next, in_progress);
        let id = self.holds.hold(xmin, scope);
        self.reads.insert(id, self.last_commit);
        (id, snapshot)
    }

    pub fn release(&mut self, id: SnapshotId) {
        self.holds.release(id);
        self.reads.remove(&id);
    }

    /// Returns the LSN of the commit record of the latest commit snapshot `id` may read, or
    /// `None` when it reads only commits made before the commit log was opened, which recovery
    /// made durable. A transaction reading at `id` is acknowledged only once the log is durable
    /// past this LSN.
    ///
    /// Panics if `id` is not held.
    pub fn read_upto(&self, id: SnapshotId) -> Option<Lsn> {
        *self
            .reads
            .get(&id)
            .unwrap_or_else(|| panic!("{id:?} is not held"))
    }

    /// Returns whether the version whose header holds `xmin` and `xmax` is visible to `reader`
    /// at `snapshot`.
    pub fn sees<R: Runtime>(
        &self,
        pager: &Pager<R>,
        snapshot: &Snapshot,
        reader: Option<Xid>,
        xmin: u64,
        xmax: u64,
    ) -> Result<bool, PageError> {
        snapshot.sees(reader, xmin, xmax, |x| self.clog.status(pager, x))
    }

    pub fn holds_mut(&mut self) -> &mut Holds {
        &mut self.holds
    }

    /// Returns the watermark of `heap`: the oldest snapshot xmin any snapshot or replica holding
    /// `heap` has, or the next id when none holds it.
    pub fn watermark(&self, heap: u32) -> Xid {
        self.holds.watermark(heap, self.next)
    }

    /// Truncate the commit log below `bound`.
    ///
    /// Preconditions: those of [`CommitLog::truncate`].
    ///
    /// Panics if a transaction below `bound` is running, or if `bound` is not assigned yet.
    pub fn truncate<R: Runtime>(&mut self, pager: &Pager<R>, bound: Xid) -> Result<(), PageError> {
        assert!(bound <= self.next, "{bound:?} is not assigned yet");
        assert!(
            self.running.first().is_none_or(|&r| r >= bound),
            "a transaction below {bound:?} is running"
        );
        self.clog.truncate(pager, bound)
    }
}
