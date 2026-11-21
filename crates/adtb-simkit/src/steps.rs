//! Stop a simulated thread at a step of the engine's work, so a test runs other work at that
//! step boundary, and interleave two step functions at every pair of their boundaries.

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

/// The boundaries of a step function a thread runs: the thread, and which steps it counts.
pub type Boundaries = (ThreadId, fn(Step) -> bool);

impl Stop {
    /// Stop `thread` at the `at`th step it reaches in `db` from now of those `counts` accepts,
    /// counting from 1, until
    /// [`Stop::release`]. The calling thread drives the stop; dropping it stops the thread at no
    /// later step.
    pub fn set(rt: &SimRuntime, db: &Database<SimRuntime>, on: Boundaries, at: usize) -> Stop {
        let [stop] = Stop::set_each(rt, db, [(on, at)]);
        stop
    }

    /// Set a stop as [`Stop::set`] does for each thread of `stops`, all at once.
    ///
    /// Precondition: no two of `stops` name one thread.
    pub fn set_each<const N: usize>(
        rt: &SimRuntime,
        db: &Database<SimRuntime>,
        stops: [(Boundaries, usize); N],
    ) -> [Stop; N] {
        let states = stops.map(|((thread, counts), at)| {
            Arc::new(Mutex::new(State {
                thread,
                counts,
                driver: rt.current(),
                at,
                reached: Vec::new(),
                stopped: false,
                released: false,
            }))
        });
        let (all, rt2) = (states.clone(), rt.clone());
        db.testing().on_step(move |step| {
            let here = rt2.current();
            let Some(s) = all.iter().find(|s| state(s).thread == here) else {
                return;
            };
            let driver = {
                let mut s = state(s);
                if !(s.counts)(step) {
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
            while !state(s).released {
                rt2.park();
            }
        });
        states.map(|state| Stop { state })
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

    /// Returns whether the thread has stopped.
    pub fn has_stopped(&self) -> bool {
        state(&self.state).stopped
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

/// A step function run on a thread of its own, which a driving thread starts and waits for.
pub trait Stepping {
    /// Returns its thread and the steps that are its boundaries.
    fn boundaries(&self) -> Boundaries;

    /// Start the step function.
    ///
    /// Postcondition: whatever ends it unparks the thread that called this.
    fn start(&mut self, rt: &SimRuntime);

    /// Returns whether the step function has ended.
    fn ended(&mut self) -> bool;
}

/// Where the second side of an interleaving was when the first went on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Second {
    /// Stopped at this boundary.
    At(Step),
    /// Ended before the boundary the pair names.
    Whole,
    /// Waiting on the stopped first side, with no thread able to run.
    Blocked,
}

/// What one side of an interleaving reached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reached {
    stopped: bool,
    blocked: bool,
    steps: Vec<Step>,
}

impl Reached {
    /// Returns whether it stopped at the boundary the pair names for it.
    pub fn stopped(&self) -> bool {
        self.stopped
    }

    /// Returns whether it waited on the other side, stopped, with no thread able to make
    /// progress: the second side before the first went on, or the first once it went on.
    pub fn blocked(&self) -> bool {
        self.blocked
    }

    /// Returns every boundary it reached, in order.
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// Returns where the second side was when the first went on, `j` the boundary the pair names
    /// for it.
    pub fn second(&self, j: usize) -> Second {
        if self.blocked {
            Second::Blocked
        } else if self.stopped {
            Second::At(self.steps[j - 1])
        } else {
            Second::Whole
        }
    }
}

/// The driver's turns in a row with no other thread able to make progress that show the second
/// side blocked: enough for a thread waiting on a lock just released to take it.
const STALLED: usize = 64;

/// Run `first` to its `i`th boundary, run `between`, then run `second` to its `j`th boundary, its
/// end, or until it blocks on `first`, then `first` to its end, releasing `second` should `first`
/// block on it, then `second` to its end, each boundary counted from 1. When `first` ends before its `i`th boundary, run `second` whole after
/// it. Returns what each reached.
pub fn interleave(
    rt: &SimRuntime,
    db: &Database<SimRuntime>,
    (first, second): (&mut impl Stepping, &mut impl Stepping),
    (i, j): (usize, usize),
    between: impl FnOnce(),
) -> [Reached; 2] {
    let [a, b] = Stop::set_each(rt, db, [(first.boundaries(), i), (second.boundaries(), j)]);
    first.start(rt);
    let a_stopped = a.stopped(rt, || first.ended().then_some(())).is_ok();
    let (mut b_stopped, mut b_blocked, mut a_blocked) = (false, false, false);
    if a_stopped {
        between();
        second.start(rt);
        match waited(rt, second, Some(&b)) {
            Waited::Stopped => b_stopped = true,
            Waited::Ended => {}
            Waited::Stalled => {
                b_blocked = true;
                b.release(rt);
            }
        }
        a.release(rt);
        if waited(rt, first, None) == Waited::Stalled {
            a_blocked = true;
            b.release(rt);
            until_ended(rt, first);
        }
        b.release(rt);
    } else {
        b.release(rt);
        second.start(rt);
    }
    until_ended(rt, second);
    [(a, a_stopped, a_blocked), (b, b_stopped, b_blocked)].map(|(stop, stopped, blocked)| Reached {
        stopped,
        blocked,
        steps: stop.reached(),
    })
}

/// How a wait for one side of an interleaving ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Waited {
    Stopped,
    Ended,
    /// No thread could make progress for [`STALLED`] turns of the driver in a row.
    Stalled,
}

/// Wait until `side` stops at `stop`, ends, or blocks.
fn waited(rt: &SimRuntime, side: &mut impl Stepping, stop: Option<&Stop>) -> Waited {
    let mut stalled = 0;
    loop {
        if stop.is_some_and(Stop::has_stopped) {
            return Waited::Stopped;
        }
        if side.ended() {
            return Waited::Ended;
        }
        stalled = if rt.stalled() { stalled + 1 } else { 0 };
        if stalled == STALLED {
            return Waited::Stalled;
        }
        rt.yield_to_event();
    }
}

fn until_ended(rt: &SimRuntime, side: &mut impl Stepping) {
    while !side.ended() {
        rt.park();
    }
}

/// One case of a search: the boundary of the first side it stopped at, and where the second was.
pub type Pair = (Step, Second);

/// Run `case` at every pair of boundaries its two sides reach, ending once a case shows the first
/// side reaches none past the one it stopped at, as [`interleave`] numbers them:
/// each boundary of the first side the first reaches, with each boundary of the second the
/// second then reaches, and with the second run to its end or blocked. Returns the boundaries
/// the first side stopped at and each pair run.
///
/// Panic when a case does not stop where its pair says, or the first side reaches other
/// boundaries than the case before it.
pub fn every_pair(mut case: impl FnMut((usize, usize)) -> [Reached; 2]) -> (Vec<Step>, Vec<Pair>) {
    let mut pairs = Vec::new();
    let mut stops = Vec::new();
    for i in 1.. {
        let mut first: Option<Vec<Step>> = None;
        for j in 1.. {
            let [a, b] = case((i, j));
            if !first_stopped(i, j, &a) {
                return (stops, pairs);
            }
            if j == 1 {
                stops.push(a.steps[i - 1]);
            }
            if let Some(f) = &first {
                assert_eq!(f, &a.steps, "the first side at {i} reaches as it did");
            }
            let second = b.second(j);
            pairs.push((a.steps[i - 1], second));
            let last = a.steps.len() == i;
            first = Some(a.steps);
            if !matches!(second, Second::At(_)) {
                if last {
                    return (stops, pairs);
                }
                break;
            }
        }
    }
    unreachable!("a step function reaches finitely many boundaries")
}

/// Run `case` at each boundary its first side reaches, with the second run to its end or
/// blocked, ending once a case shows the first side reaches none past the one it stopped at. Returns the boundaries the first side stopped at and each pair run.
pub fn every_boundary(
    mut case: impl FnMut((usize, usize)) -> [Reached; 2],
) -> (Vec<Step>, Vec<Pair>) {
    let mut pairs = Vec::new();
    let mut stops = Vec::new();
    for i in 1.. {
        let [a, b] = case((i, usize::MAX));
        if !first_stopped(i, 1, &a) {
            return (stops, pairs);
        }
        stops.push(a.steps[i - 1]);
        pairs.push((a.steps[i - 1], b.second(usize::MAX)));
        if a.steps.len() == i {
            return (stops, pairs);
        }
    }
    unreachable!("a step function reaches finitely many boundaries")
}

/// Returns whether the first side stopped at its `i`th boundary in the case `(i, j)`.
///
/// Panic when it ended first anywhere but at the first case of `i`, or past `i - 1` boundaries.
fn first_stopped(i: usize, j: usize, a: &Reached) -> bool {
    if a.stopped {
        return true;
    }
    assert_eq!(1, j, "the first side ends before boundary {i} at once");
    assert_eq!(
        i - 1,
        a.steps.len(),
        "the first side ends after {} steps",
        i - 1
    );
    false
}
