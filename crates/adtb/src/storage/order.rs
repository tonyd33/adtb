//! The witnesses that a mark is set in commit order.

use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::mvcc::Serializer;
use crate::runtime::Runtime;

use super::pager::Pager;
use super::policy::Phase;
use super::recovery::Recovery;

/// A witness that no commit runs concurrently with its holder: the holder has its database's
/// serializer exclusively, under the serialization lock, or is the thread of the database's open.
#[derive(Debug)]
pub struct Ordered<'a>(PhantomData<&'a mut ()>);

/// A source of commit order.
pub trait Orders {
    /// Returns what `f` returns, run in commit order.
    fn ordered<T>(&self, f: impl FnOnce(Ordered<'_>) -> T) -> T;
}

impl Orders for Ordered<'_> {
    fn ordered<T>(&self, f: impl FnOnce(Ordered<'_>) -> T) -> T {
        f(Ordered(PhantomData))
    }
}

#[cfg(test)]
pub(super) fn ordered_for_test() -> Ordered<'static> {
    Ordered(PhantomData)
}

impl Serializer {
    /// Returns the witness that the caller holds the serialization lock.
    pub fn ordered(&mut self) -> Ordered<'_> {
        Ordered(PhantomData)
    }
}

/// The open of a database file, from the pager's open until the engine over it is built or the
/// open fails. While it lives the pager maps damage as opening.
#[derive(Debug)]
pub struct Opening {
    /// The pager's flag of an open in progress, set while this lives.
    flag: Arc<AtomicBool>,
    recovery: Option<Recovery>,
}

impl Opening {
    /// Returns the open of `pager`, with no recovery run.
    pub(super) fn begin<R: Runtime>(pager: &Pager<R>) -> Opening {
        let flag = pager.opening_flag().clone();
        flag.store(true, Ordering::Release);
        Opening {
            flag,
            recovery: None,
        }
    }

    /// Take `recovery`, what the open's recovery found and did.
    pub(super) fn recovered(&mut self, recovery: Recovery) {
        self.recovery = Some(recovery);
    }

    /// Returns what the open's recovery found and did, `None` for a file this open created.
    pub fn recovery(&self) -> Option<&Recovery> {
        self.recovery.as_ref()
    }

    /// Returns the witness that the open is ordered with every commit.
    pub fn ordered(&self) -> Ordered<'_> {
        Ordered(PhantomData)
    }
}

impl Orders for Opening {
    fn ordered<T>(&self, f: impl FnOnce(Ordered<'_>) -> T) -> T {
        f(Opening::ordered(self))
    }
}

impl Drop for Opening {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
    }
}

impl<R: Runtime> Pager<R> {
    /// Returns the phase damage maps in now: opening while an [`Opening`] of this pager lives.
    pub(super) fn phase(&self) -> Phase {
        if self.opening_flag().load(Ordering::Acquire) {
            Phase::Opening
        } else {
            Phase::Running
        }
    }
}
