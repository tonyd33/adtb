//! Lock guards that name the thread holding them.

use std::ops::{Deref, DerefMut};
use std::sync::MutexGuard;
use std::sync::atomic::{AtomicU64, Ordering};

use adtb_sys::lock::Lock;

use crate::runtime::{Runtime, ThreadId};

pub(super) const NO_HOLDER: u64 = u64::MAX;

/// A lock's guard that names the thread holding it in `holder` while it lives.
pub(crate) struct Holding<'a, T> {
    guard: MutexGuard<'a, T>,
    holder: &'a AtomicU64,
}

impl<'a, T> Holding<'a, T> {
    pub(super) fn take(
        lock: &'a Lock<T>,
        holder: &'a AtomicU64,
        rt: &impl Runtime,
        me: ThreadId,
    ) -> Self {
        let guard = lock.lock(rt);
        holder.store(me.0, Ordering::Relaxed);
        Holding { guard, holder }
    }
}

impl<T> Deref for Holding<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T> DerefMut for Holding<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

impl<T> Drop for Holding<'_, T> {
    fn drop(&mut self) {
        self.holder.store(NO_HOLDER, Ordering::Relaxed);
    }
}
