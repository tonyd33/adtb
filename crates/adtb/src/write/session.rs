//! Begin transactions and end them once, by commit, abort or drop.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::sync::Arc;
#[cfg(any(test, feature = "testing"))]
use std::sync::atomic::Ordering;

use adtb_sys::lock::Ranked;

use crate::mvcc::{self, Access, Begun, Hold, Limits, Scope, SerialId, Serializer};
use crate::mvcc::{Transactions, Waits, Xid};
use crate::ranks;
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::{Ended, Error, Held, Left, Session, Txn, vacuums};

/// Release what the end of a transaction has not.
pub(super) trait Abandon: Send + Sync {
    fn abandon(&self, tx: &mut Ended);

    /// Count a transaction ended by being dropped.
    fn count_drop(&self);
}

impl<R: Runtime> Session<R> {
    pub(super) fn new(
        rt: R,
        pager: Pager<R>,
        txns: Arc<Transactions<R>>,
        limits: Limits,
    ) -> Arc<Session<R>> {
        Arc::new(Session {
            waits: Waits::new(rt.clone()),
            rt,
            pager,
            txns,
            ser: Arc::new(Ranked::new(ranks::SERIALIZER, Serializer::new(limits))),
            readers: Ranked::new(ranks::READERS, BTreeMap::new()),
            dead: Ranked::new(ranks::DEAD, vacuums::Dead::default()),
            #[cfg(any(test, feature = "testing"))]
            dropped: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            fail_log: adtb_sys::lock::Lock::default(),
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
            self.txn(Some(begun), sid, hold)
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
        Ok(self.txn(None, sid, hold))
    }

    fn txn(self: &Arc<Self>, begun: Option<Begun>, sid: SerialId, hold: Hold) -> Txn {
        Txn {
            xid: begun.as_ref().map(Begun::xid),
            sid,
            taken: hold.taken(),
            written: RefCell::default(),
            changed: RefCell::default(),
            failed: Cell::new(false),
            held: Some(Held { begun, hold }),
            session: self.clone(),
        }
    }

    /// Log a safe snapshot marker for every read-only transaction `ser` has found safe.
    pub(super) fn log_safe(&self, ser: &mut Serializer) {
        let readers = self.readers.lock(&self.rt);
        #[cfg(test)]
        if let Some(message) = self.fail_log.lock(&self.rt).take() {
            panic!("{message}");
        }
        for sid in ser.take_safe() {
            let (taken, snapshot) = &readers[&sid];
            self.txns.log_safe_snapshot(&self.pager, *taken, snapshot);
        }
    }

    /// Abort `tx` and release what it holds, leaving its dead versions to count.
    ///
    /// Fail if the database is quarantined, the commit log still saying `tx`'s id is in
    /// progress.
    pub(super) fn abort(&self, tx: &mut Ended) -> Result<(), mvcc::Error> {
        let aborted = {
            let mut ser = self.ser.lock(&self.rt);
            let aborted = tx
                .begun
                .take()
                .map(|b| self.txns.abort(&self.pager, b))
                .transpose();
            self.release(&mut ser, tx, true);
            aborted
        };
        self.ended(tx.xid);
        tx.left.waits = false;
        aborted.map(drop)
    }

    /// Release `tx`'s entry in the serializer unless already released, then its reader's
    /// snapshot and its hold, logging the safe snapshots found when `log` is set.
    fn release(&self, ser: &mut Serializer, tx: &mut Ended, log: bool) {
        if tx.left.serial {
            ser.aborted(tx.sid);
            tx.left.serial = false;
            if log {
                self.log_safe(ser);
            }
        }
        self.readers.lock(&self.rt).remove(&tx.sid);
        drop(tx.hold.take());
    }

    /// Wake the waiters of `xid`, a transaction that has ended.
    fn ended(&self, xid: Option<Xid>) {
        if let Some(x) = xid {
            self.waits.ended(x);
        }
    }

    /// Count the versions `changed` made dead, as [`vacuums::Dead::add`] does.
    pub(super) fn count_dead(&self, changed: &vacuums::Changed, committed: bool) {
        self.dead.lock(&self.rt).add(changed, committed);
    }
}

impl<R: Runtime> Abandon for Session<R> {
    fn abandon(&self, tx: &mut Ended) {
        {
            let mut ser = self.ser.lock(&self.rt);
            drop(tx.begun.take());
            let log = !std::thread::panicking() && !self.pager.is_closed();
            self.release(&mut ser, tx, log);
        }
        if tx.left.waits {
            self.ended(tx.xid);
        }
        if tx.left.dead {
            self.count_dead(&tx.changed, tx.left.committed);
        }
    }

    fn count_drop(&self) {
        #[cfg(any(test, feature = "testing"))]
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

impl Txn {
    /// Returns the hold on the transaction's snapshot.
    pub(super) fn hold(&self) -> &Hold {
        &self
            .held
            .as_ref()
            .expect("a transaction not ended holds its snapshot")
            .hold
    }

    /// Returns `done`, marking the transaction failed when it is an `Err`.
    pub(super) fn noted<T, E>(&self, done: Result<T, E>) -> Result<T, E> {
        if done.is_err() {
            self.failed.set(true);
        }
        done
    }

    /// Returns the transaction ended, with what its end releases.
    pub(super) fn end(mut self) -> Ended {
        let held = self
            .held
            .take()
            .expect("a transaction is ended only as it is consumed");
        self.parts(held)
    }

    fn parts(&mut self, held: Held) -> Ended {
        Ended {
            begun: held.begun,
            hold: Some(held.hold),
            xid: self.xid,
            sid: self.sid,
            taken: self.taken,
            written: self.written.take(),
            changed: self.changed.take(),
            left: Left {
                serial: true,
                waits: true,
                dead: true,
                committed: false,
            },
            session: self.session.clone(),
        }
    }
}

impl Drop for Txn {
    fn drop(&mut self) {
        if let Some(held) = self.held.take() {
            if !self.failed.get() {
                self.session.count_drop();
            }
            drop(self.parts(held));
        }
    }
}

impl Drop for Ended {
    fn drop(&mut self) {
        let session = self.session.clone();
        session.abandon(self);
    }
}
