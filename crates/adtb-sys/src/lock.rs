use std::sync::{Mutex, MutexGuard, RwLockReadGuard, RwLockWriteGuard, TryLockError};

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

/// A readers-writer lock whose waiters spin through the runtime's `yield_now`, as [`Lock`]'s do.
#[derive(Debug, Default)]
pub struct RwLock<T>(std::sync::RwLock<T>);

impl<T> RwLock<T> {
    pub fn new(value: T) -> RwLock<T> {
        RwLock(std::sync::RwLock::new(value))
    }

    /// Wait while a writer holds the lock, then share it with other readers.
    pub fn read(&self, threads: &impl Threads) -> RwLockReadGuard<'_, T> {
        loop {
            match self.0.try_read() {
                Ok(g) => return g,
                Err(TryLockError::Poisoned(e)) => return e.into_inner(),
                Err(TryLockError::WouldBlock) => threads.yield_now(),
            }
        }
    }

    /// Wait until no reader or writer holds the lock, then hold it alone.
    pub fn write(&self, threads: &impl Threads) -> RwLockWriteGuard<'_, T> {
        loop {
            match self.0.try_write() {
                Ok(g) => return g,
                Err(TryLockError::Poisoned(e)) => return e.into_inner(),
                Err(TryLockError::WouldBlock) => threads.yield_now(),
            }
        }
    }
}
