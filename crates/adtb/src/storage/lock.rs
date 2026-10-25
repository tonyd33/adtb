use std::sync::{Mutex, MutexGuard};

use crate::runtime::Threads;

/// A mutex whose waiters spin through the runtime's `yield_now`, so a scheduler that switches
/// threads only at its own calls can run the holder.
#[derive(Debug, Default)]
pub(crate) struct Lock<T>(Mutex<T>);

impl<T> Lock<T> {
    pub(crate) fn new(value: T) -> Lock<T> {
        Lock(Mutex::new(value))
    }

    pub(crate) fn lock(&self, threads: &impl Threads) -> MutexGuard<'_, T> {
        loop {
            match self.0.try_lock() {
                Ok(g) => return g,
                Err(std::sync::TryLockError::Poisoned(e)) => return e.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => threads.yield_now(),
            }
        }
    }
}
