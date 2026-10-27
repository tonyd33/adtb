use std::collections::{BTreeMap, BTreeSet};
use std::sync::{OnceLock, RwLockReadGuard};
use std::time::{Duration, SystemTime};

use adtb_sys::lock::{Lock, RwLock};

use crate::btree;
use crate::runtime::Runtime;
use crate::storage::{Lsn, PageError, Pager, Pid, Position};

use super::past::{Commit, Marker, Past};
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
    /// The LSN of the latest commit record, including retained commits recovered at open.
    last_commit: Option<Lsn>,
    /// The recorded time of the latest commit, in nanoseconds since the Unix epoch.
    last_time: u64,
    /// The log was durable below this LSN when the commit log was opened, every commit before
    /// the open among it.
    opened: Lsn,
    /// The `last_commit` and `last_time` of each held snapshot when it was taken.
    reads: BTreeMap<SnapshotId, (Option<Lsn>, u64)>,
    /// The time of each marker a read of the past reads at, with the number of reads at it.
    pins: BTreeMap<u64, usize>,
    /// The length of the retention window.
    window: Duration,
}

/// Why a read of the past is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PastError {
    /// The position is outside the retention window or after the reader's snapshot. `oldest` is
    /// the oldest position a read may name, if any.
    Unavailable { oldest: Option<Position> },
    /// A page of the record of the window is damaged.
    Damaged(btree::Error),
}

/// Returns `t` in nanoseconds since the Unix epoch.
fn nanos(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
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
///
/// Clones are handles on one set of transactions.
#[derive(Debug)]
pub struct Transactions<R: Runtime>(std::sync::Arc<Shared<R>>);

impl<R: Runtime> Clone for Transactions<R> {
    fn clone(&self) -> Self {
        Transactions(self.0.clone())
    }
}

impl<R: Runtime> std::ops::Deref for Transactions<R> {
    type Target = Shared<R>;

    fn deref(&self) -> &Shared<R> {
        &self.0
    }
}

/// The state every handle on one set of transactions shares.
#[derive(Debug)]
pub struct Shared<R: Runtime> {
    rt: R,
    clog: RwLock<CommitLog>,
    state: Lock<State>,
    quarantine: OnceLock<PageError>,
    history: OnceLock<Past>,
    /// The first damage found keeping a marker; every read of the past fails with it after.
    lost: OnceLock<btree::Error>,
    /// The end of the log once the latest marker was kept, held while one is being kept.
    marked: Lock<Lsn>,
}

impl<R: Runtime> Transactions<R> {
    pub fn create(rt: R, pager: &Pager<R>) -> Result<Transactions<R>, PageError> {
        Ok(Transactions::over(rt, pager, CommitLog::create(pager)?))
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
        let txns = Transactions::over(rt, pager, clog);
        if let Err(e) = unresolved {
            txns.damaged(e);
        }
        Ok(txns)
    }

    fn over(rt: R, pager: &Pager<R>, clog: CommitLog) -> Transactions<R> {
        let state = State {
            next: clog.reserved(),
            running: BTreeSet::new(),
            holds: Holds::default(),
            last_commit: None,
            last_time: 0,
            opened: pager.durable(),
            reads: BTreeMap::new(),
            pins: BTreeMap::new(),
            window: Duration::ZERO,
        };
        Transactions(std::sync::Arc::new(Shared {
            rt,
            clog: RwLock::new(clog),
            state: Lock::new(state),
            quarantine: OnceLock::new(),
            history: OnceLock::new(),
            lost: OnceLock::new(),
            marked: Lock::new(Lsn(0)),
        }))
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

    /// Commit `x` and record its time when history is kept.
    ///
    /// Fails, leaving `x` running, if the database is quarantined or the record of the window is
    /// damaged.
    ///
    /// Panics if `x` is not running.
    pub fn commit(&self, pager: &Pager<R>, x: Xid) -> Result<Lsn, Error> {
        self.admit()?;
        let mut st = self.state.lock(&self.rt);
        assert!(st.running.contains(&x), "{x:?} is not running");
        let time = (nanos(self.rt.wall()) / 1000 * 1000).max(st.last_time);
        if let Some(past) = self.history.get() {
            past.record_commit(pager, x, time).map_err(Error::History)?;
        }
        let committed = self.clog.read(&self.rt).commit_at(pager, x, time);
        let lsn = self.checked(committed)?;
        st.running.remove(&x);
        st.last_commit = Some(lsn);
        st.last_time = time;
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
        let read = (st.last_commit, st.last_time);
        st.reads.insert(id, read);
        Ok((id, snapshot))
    }

    pub fn release(&self, id: SnapshotId) {
        let mut st = self.state.lock(&self.rt);
        st.holds.release(id);
        st.reads.remove(&id);
    }

    /// Returns the LSN of the commit record of the latest commit snapshot `id` may read, or
    /// `None` when no commit position is known. A transaction reading at `id` is acknowledged
    /// only once the log is durable past this LSN.
    ///
    /// Panics if `id` is not held.
    pub fn read_upto(&self, id: SnapshotId) -> Option<Lsn> {
        self.state
            .lock(&self.rt)
            .reads
            .get(&id)
            .unwrap_or_else(|| panic!("{id:?} is not held"))
            .0
    }

    /// Log a safe snapshot marker for `snapshot`, held as `id`, covering every commit it reads.
    /// Returns the marker's LSN.
    ///
    /// Precondition: the serializer found the snapshot safe.
    ///
    /// Panics if `id` is not held.
    pub fn log_safe_snapshot(&self, pager: &Pager<R>, id: SnapshotId, snapshot: &Snapshot) -> Lsn {
        let st = self.state.lock(&self.rt);
        let read = st
            .reads
            .get(&id)
            .unwrap_or_else(|| panic!("{id:?} is not held"));
        let at = pager.position(read.0.unwrap_or(st.opened));
        let time = read.1;
        drop(st);
        let lsn = pager.log_safe_snapshot(at, &snapshot.encode());
        if self.history.get().is_some() {
            let marker = Marker {
                at,
                time,
                snapshot: snapshot.clone(),
            };
            self.record_marker(pager, &marker);
        }
        lsn
    }

    /// Publish a retained commit position and a safe snapshot when no writer is running.
    ///
    /// Precondition: `xid` committed at `lsn` and no later commit has run.
    pub fn publish_commit(&self, pager: &Pager<R>, xid: Xid, lsn: Lsn) {
        let st = self.state.lock(&self.rt);
        let Some(past) = self.history.get() else {
            return;
        };
        assert_eq!(
            Some(lsn),
            st.last_commit,
            "the published commit is the latest commit"
        );
        let at = pager.position(lsn);
        if let Err(e) = past.record_position(
            pager,
            Commit {
                xid,
                at,
                time: st.last_time,
            },
        ) {
            self.lost.get_or_init(|| e);
        }
        if st.window.is_zero() || !st.running.is_empty() {
            return;
        }
        // A snapshot with no running writers is safe from future serialization conflicts.
        let marker = Marker {
            at,
            time: st.last_time,
            snapshot: Snapshot::new(st.next, st.next, Vec::new()),
        };
        pager.log_safe_snapshot(marker.at, &marker.snapshot.encode());
        self.record_marker(pager, &marker);
    }

    fn record_marker(&self, pager: &Pager<R>, marker: &Marker) {
        let mut end = self.marked.lock(&self.rt);
        if let Err(e) = self
            .history
            .get()
            .expect("history is kept")
            .record_marker(pager, marker)
        {
            self.lost.get_or_init(|| e);
        }
        *end = pager.appended();
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
    /// Keep history in `past` from now on, for a retention window of `window`: record the time
    /// of every commit and keep every safe snapshot marker logged.
    ///
    /// Postconditions: every commit from now on records a time no less than the latest `past`
    /// records, whatever the clock reads.
    ///
    /// Fails if a page of `past` is damaged.
    ///
    /// Panics if history is kept already.
    pub fn keep_history(
        &self,
        pager: &Pager<R>,
        past: Past,
        window: Duration,
    ) -> Result<(), Error> {
        let latest = past.latest_commit(pager).map_err(Error::History)?;
        let commits = past.commits(pager).map_err(Error::History)?;
        let mut st = self.state.lock(&self.rt);
        if st.last_commit.is_none() {
            st.last_commit = commits.last().map(|c| c.at.lsn);
        }
        st.last_time = st.last_time.max(latest);
        st.window = window;
        assert!(self.history.set(past).is_ok(), "history is kept already");
        Ok(())
    }

    /// Make the retention window `window` long from now on.
    pub fn set_window(&self, window: Duration) {
        self.state.lock(&self.rt).window = window;
    }

    /// Returns the record of the retention window, if history is kept.
    pub fn history(&self) -> Option<&Past> {
        self.history.get()
    }

    /// Returns the least commit time the retention window keeps now: the window's length before
    /// the latest of now and the latest commit, or the time of the oldest marker a read of the
    /// past reads at, whichever is less. Returns `None` if history is not kept.
    pub fn cutoff(&self) -> Option<u64> {
        self.history.get()?;
        let st = self.state.lock(&self.rt);
        Some(self.cutoff_in(&st))
    }

    fn cutoff_in(&self, st: &State) -> u64 {
        let now = nanos(self.rt.wall()).max(st.last_time);
        let window = u64::try_from(st.window.as_nanos()).unwrap_or(u64::MAX);
        let edge = now.saturating_sub(window);
        st.pins.keys().next().map_or(edge, |&p| p.min(edge))
    }

    /// Returns whether the retention window bounded by `cutoff` keeps the versions `xmax`
    /// deleted: `xmax` committed at or after `cutoff`. A record found malformed keeps them, and
    /// every read of the past fails after.
    pub fn retains(
        &self,
        pager: &Pager<R>,
        xmax: Xid,
        cutoff: Option<u64>,
    ) -> Result<bool, PageError> {
        let (Some(past), Some(cutoff)) = (self.history.get(), cutoff) else {
            return Ok(false);
        };
        match past.committed_at(pager, xmax) {
            Ok(t) => Ok(t.is_some_and(|t| t >= cutoff)),
            Err(btree::Error::Page(e)) => Err(e),
            Err(e) => {
                self.lost.get_or_init(|| e);
                Ok(true)
            }
        }
    }

    /// Returns the marker a read of the past at `at` reads, held so that what it sees stays until
    /// [`Transactions::unpin`]: the latest marker at or before `at`, durable before it returns.
    ///
    /// Fails with [`PastError::Unavailable`] when `at` is after `upto` or no marker within the
    /// window is at or before it, and with [`PastError::Damaged`] when the record is damaged or
    /// a marker could not be kept.
    pub fn pin(&self, pager: &Pager<R>, at: Position, upto: Position) -> Result<Marker, PastError> {
        let found = self.pin_with(pager, |past| {
            if at > upto {
                return Ok(None);
            }
            past.marker_at(pager, at)
        });
        found?.map_or_else(|| Err(self.unavailable(pager)), Ok)
    }

    /// Returns the marker a read of the past at `time` reads, held as [`Transactions::pin`] holds
    /// one: the latest marker whose time is at or before `time`.
    pub fn pin_time(
        &self,
        pager: &Pager<R>,
        time: SystemTime,
        upto: Position,
    ) -> Result<Marker, PastError> {
        let found = self.pin_with(pager, |past| {
            let m = past.marker_by_time(pager, nanos(time))?;
            Ok(m.filter(|m| m.at <= upto))
        });
        found?.map_or_else(|| Err(self.unavailable(pager)), Ok)
    }

    fn pin_with(
        &self,
        pager: &Pager<R>,
        find: impl FnOnce(&Past) -> Result<Option<Marker>, btree::Error>,
    ) -> Result<Option<Marker>, PastError> {
        let Some(past) = self.history.get() else {
            return Ok(None);
        };
        if let Some(e) = self.lost.get() {
            return Err(PastError::Damaged(*e));
        }
        let found = find(past).map_err(PastError::Damaged)?;
        if found.is_some() {
            let end = *self.marked.lock(&self.rt);
            pager.flush_below(end);
        }
        let mut st = self.state.lock(&self.rt);
        let Some(m) = found.filter(|m| m.time >= self.cutoff_in(&st)) else {
            return Ok(None);
        };
        *st.pins.entry(m.time).or_default() += 1;
        Ok(Some(m))
    }

    fn unavailable(&self, pager: &Pager<R>) -> PastError {
        let Some(past) = self.history.get() else {
            return PastError::Unavailable { oldest: None };
        };
        let Some(cutoff) = self.cutoff() else {
            return PastError::Unavailable { oldest: None };
        };
        match past.markers(pager) {
            Ok(ms) => PastError::Unavailable {
                oldest: ms.iter().rev().find(|m| m.time >= cutoff).map(|m| m.at),
            },
            Err(e) => PastError::Damaged(e),
        }
    }

    /// Release the hold [`Transactions::pin`] took on `marker`.
    ///
    /// Panics if `marker` is not held.
    pub fn unpin(&self, marker: &Marker) {
        let mut st = self.state.lock(&self.rt);
        let n = st
            .pins
            .get_mut(&marker.time)
            .unwrap_or_else(|| panic!("no read of the past holds the marker at {:?}", marker.at));
        *n -= 1;
        if *n == 0 {
            st.pins.remove(&marker.time);
        }
    }
}
