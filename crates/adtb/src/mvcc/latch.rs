use std::sync::{Mutex, MutexGuard, TryLockError};

use crate::runtime::Threads;

/// A mutex whose waiters spin through the runtime's `yield_now`, so a scheduler that switches
/// threads only at its own calls can run the holder.
#[derive(Debug, Default)]
pub(crate) struct Latch<T>(Mutex<T>);

impl<T> Latch<T> {
    pub(crate) fn lock(&self, threads: &impl Threads) -> MutexGuard<'_, T> {
        loop {
            match self.0.try_lock() {
                Ok(g) => return g,
                Err(TryLockError::Poisoned(e)) => return e.into_inner(),
                Err(TryLockError::WouldBlock) => threads.yield_now(),
            }
        }
    }
}
