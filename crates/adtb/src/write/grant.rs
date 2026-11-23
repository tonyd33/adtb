use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::Waker;

use crate::exec::{self, Entry, Priority, Refusal, Requested};
use crate::runtime::{Instant, Runtime};

use crate::mvcc::{Conflict, Waits, Watch, Xid};

use super::{Bounds, Database, Error, Stop, Txn};

/// The most the last run of statements to end held of its grant at once, and the bytes it
/// spilled.
#[derive(Debug, Default)]
pub(super) struct Charged {
    peak: AtomicUsize,
    spilled: AtomicUsize,
}

/// What notes the peak and spill of a run's grant once the run ends, and counts the grant as
/// held by the run's transaction in the waits until then.
pub(super) struct PeakNoted<'d, R: Runtime> {
    charged: &'d Charged,
    grant: Arc<exec::Grant>,
    held: Option<(&'d Waits<R>, Xid)>,
}

impl<R: Runtime> Drop for PeakNoted<'_, R> {
    fn drop(&mut self) {
        if let Some((waits, x)) = self.held {
            waits.released_grant(x);
        }
        self.charged
            .peak
            .store(self.grant.peak(), Ordering::Release);
        self.charged
            .spilled
            .store(self.grant.spilled(), Ordering::Release);
    }
}

impl<R: Runtime> Database<R> {
    /// Returns `bounds` with its grant, else one admitted from the budget for the statements of
    /// `tx`, and what notes the grant's peak once the statements end.
    pub(super) fn granted(
        &self,
        tx: &Txn,
        bounds: Bounds,
    ) -> Result<(Bounds, PeakNoted<'_, R>), Error> {
        let grant = self.grant_of(tx, &bounds)?;
        let waits = self.session.waits();
        let held = tx.xid().map(|x| {
            waits.holds_grant(x);
            (waits, x)
        });
        let noted = PeakNoted {
            charged: &self.charged,
            grant: grant.clone(),
            held,
        };
        let bounds = Bounds {
            grant: Some(grant),
            ..bounds
        };
        Ok((bounds, noted))
    }

    /// Returns the most the last run of statements to end held of its grant at once, and the
    /// bytes it spilled.
    #[cfg(any(test, feature = "testing"))]
    pub fn charged(&self) -> (usize, usize) {
        let c = &self.charged;
        (
            c.peak.load(Ordering::Acquire),
            c.spilled.load(Ordering::Acquire),
        )
    }

    /// Returns the bytes the grants now admitted hold and how many calls wait for one.
    #[cfg(any(test, feature = "testing"))]
    pub fn granted_and_waiting(&self) -> (usize, usize) {
        (self.budget.granted(), self.budget.waiting())
    }

    /// Returns the grant of `bounds`, else one admitted from the budget for an ad hoc call of
    /// `tx`, waiting in the queue until the cancel or deadline of `bounds` stops it.
    pub(crate) fn grant_of(&self, tx: &Txn, bounds: &Bounds) -> Result<Arc<exec::Grant>, Error> {
        match &bounds.grant {
            Some(g) => Ok(g.clone()),
            None => {
                let stop = Stop::of(bounds, None, None);
                self.admit_grant(Priority::AdHoc, (tx.began(), tx.xid()), &stop)
            }
        }
    }

    /// Returns a grant of the database's per-call memory from its budget for a call of
    /// `priority` whose transaction began at `began`, blocking the calling thread while the call
    /// waits in the budget's queue. A transaction `me` waits there, in the waits, for every
    /// transaction holding a grant, and one of those that waits for `me` is refused with a
    /// deadlock. Fail with what stops `stop`, and with `Cancelled` once the engine closes,
    /// leaving the queue.
    pub fn admit_grant(
        &self,
        priority: Priority,
        (began, me): (Instant, Option<Xid>),
        stop: &Stop,
    ) -> Result<Arc<exec::Grant>, Error> {
        let entry = Entry {
            priority,
            rank: u8::MAX,
            start: began,
            bytes: 0,
            deadline: None,
        };
        let ticket = match self.request_grant(&entry)? {
            Requested::Granted(g) => return Ok(g),
            Requested::Queued(t) => t,
        };
        let waits = self.session.waits();
        let _queued = me.map(|x| {
            waits.queued(x);
            Queued { waits, me: x }
        });
        loop {
            if let Some(g) = ticket.poll(Waker::noop()) {
                return Ok(g);
            }
            if let Some(x) = me {
                waits.break_cycle(x);
            }
            self.rebuild_stopped()?;
            stop.check(self.rt.now())?;
            self.pause_a_poll();
        }
    }

    /// Register `waker` to be woken once transaction `on` has ended, as
    /// [`crate::mvcc::Waits::watch`] does. `me` is the waiting transaction, if it has an xid,
    /// waiting to write a version of `heap`, or at a gate for `None`.
    pub fn watch(
        &self,
        me: Option<Xid>,
        (on, heap): (Xid, Option<u32>),
        waker: &Waker,
    ) -> Result<Watch, Conflict> {
        self.session.waits().watch(me, on, heap, waker)
    }

    /// Withdraw the registration of `waker` on `on` made as `me`, if `on` has not ended.
    pub fn withdraw(&self, me: Option<Xid>, on: Xid, waker: &Waker) {
        self.session.waits().withdraw(me, on, waker);
    }

    /// Record transaction `me` as waiting in the budget's queue, as [`Database::admit_grant`]
    /// does, until [`Database::left_grant_queue`] or its end.
    pub fn queued_for_grant(&self, me: Xid) {
        self.session.waits().queued(me);
    }

    pub fn left_grant_queue(&self, me: Xid) {
        self.session.waits().left_queue(me);
    }

    /// Request a grant of the database's per-call memory for the call `entry` describes.
    ///
    /// Fail, queueing nothing, with `Overloaded` when the budget's queue is full, and with the
    /// statement limit of the queue's byte bound when the call alone passes it.
    pub fn request_grant(&self, entry: &Entry) -> Result<Requested, Error> {
        self.budget.request(entry, self.grant).map_err(|r| match r {
            Refusal::Overloaded | Refusal::ShutDown => Error::Exec(exec::Error::Overloaded),
            Refusal::TooLarge => Error::Exec(exec::Error::StatementLimit(exec::Exceeded::Memory {
                limit: self.budget.queue_bytes(),
            })),
        })
    }
}

/// A transaction's place in the waits while it waits in the budget's queue.
struct Queued<'w, R: Runtime> {
    waits: &'w Waits<R>,
    me: Xid,
}

impl<R: Runtime> Drop for Queued<'_, R> {
    fn drop(&mut self) {
        self.waits.left_queue(self.me);
    }
}
