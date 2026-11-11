use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, RwLockReadGuard, Weak};
use std::time::{Duration, SystemTime};

use adtb_sys::lock::{Lock, RwLock};

use crate::btree;
use crate::runtime::Runtime;
use crate::storage::{Damage, Lsn, PageError, Pager, Pid, Position, Quarantine};

use super::past::{Commit, Marker, Past};
use super::{CommitLog, Error, Holds, Scope, Snapshot, SnapshotId, Status, StatusError, Xid};

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

impl State {
    fn resolved(&self) -> Xid {
        let first = self
            .running
            .first()
            .into_iter()
            .chain(self.unresolved.first());
        first.min().copied().unwrap_or(self.next)
    }
}

#[derive(Debug)]
struct State {
    next: Xid,
    running: BTreeSet<Xid>,
    /// The ids out of `running` whose abort the commit log does not record.
    unresolved: BTreeSet<Xid>,
    holds: Holds,
    /// The LSN of the latest commit record, including retained commits recovered at open.
    last_commit: Option<Lsn>,
    /// The recorded time of the latest commit, in nanoseconds since the Unix epoch.
    last_time: u64,
    /// The log was durable below this LSN when the commit log was opened, every commit before
    /// the open among it.
    opened: Lsn,
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
/// Returns the damage recorded of a page of `clog` that it does not settle.
fn recorded_damage<R: Runtime>(pager: &Pager<R>, clog: &CommitLog) -> Option<PageError> {
    let recorded = pager.recorded();
    let mut pages = vec![clog.root()];
    pages.extend(clog.directory_pages());
    pages.extend(clog.status_pages().values());
    pages.into_iter().find_map(|pid| {
        let damage = *recorded.pages.get(&pid)?;
        let settled = matches!(damage, Damage::Settled { .. });
        (!settled).then_some(PageError { pid, damage })
    })
}

/// Once any commit log page fails verification the database is quarantined: from then on every
/// call that starts, ends or reads for a transaction fails with [`Error::Corrupt`], naming the
/// first page found damaged. The commit log stays readable through [`Transactions::commit_log`]
/// for verification and repair.
///
/// Clones are handles on one set of transactions.
#[derive(Debug)]
pub struct Transactions<R: Runtime>(Arc<Shared<R>>);

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
    pager: Pager<R>,
    clog: RwLock<CommitLog>,
    state: Lock<State>,
    history: OnceLock<Past>,
    /// The first damage found keeping a marker; every read of the past fails with it after.
    lost: OnceLock<btree::Error>,
    /// The end of the log once the latest marker was kept, held while one is being kept.
    marked: Lock<Lsn>,
    /// The abort records that failed or were skipped on a panicking thread, of transactions
    /// dropped, of failed commits, and of transactions a call abandons when it fails or releases
    /// them open.
    unrecorded: AtomicUsize,
}

impl<R: Runtime> Transactions<R> {
    pub fn create(rt: R, pager: &Pager<R>) -> Result<Transactions<R>, PageError> {
        Ok(Transactions::over(rt, pager, CommitLog::create(pager)?))
    }

    /// Open the commit log rooted at `root` in a recovered pager.
    ///
    /// Postconditions:
    ///
    /// - Every id assigned before the crash reads as committed exactly when its status page or a
    ///   commit record of the retained log says so, and as aborted otherwise.
    /// - No id assigned before the crash is assigned again.
    /// - If a status page is damaged, or the retained log is too damaged to settle an id in
    ///   progress or does not prove the status of one on a status page that lost a write, the
    ///   database is quarantined.
    /// - A file with a damaged status page, a log that cannot settle an id in progress, or the
    ///   database's mark has nothing written, and an id running at the crash still reads as in
    ///   progress.
    ///
    /// Fail if the commit log's root or a directory page is damaged, or a status page cannot
    /// be read.
    pub fn open(rt: R, pager: &Pager<R>, root: Pid) -> Result<Transactions<R>, PageError> {
        let mut clog = CommitLog::load(pager, root)?;
        let damage = if pager.is_quarantined(Quarantine::Database) {
            clog.unresolved(pager).err()
        } else {
            clog.settle_unresolved(pager).err()
        };
        if let Some(e) = damage.filter(|e| e.damage == Damage::Unreadable) {
            return Err(e);
        }
        let damage = damage.or_else(|| recorded_damage(pager, &clog));
        let txns = Transactions::over(rt, pager, clog);
        if let Some(e) = damage {
            txns.damaged(e);
        }
        Ok(txns)
    }

    /// Returns the transactions layer of a file whose commit log cannot be read, refusing every
    /// transaction with the first damage the pager keeps, `e` when it keeps none.
    pub fn refusing(rt: R, pager: &Pager<R>, e: PageError) -> Transactions<R> {
        let txns = Transactions::over(rt, pager, CommitLog::unread(pager));
        txns.damaged(e);
        txns
    }

    /// Returns whether `x` committed, by its status or, when its status page is damaged, by
    /// `logged`, the commit records of the retained log. Returns `None` when neither settles it.
    ///
    /// A commit log that reserves nothing, as an unread one, has only `logged` settle `x`. Fail
    /// with [`StatusError::Unreserved`] when `x` is not reserved by a log that reserves some id.
    pub(super) fn settled(
        &self,
        pager: &Pager<R>,
        logged: &BTreeSet<u64>,
        x: Xid,
    ) -> Result<Option<bool>, StatusError> {
        let clog = self.clog.read(&self.rt);
        match clog.status(pager, x) {
            Ok(status) => Ok(Some(status == Status::Committed)),
            Err(StatusError::Page(_)) => Ok(logged.contains(&x.0).then_some(true)),
            Err(StatusError::Unreserved(_)) if clog.reserved() == Xid::FIRST => {
                Ok(logged.contains(&x.0).then_some(true))
            }
            Err(e @ StatusError::Unreserved(_)) => Err(e),
        }
    }

    fn over(rt: R, pager: &Pager<R>, clog: CommitLog) -> Transactions<R> {
        let state = State {
            next: clog.reserved(),
            running: BTreeSet::new(),
            unresolved: BTreeSet::new(),
            holds: Holds::default(),
            last_commit: None,
            last_time: 0,
            opened: pager.durable(),
            pins: BTreeMap::new(),
            window: Duration::ZERO,
        };
        Transactions(Arc::new(Shared {
            rt,
            pager: pager.clone(),
            clog: RwLock::new(clog),
            state: Lock::new(state),
            history: OnceLock::new(),
            lost: OnceLock::new(),
            marked: Lock::new(Lsn(0)),
            unrecorded: AtomicUsize::new(0),
        }))
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn pager(&self) -> &Pager<R> {
        &self.pager
    }

    /// Returns the commit log, held shared until the guard drops.
    pub fn commit_log(&self) -> RwLockReadGuard<'_, CommitLog> {
        self.clog.read(&self.rt)
    }

    /// Returns the id the next transaction gets.
    pub(crate) fn next(&self) -> Xid {
        self.state.lock(&self.rt).next
    }

    pub fn running(&self) -> BTreeSet<Xid> {
        self.state.lock(&self.rt).running.clone()
    }

    /// Returns the least id that is running or unresolved, else the next id: every id below it
    /// has resolved.
    pub fn resolved_below(&self) -> Xid {
        self.state.lock(&self.rt).resolved()
    }

    /// Start a transaction, first reserving more ids and forcing the log when the reservation is
    /// spent. Returns the guard of its id, which is running until the guard is given to
    /// [`Transactions::commit`] or [`Transactions::abort`], or dropped.
    ///
    /// Fails with [`Error::Exhausted`] once every 64-bit id is spent; snapshots and reads go on.
    pub fn begin(&self, pager: &Pager<R>) -> Result<Begun, Error> {
        self.admit()?;
        let mut st = self.state.lock(&self.rt);
        let reserved = self.clog.read(&self.rt).reserved();
        match assign(st.next, reserved) {
            Assign::Next => {}
            Assign::Exhausted => return Err(Error::Exhausted),
            Assign::Reserve(upto) => {
                let reserved = self
                    .clog
                    .write(&self.rt)
                    .reserve(pager, upto, st.resolved());
                self.checked(reserved)?;
            }
        }
        let x = st.next;
        st.next = Xid(x.0 + 1);
        st.running.insert(x);
        let txns: Weak<dyn Release> = Arc::<Shared<R>>::downgrade(&self.0);
        Ok(Begun {
            xid: x,
            ended: false,
            txns,
        })
    }

    /// Commit the transaction of `begun` and record its time when history is kept.
    ///
    /// Fail if the database is quarantined or the record of the window is damaged. The
    /// transaction is then aborted as a dropped guard aborts it, and the commit's failure
    /// returned.
    ///
    /// Panic if `begun` did not begin in these transactions.
    pub fn commit(&self, pager: &Pager<R>, begun: Begun) -> Result<Lsn, Error> {
        let x = self.end(begun);
        let admitted = self.admit();
        let mut st = self.state.lock(&self.rt);
        let committed = admitted.and_then(|()| self.commit_running(&mut st, pager, x));
        if committed.is_err() {
            self.abandon_locked(&mut st, pager, x);
        }
        committed
    }

    fn commit_running(&self, st: &mut State, pager: &Pager<R>, x: Xid) -> Result<Lsn, Error> {
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

    /// Abort the transaction of `begun`: take its id out of the running transactions, record it
    /// in the commit log and log its abort record.
    ///
    /// Fail if the database is quarantined, or the record fails and quarantines it. The id is
    /// then no longer running and the commit log still says it is in progress.
    ///
    /// Panic if `begun` did not begin in these transactions.
    pub fn abort(&self, pager: &Pager<R>, begun: Begun) -> Result<Lsn, Error> {
        let x = self.end(begun);
        let admitted = self.admit();
        let mut st = self.state.lock(&self.rt);
        self.abort_locked(&mut st, pager, x, admitted)
    }

    /// Count a failed abort record of a transaction a call abandons when it fails or releases it
    /// open.
    pub fn count_unrecorded(&self) {
        self.0.count_unrecorded();
    }

    /// Returns the id of `begun`, ended so that dropping it aborts nothing.
    fn end(&self, mut begun: Begun) -> Xid {
        assert!(
            std::ptr::addr_eq(begun.txns.as_ptr(), Arc::as_ptr(&self.0)),
            "{:?} began in these transactions",
            begun.xid
        );
        begun.ended = true;
        begun.xid
    }

    /// Returns how many abort records failed or were skipped on a panicking thread, of
    /// transactions dropped, of failed commits, and of transactions a call abandons when it fails
    /// or releases them open.
    #[cfg(any(test, feature = "testing"))]
    pub fn unrecorded_aborts(&self) -> usize {
        self.unrecorded.load(Ordering::Relaxed)
    }

    /// Returns the commit log status of `x`.
    ///
    /// Fail with the first damage found once the database is quarantined, quarantining it when
    /// the status page of `x` fails verification, and with [`StatusError::Unreserved`] when `x`
    /// is not reserved.
    pub(super) fn status(&self, pager: &Pager<R>, x: Xid) -> Result<Status, StatusError> {
        if let Some(e) = self.quarantined() {
            return Err(StatusError::Page(e));
        }
        match self.clog.read(&self.rt).status(pager, x) {
            Err(StatusError::Page(e)) => Err(StatusError::Page(self.pager.refuse_whole(e))),
            found => found,
        }
    }

    /// Returns what the commit log records of `x`, whether or not the database is quarantined.
    #[cfg(any(test, feature = "testing"))]
    pub fn recorded(&self, pager: &Pager<R>, x: Xid) -> Result<Status, StatusError> {
        self.commit_log().status(pager, x)
    }

    /// Take a snapshot of the transactions now committed and hold back the heaps of `scope` for
    /// it until the returned [`Hold`] drops.
    ///
    /// Fails if the database is quarantined.
    pub fn take(&self, scope: Scope) -> Result<(Hold, Snapshot), Error> {
        self.admit()?;
        let mut st = self.state.lock(&self.rt);
        let in_progress: Vec<Xid> = st.running.iter().copied().collect();
        let xmin = in_progress.first().copied().unwrap_or(st.next);
        let snapshot = Snapshot::new(xmin, st.next, in_progress);
        let id = st.holds.hold(xmin, scope);
        let taken = Taken {
            upto: st.last_commit,
            time: st.last_time,
        };
        drop(st);
        let txns: Weak<dyn Release> = Arc::<Shared<R>>::downgrade(&self.0);
        let hold = Hold { id, taken, txns };
        Ok((hold, snapshot))
    }

    /// Log a safe snapshot marker for `snapshot`, which `taken` describes, covering every commit
    /// it reads. Returns the marker's LSN.
    ///
    /// Precondition: the serializer found the snapshot safe.
    pub fn log_safe_snapshot(&self, pager: &Pager<R>, taken: Taken, snapshot: &Snapshot) -> Lsn {
        let at = pager.position(
            taken
                .upto
                .unwrap_or_else(|| self.state.lock(&self.rt).opened),
        );
        let time = taken.time;
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

    /// Run `f` on the snapshots and replica reports that bound reclamation.
    #[cfg(any(test, feature = "testing"))]
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
    /// Panic if a transaction below `bound` is running or unresolved, or if `bound` is not
    /// assigned yet.
    #[cfg(any(test, feature = "testing"))]
    pub fn truncate(&self, pager: &Pager<R>, bound: Xid) -> Result<(), Error> {
        self.admit()?;
        let st = self.state.lock(&self.rt);
        assert!(bound <= st.next, "{bound:?} is not assigned yet");
        assert!(
            st.running.first().is_none_or(|&r| r >= bound)
                && st.unresolved.first().is_none_or(|&r| r >= bound),
            "a transaction below {bound:?} is running or unresolved"
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
    /// Fail if a page of `past` is damaged.
    ///
    /// Panic if history is kept already.
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
    /// Fail with [`PastError::Unavailable`] when `at` is after `upto` or no marker within the
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
    /// Panic if `marker` is not held.
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

/// The latest commit a snapshot reads, as it stood when the snapshot was taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Taken {
    upto: Option<Lsn>,
    time: u64,
}

impl Taken {
    /// Returns the LSN of the commit record of the latest commit the snapshot may read, or `None`
    /// when no commit position is known. A transaction reading at the snapshot is acknowledged
    /// only once the log is durable past this LSN.
    pub fn upto(self) -> Option<Lsn> {
        self.upto
    }
}

impl<R: Runtime> Shared<R> {
    /// Returns the damage that quarantined the database, if any.
    fn quarantined(&self) -> Option<PageError> {
        self.pager.whole_cause()
    }

    fn admit(&self) -> Result<(), Error> {
        if self.pager.is_closed() {
            return Err(Error::Closed);
        }
        self.quarantined()
            .map_or(Ok(()), |e| Err(Error::Corrupt(e)))
    }

    /// Quarantine the database for `e`, keeping the first damage found. Returns the failure.
    fn damaged(&self, e: PageError) -> Error {
        Error::Corrupt(self.pager.refuse_whole(e))
    }

    fn checked<T>(&self, r: Result<T, PageError>) -> Result<T, Error> {
        r.map_err(|e| self.damaged(e))
    }

    /// Take `x` out of the running transactions and, once `admitted`, record its abort in the
    /// commit log. Keep `x` unresolved when nothing is recorded.
    ///
    /// Panic if `x` is not running.
    fn abort_locked(
        &self,
        st: &mut State,
        pager: &Pager<R>,
        x: Xid,
        admitted: Result<(), Error>,
    ) -> Result<Lsn, Error> {
        assert!(st.running.remove(&x), "{x:?} is not running");
        let recorded = admitted.and_then(|()| {
            let aborted = self.clog.read(&self.rt).abort(pager, x);
            self.checked(aborted)
        });
        if recorded.is_err() {
            st.unresolved.insert(x);
        }
        recorded
    }

    /// Abort `x` as [`Shared::abort_locked`] does, counting a failed record.
    fn abandon_locked(&self, st: &mut State, pager: &Pager<R>, x: Xid) {
        if self.abort_locked(st, pager, x, self.admit()).is_err() {
            self.count_unrecorded();
        }
    }

    fn count_unrecorded(&self) {
        self.unrecorded.fetch_add(1, Ordering::Relaxed);
    }
}

trait Release: Send + Sync {
    fn release(&self, id: SnapshotId);

    /// Abort `x` best effort: take it out of the running transactions and, unless the thread is
    /// panicking, attempt its record.
    fn abandon(&self, x: Xid);
}

impl<R: Runtime> Release for Shared<R> {
    fn release(&self, id: SnapshotId) {
        self.state.lock(&self.rt).holds.release(id);
    }

    fn abandon(&self, x: Xid) {
        let mut st = self.state.lock(&self.rt);
        if std::thread::panicking() {
            st.running.remove(&x);
            st.unresolved.insert(x);
            self.count_unrecorded();
        } else {
            self.abandon_locked(&mut st, &self.pager, x);
        }
    }
}

/// A running transaction's id, aborted best effort if it drops before
/// [`Transactions::commit`] or [`Transactions::abort`] takes it: its id always leaves the
/// running transactions, and the abort is recorded in the commit log unless the thread is
/// panicking or the pager is closed. An abort left unrecorded is counted, not returned.
///
/// Dropping it takes the lock over running transactions, then the commit log shared, then the
/// status page of its id as the pager logs the abort.
#[must_use = "dropping it aborts the transaction"]
pub struct Begun {
    xid: Xid,
    ended: bool,
    /// Empty once the transactions are gone, when nothing is left to abort.
    txns: Weak<dyn Release>,
}

impl Begun {
    pub fn xid(&self) -> Xid {
        self.xid
    }
}

impl std::fmt::Debug for Begun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Begun")
            .field("xid", &self.xid)
            .field("ended", &self.ended)
            .finish_non_exhaustive()
    }
}

impl Drop for Begun {
    fn drop(&mut self) {
        if !self.ended
            && let Some(txns) = self.txns.upgrade()
        {
            txns.abandon(self.xid);
        }
    }
}

/// A snapshot's hold on the heaps of its scope, released when it drops.
pub struct Hold {
    id: SnapshotId,
    taken: Taken,
    /// Empty once the transactions are gone, when nothing is left to release.
    txns: Weak<dyn Release>,
}

impl Hold {
    /// Returns the latest commit the held snapshot reads.
    pub fn taken(&self) -> Taken {
        self.taken
    }
}

impl std::fmt::Debug for Hold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hold")
            .field("id", &self.id)
            .field("taken", &self.taken)
            .finish_non_exhaustive()
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        if let Some(txns) = self.txns.upgrade() {
            txns.release(self.id);
        }
    }
}
