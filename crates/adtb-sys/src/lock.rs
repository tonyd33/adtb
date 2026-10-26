use std::sync::{Mutex, MutexGuard, TryLockError};

use crate::runtime::Threads;

/// A mutex whose waiters spin through the runtime's `yield_now`, so a scheduler that switches
/// threads only at its own calls can run the holder.
///
/// A holder may block in the runtime, as on I/O, but must not park waiting for a thread that
/// needs the lock.
#[derive(Debug, Default)]
pub struct Lock<T>(Mutex<T>);

impl<T> Lock<T> {
    pub fn new(value: T) -> Lock<T> {
        Lock(Mutex::new(value))
    }

    pub fn lock(&self, threads: &impl Threads) -> MutexGuard<'_, T> {
        loop {
            match self.0.try_lock() {
                Ok(g) => return g,
                Err(TryLockError::Poisoned(e)) => return e.into_inner(),
                Err(TryLockError::WouldBlock) => threads.yield_now(),
            }
        }
    }
}
