use std::collections::{BTreeMap, BTreeSet};
use std::sync::{OnceLock, RwLockReadGuard};

use adtb_sys::lock::{Lock, RwLock};

use crate::runtime::Runtime;
use crate::storage::{Lsn, PageError, Pager, Pid};

use super::{CommitLog, Error, Holds, Scope, Snapshot, SnapshotId, Status, Xid};

/// The number of ids one xid allocation record reserves.
pub const RESERVATION: u64 = 1024;

/// Returns the reservation that follows one ending at `reserved`, or `None` once ids are spent.
pub(crate) fn extend(reserved: Xid) -> Option<Xid> {
    (reserved.0 < u64::MAX).then(|| Xid(reserved.0.saturating_add(RESERVATION)))
}

/// What `begin` does with `next`, the next id, in a reservation ending at `reserved`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Assign {
    Next,
    /// Reserve the ids below this one first.
    Reserve(Xid),
    /// Every id is spent; `u64::MAX` is never assigned, so `next` never overflows.
    Exhausted,
}

pub(crate) fn assign(next: Xid, reserved: Xid) -> Assign {
    if next.0 == u64::MAX {
        Assign::Exhausted
    } else if next == reserved {
        extend(reserved).map_or(Assign::Exhausted, Assign::Reserve)
    } else {
        Assign::Next
    }
}

#[derive(Debug)]
struct State {
    next: Xid,
    running: BTreeSet<Xid>,
    holds: Holds,
    /// The LSN of the latest commit record logged since the commit log was opened.
    last_commit: Option<Lsn>,
    /// The `last_commit` of each held snapshot when it was taken.
    reads: BTreeMap<SnapshotId, Option<Lsn>>,
}

/// The transactions of one database: id assignment, the commit log, running transactions and
/// the snapshots that hold back reclamation. Any number of threads share it.
///
/// A status lookup takes only a shared hold on the commit log, which a commit or abort also
/// takes shared; a reservation of more ids and a truncation take it alone. Every other call
/// takes the lock over running transactions and snapshots, then, if it needs it, the commit log.
///
/// Once any commit log page fails verification the database is quarantined: from then on every
/// call that starts, ends or reads for a transaction fails with [`Error::Corrupt`], naming the
/// first page found damaged. The commit log stays readable through [`Transactions::commit_log`]
/// for verification and repair.
#[derive(Debug)]
pub struct Transactions<R: Runtime> {
    rt: R,
    clog: RwLock<CommitLog>,
    state: Lock<State>,
    quarantine: OnceLock<PageError>,
}

impl<R: Runtime> Transactions<R> {
    pub fn create(rt: R, pager: &Pager<R>) -> Result<Transactions<R>, PageError> {
        Ok(Transactions::over(rt, CommitLog::create(pager)?))
    }

    /// Open the commit log rooted at `root` in a recovered pager.
    ///
    /// Postconditions:
    ///
    /// - Every id assigned before the crash reads as committed exactly when its commit record was
    ///   recovered, and as aborted otherwise.
    /// - No id assigned before the crash is assigned again.
    /// - If a status page the open reads is damaged, the database is quarantined.
    ///
    /// Fails if the commit log's root or a directory page is damaged.
    pub fn open(rt: R, pager: &Pager<R>, root: Pid) -> Result<Transactions<R>, PageError> {
        let mut clog = CommitLog::load(pager, root)?;
        let unresolved = clog.abort_unresolved(pager);
        let txns = Transactions::over(rt, clog);
        if let Err(e) = unresolved {
            txns.damaged(e);
        }
        Ok(txns)
    }

    fn over(rt: R, clog: CommitLog) -> Transactions<R> {
        let state = State {
            next: clog.reserved(),
            running: BTreeSet::new(),
            holds: Holds::default(),
            last_commit: None,
            reads: BTreeMap::new(),
        };
        Transactions {
            rt,
            clog: RwLock::new(clog),
            state: Lock::new(state),
            quarantine: OnceLock::new(),
        }
    }

    /// Returns the damage that quarantined the database, if any.
    pub fn quarantined(&self) -> Option<PageError> {
        self.quarantine.get().copied()
    }

    fn admit(&self) -> Result<(), Error> {
        self.quarantined()
            .map_or(Ok(()), |e| Err(Error::Corrupt(e)))
    }

    /// Quarantine the database for `e`, keeping the first damage found. Returns the failure.
    fn damaged(&self, e: PageError) -> Error {
        Error::Corrupt(*self.quarantine.get_or_init(|| e))
    }

    fn checked<T>(&self, r: Result<T, PageError>) -> Result<T, Error> {
        r.map_err(|e| self.damaged(e))
    }

    /// Returns the commit log, held shared until the guard drops.
    pub fn commit_log(&self) -> RwLockReadGuard<'_, CommitLog> {
        self.clog.read(&self.rt)
    }

    /// Returns the id the next transaction gets.
    pub fn next(&self) -> Xid {
        self.state.lock(&self.rt).next
    }

    pub fn running(&self) -> BTreeSet<Xid> {
        self.state.lock(&self.rt).running.clone()
    }

    /// Start a transaction, first reserving more ids and forcing the log when the reservation is
    /// spent.
    ///
    /// Fails with [`Error::Exhausted`] once every 64-bit id is spent; snapshots and reads go on.
    pub fn begin(&self, pager: &Pager<R>) -> Result<Xid, Error> {
        self.admit()?;
        let mut st = self.state.lock(&self.rt);
        let reserved = self.clog.read(&self.rt).reserved();
        match assign(st.next, reserved) {
            Assign::Next => {}
            Assign::Exhausted => return Err(Error::Exhausted),
            Assign::Reserve(upto) => {
                let resolved = st.running.first().copied().unwrap_or(st.next);
                let reserved = self.clog.write(&self.rt).reserve(pager, upto, resolved);
                self.checked(reserved)?;
            }
        }
        let x = st.next;
        st.next = Xid(x.0 + 1);
        st.running.insert(x);
        Ok(x)
    }

    /// Commit `x`: record it in the commit log, then log its commit record. The commit is durable
    /// once the log is durable past the returned LSN.
    ///
    /// Fails, leaving `x` running, if the database is quarantined.
    ///
    /// Panics if `x` is not running.
    pub fn commit(&self, pager: &Pager<R>, x: Xid) -> Result<Lsn, Error> {
        self.admit()?;
        let mut st = self.state.lock(&self.rt);
        assert!(st.running.contains(&x), "{x:?} is not running");
        let committed = self.clog.read(&self.rt).commit(pager, x);
        let lsn = self.checked(committed)?;
        st.running.remove(&x);
        st.last_commit = Some(lsn);
        Ok(lsn)
    }

    /// Abort `x`: record it in the commit log, then log its abort record.
    ///
    /// Fails, leaving `x` running, if the database is quarantined.
    ///
    /// Panics if `x` is not running.
    pub fn abort(&self, pager: &Pager<R>, x: Xid) -> Result<Lsn, Error> {
        self.admit()?;
        let mut st = self.state.lock(&self.rt);
        assert!(st.running.contains(&x), "{x:?} is not running");
        let aborted = self.clog.read(&self.rt).abort(pager, x);
        let lsn = self.checked(aborted)?;
        st.running.remove(&x);
        Ok(lsn)
    }

    /// Returns the commit log status of `x`, an id already reserved.
    ///
    /// Fails with the first damage found once the database is quarantined.
    pub fn status(&self, pager: &Pager<R>, x: Xid) -> Result<Status, PageError> {
        if let Some(e) = self.quarantined() {
            return Err(e);
        }
        let status = self.clog.read(&self.rt).status(pager, x);
        status.map_err(|e| *self.quarantine.get_or_init(|| e))
    }

    /// Take a snapshot of the transactions now committed and hold back the heaps of `scope` for
    /// it until it is released.
    ///
    /// Fails if the database is quarantined.
    pub fn take(&self, scope: Scope) -> Result<(SnapshotId, Snapshot), Error> {
        self.admit()?;
        let mut st = self.state.lock(&self.rt);
        let in_progress: Vec<Xid> = st.running.iter().copied().collect();
        let xmin = in_progress.first().copied().unwrap_or(st.next);
        let snapshot = Snapshot::new(xmin, st.next, in_progress);
        let id = st.holds.hold(xmin, scope);
        let read = st.last_commit;
        st.reads.insert(id, read);
        Ok((id, snapshot))
    }

    pub fn release(&self, id: SnapshotId) {
        let mut st = self.state.lock(&self.rt);
        st.holds.release(id);
        st.reads.remove(&id);
    }

    /// Returns the LSN of the commit record of the latest commit snapshot `id` may read, or
    /// `None` when it reads only commits made before the commit log was opened, which recovery
    /// made durable. A transaction reading at `id` is acknowledged only once the log is durable
    /// past this LSN.
    ///
    /// Panics if `id` is not held.
    pub fn read_upto(&self, id: SnapshotId) -> Option<Lsn> {
        *self
            .state
            .lock(&self.rt)
            .reads
            .get(&id)
            .unwrap_or_else(|| panic!("{id:?} is not held"))
    }

    /// Returns whether the version whose header holds `xmin` and `xmax` is visible to `reader`
    /// at `snapshot`.
    pub fn sees(
        &self,
        pager: &Pager<R>,
        snapshot: &Snapshot,
        reader: Option<Xid>,
        xmin: u64,
        xmax: u64,
    ) -> Result<bool, PageError> {
        snapshot.sees(reader, xmin, xmax, |x| self.status(pager, x))
    }

    /// Run `f` on the snapshots and replica reports that bound reclamation.
    pub fn with_holds<T>(&self, f: impl FnOnce(&mut Holds) -> T) -> T {
        f(&mut self.state.lock(&self.rt).holds)
    }

    /// Returns the watermark of `heap`: the oldest snapshot xmin any snapshot or replica holding
    /// `heap` has, or the next id when none holds it.
    pub fn watermark(&self, heap: u32) -> Xid {
        let st = self.state.lock(&self.rt);
        st.holds.watermark(heap, st.next)
    }

    /// Truncate the commit log below `bound`.
    ///
    /// Preconditions: those of [`CommitLog::truncate`].
    ///
    /// Panics if a transaction below `bound` is running, or if `bound` is not assigned yet.
    pub fn truncate(&self, pager: &Pager<R>, bound: Xid) -> Result<(), Error> {
        self.admit()?;
        let st = self.state.lock(&self.rt);
        assert!(bound <= st.next, "{bound:?} is not assigned yet");
        assert!(
            st.running.first().is_none_or(|&r| r >= bound),
            "a transaction below {bound:?} is running"
        );
        let truncated = self.clog.write(&self.rt).truncate(pager, bound);
        self.checked(truncated)
    }
}
