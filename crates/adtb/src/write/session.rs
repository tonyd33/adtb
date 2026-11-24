//! Begin transactions and end them once, by commit, abort or drop.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::sync::Arc;
#[cfg(any(test, feature = "testing"))]
use std::sync::atomic::Ordering;

use adtb_lang::types::RelId;
use adtb_sys::lock::Ranked;

use crate::mvcc::{self, Access, Begun, Hold, Limits, Scope, SerialId, Serializer};
use crate::mvcc::{Transactions, Waits, Xid};
use crate::ranks;
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::{Error, Held, Session, Txn, vacuums};

/// How a transaction's entry in the serializer ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SerialEnd {
    Committed,
    /// Aborted by a call that ends it.
    Closed,
    /// Aborted as it drops.
    Dropped,
}

pub(super) trait Release: Send + Sync {
    /// Release what `held` has left, recording its id's abort when it still runs. Returns the
    /// abort record's failure.
    fn close(&self, held: &mut Held) -> Result<(), mvcc::Error>;

    /// Release what `held` has left as it drops, its id's abort left to its `Begun`'s drop.
    fn release(&self, held: &mut Held);

    /// Count a transaction ended by being dropped.
    fn count_drop(&self);
}

impl<R: Runtime> Session<R> {
    pub(super) fn new(
        rt: R,
        pager: Pager<R>,
        txns: Arc<Transactions<R>>,
        limits: Limits,
        heaps: vacuums::RelHeaps,
    ) -> Arc<Session<R>> {
        let ser = txns.serializer().clone();
        ser.lock(&rt).limit(limits);
        Arc::new(Session {
            steps: pager.steps().clone(),
            rt,
            pager,
            txns,
            ser,
            readers: Ranked::new(ranks::READERS, BTreeMap::new()),
            dead: Ranked::new(ranks::DEAD, vacuums::Dead::default()),
            heaps,
            #[cfg(any(test, feature = "testing"))]
            dropped: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// Begin a read-write transaction.
    pub(super) fn begin(self: &Arc<Self>) -> Result<Txn, Error> {
        let begun = self.txns.begin(&self.pager)?;
        let xid = begun.xid();
        let mut ser = self.ser.lock(&self.rt);
        let hold = self.txns.take(Scope::Every)?;
        let sid = ser.begin(Access::ReadWrite(xid));
        Ok(self.txn(Some(begun), None, sid, hold))
    }

    /// Begin a read-only transaction, `deferrable` or not.
    pub(super) fn begin_reader(self: &Arc<Self>, deferrable: bool) -> Result<Txn, Error> {
        let (sid, hold) = {
            let mut ser = self.ser.lock(&self.rt);
            let hold = self.txns.take(Scope::Every)?;
            let sid = ser.begin(if deferrable {
                Access::Deferrable
            } else {
                Access::ReadOnly
            });
            self.readers
                .lock(&self.rt)
                .insert(sid, (hold.taken(), hold.snapshot().clone()));
            self.log_safe(&mut ser);
            (sid, hold)
        };
        Ok(self.txn(None, Some(sid), sid, hold))
    }

    fn txn(
        self: &Arc<Self>,
        begun: Option<Begun>,
        reader: Option<SerialId>,
        sid: SerialId,
        hold: Hold,
    ) -> Txn {
        let xid = begun.as_ref().map(Begun::xid);
        Txn {
            xid,
            sid,
            began: self.rt.now(),
            taken: hold.taken(),
            written: RefCell::default(),
            held: Held {
                begun,
                serial: Some(sid),
                reader,
                hold: Some(hold),
                changed: RefCell::default(),
                failed: Cell::new(false),
                ending: false,
                session: self.clone(),
            },
        }
    }

    /// Returns the waits on running transactions, which the transactions start and end.
    pub(super) fn waits(&self) -> &Waits<R> {
        self.txns.waits()
    }

    /// End what `held` holds of the serializer under its lock `ser`, in order: its entry by
    /// `end`, the safe snapshot markers due unless `end` is a drop on a panicking thread or after
    /// the pager closed, its reader's snapshot in the readers, then its hold.
    pub(super) fn end_serial(&self, ser: &mut Serializer, held: &mut Held, end: SerialEnd) {
        if let Some(sid) = held.serial.take() {
            if end == SerialEnd::Committed {
                ser.committed(sid);
            } else {
                ser.aborted(sid);
            }
            let quiet =
                end == SerialEnd::Dropped && (std::thread::panicking() || self.pager.is_closed());
            if !quiet {
                self.log_safe(ser);
            }
        }
        if let Some(sid) = held.reader.take() {
            self.readers.lock(&self.rt).remove(&sid);
        }
        drop(held.hold.take());
    }

    /// Log a safe snapshot marker for every read-only transaction `ser` has found safe.
    pub(super) fn log_safe(&self, ser: &mut Serializer) {
        let readers = self.readers.lock(&self.rt);
        for sid in ser.take_safe() {
            let (taken, snapshot) = &readers[&sid];
            self.txns.log_safe_snapshot(&self.pager, *taken, snapshot);
        }
    }

    /// Count the versions `changed` made dead, as [`vacuums::Dead::add`] does, and unpark the
    /// background worker if it is idle and a relation is now due.
    pub(super) fn count_dead(&self, changed: &vacuums::Changed, committed: bool) {
        if changed.is_empty() {
            return;
        }
        let mut d = self.dead.lock(&self.rt);
        d.add(changed, committed);
        let due = (self.heaps.iter())
            .filter(|(_, heaps)| heaps.iter().any(|(h, _)| changed.contains_key(h)))
            .any(|(rel, _)| self.dead_due(&d, *rel));
        if let Some(t) = due.then(|| d.wake()).flatten() {
            self.rt.unpark(t);
        }
    }

    /// Returns whether `rel` is due a vacuum by the dead versions `d` counts.
    pub(super) fn dead_due(&self, d: &vacuums::Dead, rel: RelId) -> bool {
        let heaps = self.heaps.get(&rel).map_or(&[][..], Vec::as_slice);
        d.due(rel, heaps, || self.horizon(rel))
    }

    /// Returns the least watermark of the heaps of `rel`.
    pub(super) fn horizon(&self, rel: RelId) -> Xid {
        let heaps = self.heaps.get(&rel).map_or(&[][..], Vec::as_slice);
        (heaps.iter())
            .map(|(_, id)| self.txns.watermark(*id))
            .min()
            .unwrap_or_else(|| self.txns.next())
    }
}

impl<R: Runtime> Session<R> {
    /// Release what `held` has left, ending its id, when it still runs, by `abort` under the
    /// serializer's lock before its entry ends by `end`. Returns what `abort` returned.
    fn end_held<T>(
        &self,
        held: &mut Held,
        end: SerialEnd,
        abort: impl FnOnce(Begun) -> T,
    ) -> Option<T> {
        let committed = held.serial.is_none();
        let mut aborted = None;
        if held.begun.is_some()
            || held.serial.is_some()
            || held.reader.is_some()
            || held.hold.is_some()
        {
            let mut ser = self.ser.lock(&self.rt);
            aborted = held.begun.take().map(abort);
            self.end_serial(&mut ser, held, end);
        }
        self.count_dead(&held.changed.take(), committed);
        aborted
    }
}

impl<R: Runtime> Release for Session<R> {
    fn close(&self, held: &mut Held) -> Result<(), mvcc::Error> {
        let abort = |b| self.txns.abort(&self.pager, b).map(drop);
        self.end_held(held, SerialEnd::Closed, abort)
            .unwrap_or(Ok(()))
    }

    fn release(&self, held: &mut Held) {
        self.end_held(held, SerialEnd::Dropped, drop);
    }

    fn count_drop(&self) {
        #[cfg(any(test, feature = "testing"))]
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

impl Txn {
    /// Returns the transaction as keyed catalog storage writes in it, `None` for a read-only
    /// one.
    pub fn catalog_writer(&self) -> Option<crate::catalog::Writer<'_>> {
        (self.xid).map(|x| crate::catalog::Writer::new(self.hold(), x))
    }

    /// Returns the hold on the transaction's snapshot.
    pub(super) fn hold(&self) -> &Hold {
        self.held
            .hold
            .as_ref()
            .expect("a transaction holds its snapshot until its end takes it")
    }

    /// Returns `done`, marking the transaction failed when it is an `Err`.
    pub(super) fn noted<T, E>(&self, done: Result<T, E>) -> Result<T, E> {
        if done.is_err() {
            self.held.failed.set(true);
        }
        done
    }

    /// Returns what the transaction holds, taken by its end.
    pub(super) fn end(self) -> Held {
        let mut held = self.held;
        held.ending = true;
        held
    }
}

impl Held {
    /// Release what is left and record its id's abort, also on a panicking thread or once the
    /// pager closes, logging the safe snapshot markers its end makes due. Count no end by drop.
    /// Returns the abort record's failure, uncounted.
    pub(super) fn close(mut self) -> Result<(), mvcc::Error> {
        let session = self.session.clone();
        session.close(&mut self)
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        let session = self.session.clone();
        if !self.ending && !self.failed.get() {
            session.count_drop();
        }
        session.release(self);
    }
}
