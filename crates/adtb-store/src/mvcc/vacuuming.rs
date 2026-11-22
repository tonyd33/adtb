//! The vacuum lock of one database.

use std::cell::Cell;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use adtb_sys::lock::Lock;

use super::Transactions;
use crate::runtime::{Runtime, ThreadId};
use crate::storage::Pager;

/// The holder of the vacuum lock.
#[derive(Debug)]
pub enum Holder {
    /// One on `thread` that keeps the lock until it is done.
    Keeps { thread: ThreadId },
    /// One that gives the lock up once `asked` is set, looking at it each time `thread` unparks.
    Yields {
        asked: Arc<AtomicBool>,
        thread: ThreadId,
    },
}

/// Why [`Transactions::take_vacuum`] did not take the lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Busy {
    /// A holder that keeps it holds it, on this thread.
    Kept(ThreadId),
    /// A holder that yields holds it, on this thread, and was asked to give it up.
    Asked(ThreadId),
}

/// The vacuum lock of one database: who holds it, if anyone.
#[derive(Debug)]
pub(super) struct VacuumLock {
    held: Lock<Option<Holder>>,
}

impl VacuumLock {
    pub(super) fn new() -> VacuumLock {
        VacuumLock {
            held: Lock::new(None),
        }
    }

    /// Take the lock over `pager` and `txns` for `holder`, calling `taken` while no other can take
    /// it. The lock is free again once the returned [`Vacuuming`] drops.
    ///
    /// Fail while another holds it, setting a yielding holder's `asked` first.
    pub(super) fn take<'d, R: Runtime>(
        &'d self,
        rt: &'d R,
        pager: &'d Pager<R>,
        txns: &'d Transactions<R>,
        holder: Holder,
        taken: impl FnOnce(),
    ) -> Result<Vacuuming<'d, R>, Busy> {
        let mut held = self.held.lock(rt);
        match &*held {
            Some(Holder::Keeps { thread }) => Err(Busy::Kept(*thread)),
            Some(Holder::Yields { asked, thread }) => {
                asked.store(true, Ordering::Release);
                Err(Busy::Asked(*thread))
            }
            None => {
                *held = Some(holder);
                taken();
                Ok(Vacuuming {
                    lock: Some((self, rt)),
                    pager,
                    txns,
                    _one_thread: PhantomData,
                })
            }
        }
    }
}

/// The vacuum lock of one database, held on one thread, as the vacuum, the sweep of a large-value
/// store and a copy to a history heap take it. Built only by [`Transactions::take_vacuum`], and by
/// tests.
pub struct Vacuuming<'d, R: Runtime> {
    lock: Option<(&'d VacuumLock, &'d R)>,
    pager: &'d Pager<R>,
    txns: &'d Transactions<R>,
    _one_thread: PhantomData<Cell<()>>,
}

impl<R: Runtime> Drop for Vacuuming<'_, R> {
    fn drop(&mut self) {
        if let Some((lock, rt)) = self.lock {
            *lock.held.lock(rt) = None;
        }
    }
}

impl<'d, R: Runtime> Vacuuming<'d, R> {
    /// Returns a lock over `pager` and `txns` that holds nothing off.
    #[cfg(any(test, feature = "testing"))]
    pub fn unheld(pager: &'d Pager<R>, txns: &'d Transactions<R>) -> Vacuuming<'d, R> {
        Vacuuming {
            lock: None,
            pager,
            txns,
            _one_thread: PhantomData,
        }
    }

    /// Returns the pager of the database whose lock this is.
    pub fn pager(&self) -> &'d Pager<R> {
        self.pager
    }

    /// Returns the transactions of the database whose lock this is.
    pub fn txns(&self) -> &'d Transactions<R> {
        self.txns
    }
}

impl<R: Runtime> std::fmt::Debug for Vacuuming<'_, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Vacuuming")
    }
}
