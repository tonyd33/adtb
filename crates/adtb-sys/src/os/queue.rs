use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::Thread;

use crate::runtime::{AlignedBuf, Completion, Done, IoError, Ticket};

/// The completions of one I/O queue, waiting to be polled.
#[derive(Debug, Default)]
pub(crate) struct Ring(Mutex<VecDeque<Completion>>);

impl Ring {
    pub(crate) fn pop(&self) -> Option<Completion> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
    }
}

/// Where one submitted op completes: its queue, its ticket and the thread to unpark.
#[derive(Debug)]
pub(crate) struct Waiter {
    pub(crate) ring: Arc<Ring>,
    pub(crate) ticket: Ticket,
    pub(crate) thread: Thread,
}

impl Waiter {
    pub(crate) fn complete(self, buf: Option<AlignedBuf>, result: Result<Done, IoError>) {
        let completion = Completion {
            ticket: self.ticket,
            buf,
            result,
        };
        self.ring
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(completion);
        self.thread.unpark();
    }
}
