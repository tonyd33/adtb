//! Stop a simulated thread at a step of the engine's work, so a test runs other work at that
//! step boundary.

use std::sync::{Arc, Mutex, MutexGuard};

use adtb::Database;
use adtb::runtime::{ThreadId, Threads};
use adtb::testing::Step;
use adtb_sim::SimRuntime;

#[derive(Debug)]
struct State {
    thread: ThreadId,
    counts: fn(Step) -> bool,
    driver: ThreadId,
    at: usize,
    reached: Vec<Step>,
    stopped: bool,
    released: bool,
}

/// The stop of one thread at one step, set as the database's step hook.
#[derive(Debug)]
pub struct Stop {
    state: Arc<Mutex<State>>,
}

fn state(s: &Mutex<State>) -> MutexGuard<'_, State> {
    s.lock().expect("no thread panics holding a stop's state")
}

impl Stop {
    /// Stop `thread` at the `at`th step it reaches in `db` from now of those `counts` accepts,
    /// counting from 1, until
    /// [`Stop::release`]. The calling thread drives the stop; dropping it stops the thread at no
    /// later step.
    pub fn set(
        rt: &SimRuntime,
        db: &Database<SimRuntime>,
        (thread, counts): (ThreadId, fn(Step) -> bool),
        at: usize,
    ) -> Stop {
        let shared = Arc::new(Mutex::new(State {
            thread,
            counts,
            driver: rt.current(),
            at,
            reached: Vec::new(),
            stopped: false,
            released: false,
        }));
        let (s, rt2) = (shared.clone(), rt.clone());
        db.testing().on_step(move |step| {
            let driver = {
                let mut s = state(&s);
                if rt2.current() != s.thread || !(s.counts)(step) {
                    return;
                }
                s.reached.push(step);
                if s.reached.len() != s.at {
                    return;
                }
                s.stopped = true;
                s.driver
            };
            rt2.unpark(driver);
            while !state(&s).released {
                rt2.park();
            }
        });
        Stop { state: shared }
    }

    /// Wait until the thread stops or `done` returns `Some`. Returns the step it stopped at,
    /// or what `done` returned.
    ///
    /// Precondition: whatever makes `done` return `Some` unparks the driving thread.
    pub fn stopped<T>(
        &self,
        rt: &SimRuntime,
        mut done: impl FnMut() -> Option<T>,
    ) -> Result<Step, T> {
        loop {
            {
                let s = state(&self.state);
                if s.stopped {
                    return Ok(*s.reached.last().expect("a stopped thread reached a step"));
                }
            }
            if let Some(out) = done() {
                return Err(out);
            }
            rt.park();
        }
    }

    /// Let the stopped thread go on.
    pub fn release(&self, rt: &SimRuntime) {
        let thread = {
            let mut s = state(&self.state);
            s.released = true;
            s.thread
        };
        rt.unpark(thread);
    }

    /// Returns the steps `counts` accepts the thread reached since the stop was set.
    pub fn reached(&self) -> Vec<Step> {
        state(&self.state).reached.clone()
    }
}

impl Drop for Stop {
    /// Stop the thread at no later step.
    fn drop(&mut self) {
        let mut s = state(&self.state);
        s.at = 0;
        s.released = true;
    }
}
