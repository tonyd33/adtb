//! The ranks of the engine's ordered locks, lowest first. A thread waits for one of these locks
//! only while every one it holds is ranked below it.

use adtb_sys::lock::Rank;

pub const VACUUM: Rank = Rank::new(1, "the vacuum lock");
pub const BUNDLE_WRITER: Rank = Rank::new(2, "the bundle writer lock");
/// The latch of one index's inserts; a thread holds at most one.
pub const INDEX_LATCH: Rank = Rank::new(3, "an index latch");
pub const SERIALIZER: Rank = Rank::new(4, "the serializer lock");
pub const READERS: Rank = Rank::new(5, "the readers lock");
pub const WAITS: Rank = Rank::new(6, "the waits lock");
pub const DEAD: Rank = Rank::new(7, "the dead counts lock");
pub const SEQUENCES: Rank = Rank::new(8, "the sequences lock");
pub const TASK_QUEUE: Rank = Rank::new(9, "the task queue lock");
pub const CALLS: Rank = Rank::new(10, "the calls lock");
pub const IDLE: Rank = Rank::new(11, "the idle workers lock");

/// Panic, in debug builds and under `latch-checks`, if the calling thread holds a page latch or
/// an index latch as it does `what`.
pub fn unlatched(what: std::fmt::Arguments<'_>) {
    let latched = adtb_sys::pages::latched_by_this_thread();
    assert!(
        latched == 0,
        "{what} while this thread latches {latched} pages"
    );
    assert!(!INDEX_LATCH.held_here(), "{what} under an index latch");
}

/// Returns the message of the panic `f` ends in.
#[cfg(any(test, feature = "testing"))]
pub fn order_broken(f: impl FnOnce()) -> String {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => panic!("the wait returned"),
        Err(payload) => crate::panic_text(&*payload).expect("the panic has a message"),
    }
}

/// Returns the panic of a wait for `taken` while holding `holding`.
#[cfg(any(test, feature = "testing"))]
pub fn broken(taken: &str, holding: &str) -> String {
    format!("{taken} taken while holding {holding} breaks the lock order")
}
