//! The ranks of the engine's ordered locks, lowest first. A thread waits for one of these locks
//! only while every one it holds is ranked below it.

use adtb_sys::lock::Rank;

pub(crate) const VACUUM: Rank = Rank::new(1, "the vacuum lock");
pub(crate) const BUNDLE_WRITER: Rank = Rank::new(2, "the bundle writer lock");
/// The latch of one index's inserts; a thread holds at most one.
pub(crate) const INDEX_LATCH: Rank = Rank::new(3, "an index latch");
pub(crate) const SERIALIZER: Rank = Rank::new(4, "the serializer lock");
pub(crate) const READERS: Rank = Rank::new(5, "the readers lock");
pub(crate) const WAITS: Rank = Rank::new(6, "the waits lock");
pub(crate) const DEAD: Rank = Rank::new(7, "the dead counts lock");
pub(crate) const SEQUENCES: Rank = Rank::new(8, "the sequences lock");
pub(crate) const TASK_QUEUE: Rank = Rank::new(9, "the task queue lock");
pub(crate) const CALLS: Rank = Rank::new(10, "the calls lock");
pub(crate) const IDLE: Rank = Rank::new(11, "the idle workers lock");

/// Returns the message of the panic `f` ends in.
#[cfg(test)]
pub(crate) fn order_broken(f: impl FnOnce()) -> String {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => panic!("the wait returned"),
        Err(payload) => crate::write::panic_text(&*payload).expect("the panic has a message"),
    }
}

/// Returns the panic of a wait for `taken` while holding `holding`.
#[cfg(test)]
pub(crate) fn broken(taken: &str, holding: &str) -> String {
    format!("{taken} taken while holding {holding} breaks the lock order")
}
