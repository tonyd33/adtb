use loom::sync::Arc;
use loom::thread;

use super::{PageMemory, PageState};
use crate::runtime::{ThreadId, Threads};
use crate::sync::Ordering;

/// A thread interface that hands the model's scheduler the turn while a thread waits.
struct Model;

impl Threads for Model {
    fn spawn<F: FnOnce() + Send + 'static>(&self, _: &str, _: F) -> ThreadId {
        unreachable!()
    }
    fn current(&self) -> ThreadId {
        unreachable!()
    }
    fn park(&self) {
        unreachable!()
    }
    fn unpark(&self, _: ThreadId) {
        unreachable!()
    }
    fn yield_now(&self) {
        thread::yield_now();
    }
    fn join(&self, _: ThreadId) {
        unreachable!()
    }
}

const PID: u64 = 0;

/// Returns one resident page whose first two words hold `fill`, at version 1.
fn filled(fill: u8) -> Arc<PageMemory> {
    let m = PageMemory::reserve(4096, 1);
    m.lock_exclusive(PID, &Model).write(0, &[fill; 16]);
    Arc::new(m)
}

fn first_word(m: &PageMemory) -> u64 {
    m.words(PID)[0].load(Ordering::Relaxed)
}

#[test]
fn a_shared_latch_sees_no_writer_or_all_of_one() {
    loom::model(|| {
        let m = filled(0);
        let w = Arc::clone(&m);
        let writer = thread::spawn(move || {
            let mut g = w.lock_exclusive(PID, &Model);
            g.write(0, &[1; 8]);
            g.write(0, &[2; 8]);
        });
        if let Some(s) = m.lock_shared(PID, &Model) {
            let (state, version) = m.state(PID);
            assert_eq!(PageState::Shared(1), state);
            let seen = first_word(&m);
            match version {
                1 => assert_eq!(
                    0, seen,
                    "a shared latch saw a writer's store before it unlatched"
                ),
                2 => assert_eq!(
                    u64::from_le_bytes([2; 8]),
                    seen,
                    "a shared latch after the writer unlatched missed its last store"
                ),
                v => panic!("version {v} after one writer"),
            }
            drop(s);
        }
        writer.join().unwrap();
        assert_eq!((PageState::Unlocked, 2), m.state(PID));
    });
}

#[test]
fn an_optimistic_read_sees_a_version_change_or_a_consistent_page() {
    loom::model(|| {
        let m = filled(0);
        let w = Arc::clone(&m);
        let writer = thread::spawn(move || {
            w.lock_exclusive(PID, &Model).write(0, &[1; 16]);
        });
        if let Some(r) = m.optimistic(PID) {
            let mut out = [9; 16];
            r.read(0, &mut out);
            if r.validate() {
                assert!(
                    out == [0; 16] || out == [1; 16],
                    "a validated read saw a torn page: {out:?}"
                );
            } else {
                assert_ne!((PageState::Unlocked, 1), m.state(PID));
            }
        }
        writer.join().unwrap();
    });
}

#[test]
fn the_clock_never_evicts_a_latched_page() {
    loom::model(|| {
        let m = filled(7);
        assert!(!m.mark(PID));
        let c = Arc::clone(&m);
        let clock = thread::spawn(move || {
            if c.mark(PID)
                && let Some(g) = c.try_lock_exclusive(PID)
            {
                g.evict();
            }
        });
        if let Some(s) = m.lock_shared(PID, &Model) {
            assert_eq!(
                u64::from_le_bytes([7; 8]),
                first_word(&m),
                "the clock evicted a page under a shared latch"
            );
            drop(s);
        }
        clock.join().unwrap();
        let (state, version) = m.state(PID);
        assert!(
            matches!(
                (state, version),
                (PageState::Unlocked | PageState::Marked, 1) | (PageState::Evicted, 2)
            ),
            "the clock and a shared latch left {state:?} at version {version}"
        );
    });
}

#[test]
fn the_clock_never_marks_over_an_exclusive_latch() {
    loom::model(|| {
        let m = filled(0);
        let c = Arc::clone(&m);
        let clock = thread::spawn(move || {
            c.mark(PID);
        });
        m.lock_exclusive(PID, &Model).write(0, &[3; 8]);
        clock.join().unwrap();
        let (state, version) = m.state(PID);
        assert!(
            matches!(state, PageState::Unlocked | PageState::Marked) && version == 2,
            "the clock and an exclusive latch left {state:?} at version {version}"
        );
        assert_eq!(u64::from_le_bytes([3; 8]), first_word(&m));
    });
}
