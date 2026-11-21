//! Begin transactions and end them once, by commit, abort or drop.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::sync::Arc;
#[cfg(any(test, feature = "testing"))]
use std::sync::atomic::Ordering;

use adtb_sys::lock::Ranked;

use crate::mvcc::{self, Access, Begun, Hold, Limits, Scope, SerialId, Serializer};
use crate::mvcc::{Transactions, Waits};
use crate::ranks;
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::{Error, Held, Session, Txn, vacuums};

pub(super) trait Release: Send + Sync {
    /// Release what `held` has left, aborting its id when it still runs. Returns the abort
    /// record's failure. A drop on a panicking thread records no abort, and a drop on a
    /// panicking thread or after the pager closes logs no safe snapshot marker.
    fn release(&self, held: &mut Held, dropped: bool) -> Result<(), mvcc::Error>;

    /// Count a transaction ended by being dropped.
    fn count_drop(&self);

    /// Count an abort record that failed.
    fn count_unrecorded(&self);
}

impl<R: Runtime> Session<R> {
    pub(super) fn new(
        rt: R,
        pager: Pager<R>,
        txns: Arc<Transactions<R>>,
        limits: Limits,
    ) -> Arc<Session<R>> {
        let ser = txns.serializer().clone();
        ser.lock(&rt).limit(limits);
        Arc::new(Session {
            waits: Waits::new(rt.clone()),
            rt,
            pager,
            txns,
            ser,
            readers: Ranked::new(ranks::READERS, BTreeMap::new()),
            dead: Ranked::new(ranks::DEAD, vacuums::Dead::default()),
            steps: Arc::default(),
            #[cfg(any(test, feature = "testing"))]
            dropped: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// Begin a read-write transaction.
    pub(super) fn begin(self: &Arc<Self>) -> Result<Txn, Error> {
        let begun = self.txns.begin(&self.pager)?;
        let xid = begun.xid();
        let tx = {
            let mut ser = self.ser.lock(&self.rt);
            let hold = self.txns.take(Scope::Every)?;
            let sid = ser.begin(Access::ReadWrite(xid));
            self.txn(Some(begun), None, sid, hold)
        };
        self.waits.started(xid);
        Ok(tx)
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
            taken: hold.taken(),
            written: RefCell::default(),
            held: Held {
                begun,
                serial: Some(sid),
                reader,
                hold: Some(hold),
                waits: xid,
                changed: RefCell::default(),
                failed: Cell::new(false),
                ending: false,
                session: self.clone(),
            },
        }
    }

    /// Log a safe snapshot marker for every read-only transaction `ser` has found safe.
    pub(super) fn log_safe(&self, ser: &mut Serializer) {
        let readers = self.readers.lock(&self.rt);
        for sid in ser.take_safe() {
            let (taken, snapshot) = &readers[&sid];
            self.txns.log_safe_snapshot(&self.pager, *taken, snapshot);
        }
    }

    /// Count the versions `changed` made dead, as [`vacuums::Dead::add`] does.
    pub(super) fn count_dead(&self, changed: &vacuums::Changed, committed: bool) {
        self.dead.lock(&self.rt).add(changed, committed);
    }
}

impl<R: Runtime> Release for Session<R> {
    fn release(&self, held: &mut Held, dropped: bool) -> Result<(), mvcc::Error> {
        let unwinding = dropped && std::thread::panicking();
        let committed = held.serial.is_none();
        let mut recorded = Ok(());
        if held.begun.is_some()
            || held.serial.is_some()
            || held.reader.is_some()
            || held.hold.is_some()
        {
            let mut ser = self.ser.lock(&self.rt);
            match held.begun.take() {
                Some(b) if !unwinding => {
                    recorded = self.txns.abort(&self.pager, b).map(drop);
                }
                begun => drop(begun),
            }
            if let Some(sid) = held.serial.take() {
                ser.aborted(sid);
                if !dropped || (!unwinding && !self.pager.is_closed()) {
                    self.log_safe(&mut ser);
                }
            }
            if let Some(sid) = held.reader.take() {
                self.readers.lock(&self.rt).remove(&sid);
            }
            drop(held.hold.take());
        }
        if let Some(x) = held.waits.take() {
            self.waits.ended(x);
        }
        let changed = held.changed.take();
        if !changed.is_empty() {
            self.count_dead(&changed, committed);
        }
        recorded
    }

    fn count_drop(&self) {
        #[cfg(any(test, feature = "testing"))]
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }

    fn count_unrecorded(&self) {
        self.txns.count_unrecorded();
    }
}

impl Txn {
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
    /// Release what is left, as a drop does. Returns the abort record's failure, uncounted.
    pub(super) fn close(mut self) -> Result<(), mvcc::Error> {
        let session = self.session.clone();
        session.release(&mut self, false)
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        let session = self.session.clone();
        if !self.ending && !self.failed.get() {
            session.count_drop();
        }
        if session.release(self, true).is_err() {
            session.count_unrecorded();
        }
    }
}
