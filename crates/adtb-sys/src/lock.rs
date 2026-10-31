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

    /// Returns the guard if no other thread holds the lock, without waiting.
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        match self.0.try_lock() {
            Ok(g) => Some(g),
            Err(TryLockError::Poisoned(e)) => Some(e.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        }
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

    /// Returns the guard if no reader or writer holds the lock, without waiting.
    pub fn try_write(&self) -> Option<RwLockWriteGuard<'_, T>> {
        match self.0.try_write() {
            Ok(g) => Some(g),
            Err(TryLockError::Poisoned(e)) => Some(e.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        }
    }

    /// Returns the guard if no writer holds the lock, without waiting.
    pub fn try_read(&self) -> Option<RwLockReadGuard<'_, T>> {
        match self.0.try_read() {
            Ok(g) => Some(g),
            Err(TryLockError::Poisoned(e)) => Some(e.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_read_shares_with_readers_misses_a_writer_and_survives_poison() {
        let lock = RwLock::new(7);
        {
            let first = lock.try_read().expect("no writer holds it");
            let second = lock.try_read().expect("readers share it");
            assert_eq!(14, *first + *second);
        }
        {
            let writer = lock.0.write().expect("no one poisoned it");
            assert!(lock.try_read().is_none());
            drop(writer);
        }
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _writer = lock.0.write().expect("no one poisoned it");
            panic!("poison the lock");
        }));
        assert!(panicked.is_err());
        assert_eq!(7, *lock.try_read().expect("poison leaves it readable"));
    }
}
