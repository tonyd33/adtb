use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use adtb_sys::lock::Lock;

use crate::runtime::{Runtime, ThreadId};

/// Work a worker runs to completion.
pub type Task = Box<dyn FnOnce() + Send + 'static>;

struct Inner {
    queue: Lock<VecDeque<Task>>,
    /// Workers parked, or about to park, on an empty queue.
    idle: Lock<Vec<ThreadId>>,
    stop: AtomicBool,
    ran: AtomicUsize,
}

/// Worker threads that take tasks from one queue: morsels of every running query.
pub struct Scheduler<R: Runtime> {
    rt: R,
    inner: Arc<Inner>,
    workers: Lock<Vec<ThreadId>>,
}

impl<R: Runtime> std::fmt::Debug for Scheduler<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler").finish_non_exhaustive()
    }
}

impl<R: Runtime> Scheduler<R> {
    /// Start `workers` worker threads through the runtime.
    pub fn start(rt: R, workers: usize) -> Arc<Scheduler<R>> {
        let inner = Arc::new(Inner {
            queue: Lock::new(VecDeque::new()),
            idle: Lock::new(Vec::new()),
            stop: AtomicBool::new(false),
            ran: AtomicUsize::new(0),
        });
        let ids = (0..workers)
            .map(|i| {
                let (rt2, inner) = (rt.clone(), inner.clone());
                rt.spawn(&format!("morsel worker {i}"), move || work(&rt2, &inner))
            })
            .collect();
        Arc::new(Scheduler {
            rt,
            inner,
            workers: Lock::new(ids),
        })
    }

    pub fn runtime(&self) -> &R {
        &self.rt
    }

    pub fn submit(&self, task: Task) {
        self.inner.queue.lock(&self.rt).push_back(task);
        let idle = std::mem::take(&mut *self.inner.idle.lock(&self.rt));
        for t in idle {
            self.rt.unpark(t);
        }
    }

    /// Run one queued task on the calling thread. Returns false when the queue was empty.
    pub fn help(&self) -> bool {
        let task = self.inner.queue.lock(&self.rt).pop_front();
        match task {
            Some(t) => {
                t();
                true
            }
            None => false,
        }
    }

    /// Returns how many tasks the workers have run, not counting those run by `help`.
    pub fn ran(&self) -> usize {
        self.inner.ran.load(Ordering::Acquire)
    }

    /// Stop the workers once the queue is empty, and wait for them to return.
    pub fn shutdown(&self) {
        self.inner.stop.store(true, Ordering::Release);
        let workers = std::mem::take(&mut *self.workers.lock(&self.rt));
        for &w in &workers {
            self.rt.unpark(w);
        }
        for w in workers {
            self.rt.join(w);
        }
    }
}

fn work<R: Runtime>(rt: &R, inner: &Inner) {
    loop {
        let task = {
            let mut q = inner.queue.lock(rt);
            let t = q.pop_front();
            if t.is_none() {
                if inner.stop.load(Ordering::Acquire) {
                    return;
                }
                inner.idle.lock(rt).push(rt.current());
            }
            t
        };
        match task {
            Some(t) => {
                t();
                inner.ran.fetch_add(1, Ordering::AcqRel);
            }
            None => rt.park(),
        }
    }
}
