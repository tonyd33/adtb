//! Wait for locks through a runtime's scheduler.

use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, MutexGuard, RwLockReadGuard, RwLockWriteGuard, TryLockError};

use crate::runtime::Threads;

pub use rank::{HeldRank, Rank};

/// Ranks kept per thread, in debug builds and when `latch-checks` is on.
#[cfg(any(debug_assertions, feature = "latch-checks"))]
mod rank {
    use std::cell::RefCell;
    use std::marker::PhantomData;

    std::thread_local! {
        /// The ranks of the locks the calling thread holds.
        static HELD: RefCell<Vec<Rank>> = const { RefCell::new(Vec::new()) };
    }

    /// A lock's place in the order locks are taken in: a thread waits for a lock only while
    /// every lock it holds has a lower rank.
    #[derive(Clone, Copy, Debug)]
    pub struct Rank {
        level: u8,
        name: &'static str,
    }

    impl Rank {
        /// Returns the rank `level` of the lock `name` names in a panic.
        pub const fn new(level: u8, name: &'static str) -> Rank {
            Rank { level, name }
        }

        /// Returns whether the calling thread holds a lock of this rank.
        pub fn held_here(self) -> bool {
            HELD.try_with(|h| h.borrow().iter().any(|r| r.level == self.level))
                .unwrap_or(false)
        }

        /// Panic unless this rank is above every rank the calling thread holds.
        pub fn check(self) {
            let top = HELD
                .try_with(|h| h.borrow().iter().copied().max_by_key(|r| r.level))
                .ok()
                .flatten();
            if let Some(top) = top {
                assert!(
                    self.level > top.level,
                    "{} taken while holding {} breaks the lock order",
                    self.name,
                    top.name
                );
            }
        }

        /// Count a lock of this rank held by the calling thread until the returned mark drops.
        pub fn hold(self) -> HeldRank {
            HELD.try_with(|h| h.borrow_mut().push(self)).unwrap_or(());
            HeldRank {
                level: self.level,
                _thread: PhantomData,
            }
        }
    }

    /// A lock of one rank the calling thread holds. Not `Send`: it is released on the thread
    /// that took it.
    #[derive(Debug)]
    pub struct HeldRank {
        level: u8,
        _thread: PhantomData<*const ()>,
    }

    impl Drop for HeldRank {
        fn drop(&mut self) {
            let level = self.level;
            HELD.try_with(|h| {
                let mut h = h.borrow_mut();
                if let Some(i) = h.iter().rposition(|r| r.level == level) {
                    h.remove(i);
                }
            })
            .unwrap_or(());
        }
    }
}

/// Ranks that hold nothing and check nothing, in release builds without `latch-checks`.
#[cfg(not(any(debug_assertions, feature = "latch-checks")))]
mod rank {
    use std::marker::PhantomData;

    /// A lock's place in the order locks are taken in, unchecked.
    #[derive(Clone, Copy, Debug)]
    pub struct Rank;

    impl Rank {
        pub const fn new(_level: u8, _name: &'static str) -> Rank {
            Rank
        }

        /// Returns `false`: no rank is kept.
        pub fn held_here(self) -> bool {
            false
        }

        pub fn check(self) {}

        pub fn hold(self) -> HeldRank {
            HeldRank(PhantomData)
        }
    }

    /// A lock of one rank the calling thread holds, unrecorded.
    #[derive(Debug)]
    pub struct HeldRank(PhantomData<*const ()>);
}

/// A [`Lock`] of one rank, its waits checked against the order of ranks in debug builds and
/// under `latch-checks`.
#[derive(Debug)]
pub struct Ranked<T> {
    lock: Lock<T>,
    rank: Rank,
}

/// The guard of a [`Ranked`] lock.
#[derive(Debug)]
pub struct RankedGuard<'a, T> {
    guard: MutexGuard<'a, T>,
    _held: HeldRank,
}

impl<T> Ranked<T> {
    pub const fn new(rank: Rank, value: T) -> Ranked<T> {
        Ranked {
            lock: Lock::new(value),
            rank,
        }
    }

    /// Returns the guard if no other thread holds the lock, without waiting or checking the
    /// order.
    pub fn try_lock(&self) -> Option<RankedGuard<'_, T>> {
        let guard = self.lock.try_lock()?;
        Some(RankedGuard {
            guard,
            _held: self.rank.hold(),
        })
    }

    /// Wait for the lock as [`Lock::lock`] does. Panic first, in debug builds and under
    /// `latch-checks`, if the calling thread holds a lock not ranked below it.
    pub fn lock(&self, threads: &impl Threads) -> RankedGuard<'_, T> {
        self.rank.check();
        let guard = self.lock.lock(threads);
        RankedGuard {
            guard,
            _held: self.rank.hold(),
        }
    }
}

impl<T> Deref for RankedGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T> DerefMut for RankedGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

/// A mutex whose waiters spin through the runtime's `yield_now`, so a scheduler that switches
/// threads only at its own calls can run the holder.
///
/// A holder may block in the runtime, as on I/O, but must not park waiting for a thread that
/// needs the lock.
#[derive(Debug, Default)]
pub struct Lock<T>(Mutex<T>);

impl<T> Lock<T> {
    pub const fn new(value: T) -> Lock<T> {
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

    const LOW: Rank = Rank::new(1, "the low lock");
    const HIGH: Rank = Rank::new(2, "the high lock");

    fn panic_of(f: impl FnOnce()) -> Option<String> {
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        ran.err().and_then(|p| p.downcast_ref::<String>().cloned())
    }

    #[test]
    fn a_wait_below_a_held_rank_panics_naming_both_locks() {
        let _high = HIGH.hold();
        assert_eq!(
            Some("the low lock taken while holding the high lock breaks the lock order".into()),
            panic_of(|| LOW.check())
        );
    }

    #[test]
    fn a_second_lock_of_a_held_rank_panics() {
        let _low = LOW.hold();
        assert_eq!(
            Some("the low lock taken while holding the low lock breaks the lock order".into()),
            panic_of(|| LOW.check())
        );
    }

    #[test]
    fn ranks_released_out_of_order_free_their_own_place() {
        let low = LOW.hold();
        HIGH.check();
        let high = HIGH.hold();
        drop(low);
        assert!(!LOW.held_here());
        assert!(HIGH.held_here());
        drop(high);
        LOW.check();
        assert!(!HIGH.held_here());
    }

    #[test]
    fn a_try_lock_below_a_held_rank_takes_it_unchecked_and_counts_it() {
        let high = Ranked::new(HIGH, ());
        let low = Ranked::new(LOW, ());
        let _high = high.try_lock().expect("no one holds it");
        let held = low.try_lock().expect("no one holds it");
        assert!(LOW.held_here());
        drop(held);
        assert!(!LOW.held_here());
    }

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
