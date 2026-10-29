use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use super::queue::Waiter;
use super::{host_thread, monotonic};
use crate::runtime::{Done, Instant};

/// The pending timers of one runtime, fired by one thread of their own.
#[derive(Debug, Default)]
pub(crate) struct Timers {
    state: Mutex<State>,
    wake: Condvar,
}

#[derive(Debug, Default)]
struct State {
    due: BinaryHeap<Reverse<Due>>,
    next: u64,
    stop: bool,
}

#[derive(Debug)]
struct Due {
    at: Instant,
    seq: u64,
    waiter: Waiter,
}

impl PartialEq for Due {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Due {}

impl PartialOrd for Due {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Due {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.at, self.seq).cmp(&(other.at, other.seq))
    }
}

impl Timers {
    /// Returns the timers with their thread started.
    pub(crate) fn start() -> Arc<Timers> {
        let timers = Arc::new(Timers::default());
        let run = Arc::clone(&timers);
        host_thread("adtb-timers".into(), move || run.fire());
        timers
    }

    pub(crate) fn add(&self, at: Instant, waiter: Waiter) {
        let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let seq = st.next;
        st.next += 1;
        st.due.push(Reverse(Due { at, seq, waiter }));
        self.wake.notify_one();
    }

    /// Stop the timer thread; timers still pending never fire.
    pub(crate) fn stop(&self) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stop = true;
        self.wake.notify_one();
    }

    fn fire(&self) {
        let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        while !st.stop {
            let now = monotonic();
            st = match st.due.peek().map(|Reverse(d)| d.at) {
                Some(at) if at <= now => {
                    let Reverse(d) = st.due.pop().expect("a peeked timer is still queued");
                    d.waiter.complete(None, Ok(Done::Fired));
                    st
                }
                Some(at) => {
                    self.wake
                        .wait_timeout(st, at.saturating_since(now))
                        .unwrap_or_else(PoisonError::into_inner)
                        .0
                }
                None => self.wake.wait(st).unwrap_or_else(PoisonError::into_inner),
            };
        }
    }
}
