use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use super::files::Files;
use super::host_thread;
use super::queue::Waiter;
use crate::runtime::Op;

/// Threads that run file operations with blocking system calls.
#[derive(Debug)]
pub(crate) struct Pool {
    state: Mutex<State>,
    ready: Condvar,
    files: Arc<Files>,
}

#[derive(Debug, Default)]
struct State {
    jobs: VecDeque<(Op, Waiter)>,
    stop: bool,
}

impl Pool {
    /// Returns a pool of `threads` threads, started.
    ///
    /// Panics if `threads` is zero.
    pub(crate) fn start(files: Arc<Files>, threads: usize) -> Arc<Pool> {
        assert!(threads > 0, "an I/O pool has at least one thread");
        let pool = Arc::new(Pool {
            state: Mutex::default(),
            ready: Condvar::new(),
            files,
        });
        for i in 0..threads {
            let run = Arc::clone(&pool);
            host_thread(format!("adtb-io-{i}"), move || run.work());
        }
        pool
    }

    pub(crate) fn push(&self, op: Op, waiter: Waiter) {
        let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        st.jobs.push_back((op, waiter));
        self.ready.notify_one();
    }

    /// Stop the threads once every op already pushed has completed.
    pub(crate) fn stop(&self) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stop = true;
        self.ready.notify_all();
    }

    fn work(&self) {
        loop {
            let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let (op, waiter) = loop {
                if let Some(job) = st.jobs.pop_front() {
                    break job;
                }
                if st.stop {
                    return;
                }
                st = self.ready.wait(st).unwrap_or_else(PoisonError::into_inner);
            };
            drop(st);
            let (buf, result) = self.files.run(op);
            waiter.complete(buf, result);
        }
    }
}
