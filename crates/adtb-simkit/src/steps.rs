//! Stop a simulated thread at a step of the engine's work, so a test runs other work at that
//! step boundary, and interleave two step functions at every pair of their boundaries.

use std::sync::{Arc, Mutex, MutexGuard};

use adtb::Database;
use adtb::runtime::{Runtime, ThreadId, Threads};
use adtb::testing::{Reindex, Step};
use adtb_sim::SimRuntime;

#[derive(Debug)]
struct State {
    thread: Option<ThreadId>,
    counts: fn(Step) -> bool,
    driver: ThreadId,
    at: usize,
    reached: Vec<Step>,
    stopped: bool,
    released: bool,
    /// Set when the stopped thread is to unwind from its stop as it goes on.
    unwind: bool,
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
/// `None` names the first thread to reach a step it counts that no other stop set with it names.
pub type Boundaries = (Option<ThreadId>, fn(Step) -> bool);

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
                unwind: false,
            }))
        });
        let (all, rt2) = (states.clone(), rt.clone());
        db.testing().on_step(move |step| {
            let here = rt2.current();
            let named = all.iter().find(|s| state(s).thread == Some(here));
            let Some(s) = named.or_else(|| {
                let unnamed = all.iter().find(|s| {
                    let s = state(s);
                    s.thread.is_none() && (s.counts)(step)
                })?;
                state(unnamed).thread = Some(here);
                Some(unnamed)
            }) else {
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
            let unwind = state(s).unwind;
            if unwind {
                std::panic::resume_unwind(Box::new(Unwound));
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
        done: &mut dyn FnMut() -> Option<T>,
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

    /// Returns the thread it stops, once named.
    pub fn thread(&self) -> Option<ThreadId> {
        state(&self.state).thread
    }

    /// Returns whether the thread has stopped.
    pub fn has_stopped(&self) -> bool {
        state(&self.state).stopped
    }

    /// Let the stopped thread go on.
    pub fn release(&self, rt: &SimRuntime) {
        self.go_on(rt, false);
    }

    /// Let the stopped thread go on by unwinding from its stop with an [`Unwound`] payload,
    /// without running the panic hook.
    pub fn unwind(&self, rt: &SimRuntime) {
        self.go_on(rt, true);
    }

    fn go_on(&self, rt: &SimRuntime, unwind: bool) {
        let thread = {
            let mut s = state(&self.state);
            s.released = true;
            s.unwind = unwind;
            s.thread
        };
        if let Some(thread) = thread {
            rt.unpark(thread);
        }
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

/// The payload a thread [`Stop::unwind`] lets go on unwinds with.
#[derive(Debug)]
pub struct Unwound;

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

    /// Returns whether, stopped first in an [`interleave`], it goes on by unwinding from its stop.
    fn unwinds(&self) -> bool {
        false
    }
}

/// Where the second side of an interleaving was when the first went on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Second {
    /// Stopped at this boundary.
    At(Step),
    /// Ended before the boundary the pair names.
    Whole,
    /// Waiting on the stopped first side: no thread able to run, or a thread waiting for the
    /// vacuum lock or a rebuild's gate that the first side's thread holds, or for a transaction
    /// begun on that thread to end.
    Blocked,
}

/// What one side of an interleaving reached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reached {
    stopped: bool,
    blocked: bool,
    /// Whether it went on from its stop by unwinding, reaching no later boundary.
    unwound: bool,
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

    /// Returns whether a boundary after its stop may follow: it reached one, or it went on by
    /// unwinding, which cuts off those its run alone reaches.
    fn may_pass(&self, i: usize) -> bool {
        self.steps.len() > i || self.unwound
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

/// Run `first` to its `i`th boundary, run the first of `between`, then run `second` to its `j`th
/// boundary, its end, or until it blocks on `first`, run the second of `between` with where
/// `second` is, then `first` to its end, unwinding from its stop when it
/// [unwinds](Stepping::unwinds), releasing `second` should `first` block on it, then `second` to
/// its end, each boundary counted from 1. When `first` ends before its `i`th
/// boundary, run `second` whole after it and neither of `between`. Returns what each reached.
pub fn interleave(
    rt: &SimRuntime,
    db: &Database<SimRuntime>,
    (first, second): (&mut impl Stepping, &mut impl Stepping),
    (i, j): (usize, usize),
    between: Between<'_>,
) -> [Reached; 2] {
    let [a, b] = Stop::set_each(rt, db, [(first.boundaries(), i), (second.boundaries(), j)]);
    first.start(rt);
    let a_stopped = a.stopped(rt, &mut || first.ended().then_some(())).is_ok();
    let (mut b_stopped, mut b_blocked, mut a_blocked) = (false, false, false);
    let mut a_unwound = false;
    if a_stopped {
        let (stopped, waited_on) = between;
        stopped();
        second.start(rt);
        let at = match waited(rt, db, second, Some(&b), a.thread()) {
            Waited::Stopped => {
                b_stopped = true;
                Second::At(
                    *b.reached()
                        .last()
                        .expect("a stopped side reached a boundary"),
                )
            }
            Waited::Ended => Second::Whole,
            Waited::Stalled => {
                b_blocked = true;
                b.release(rt);
                Second::Blocked
            }
        };
        waited_on(at);
        a_unwound = first.unwinds();
        if a_unwound {
            a.unwind(rt);
        } else {
            a.release(rt);
        }
        if waited(rt, db, first, None, b.thread()) == Waited::Stalled {
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
    [
        (a, a_stopped, a_blocked, a_unwound),
        (b, b_stopped, b_blocked, false),
    ]
    .map(|(stop, stopped, blocked, unwound)| Reached {
        stopped,
        blocked,
        unwound,
        steps: stop.reached(),
    })
}

/// How a wait for one side of an interleaving ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Waited {
    Stopped,
    Ended,
    /// No thread could make progress for [`STALLED`] turns of the driver in a row, or a thread
    /// waited for the vacuum lock or a rebuild's gate that the stopped side's thread holds, or for
    /// a transaction begun on that thread to end.
    Stalled,
}

/// Wait until `side` stops at `stop`, ends, or blocks on `stopped`, the other side's thread.
fn waited(
    rt: &SimRuntime,
    db: &Database<SimRuntime>,
    side: &mut impl Stepping,
    stop: Option<&Stop>,
    stopped: Option<ThreadId>,
) -> Waited {
    let mut stalled = 0;
    loop {
        if stop.is_some_and(Stop::has_stopped) {
            return Waited::Stopped;
        }
        if side.ended() {
            return Waited::Ended;
        }
        stalled = if rt.stalled() { stalled + 1 } else { 0 };
        if stalled == STALLED || stopped.is_some_and(|t| db.testing().waits_on(t)) {
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

/// What [`interleave`] runs once the first side stops, and once the second stops, ends or
/// blocks, given where it is.
pub type Between<'a> = (Box<dyn FnOnce() + 'a>, Box<dyn FnOnce(Second) + 'a>);

/// Returns a [`Between`] that runs nothing.
pub fn nothing_between<'a>() -> Between<'a> {
    (Box::new(|| {}), Box::new(|_| {}))
}

/// One case of a search: the boundary of the first side it stopped at, and where the second was.
pub type Pair = (Step, Second);

/// Run `case` at every pair of boundaries its two sides reach, ending once no case at a boundary
/// of the first side shows it reaches one past it, or, for a first side that unwinds from its
/// stop, once it ends before the boundary, as [`interleave`] numbers them:
/// each boundary of the first side the first reaches, with each boundary of the second the
/// second then reaches, and with the second run to its end or blocked. Returns the boundaries
/// the first side stopped at and each pair run.
///
/// Panic when a case does not stop where its pair says, or the first side reaches other
/// boundaries up to its stop than the case before it.
pub fn every_pair(case: &mut dyn FnMut((usize, usize)) -> [Reached; 2]) -> (Vec<Step>, Vec<Pair>) {
    let mut pairs = Vec::new();
    let mut stops = Vec::new();
    for i in 1.. {
        let mut first: Option<Vec<Step>> = None;
        let mut past = false;
        for j in 1.. {
            let [a, b] = case((i, j));
            if !first_stopped(i, j, &a) {
                return (stops, pairs);
            }
            if j == 1 {
                stops.push(a.steps[i - 1]);
            }
            if let Some(f) = &first {
                assert_eq!(f, &a.steps[..i], "the first side at {i} reaches as it did");
            }
            let second = b.second(j);
            pairs.push((a.steps[i - 1], second));
            past |= a.may_pass(i);
            first = Some(a.steps[..i].to_vec());
            if !matches!(second, Second::At(_)) {
                if !past {
                    return (stops, pairs);
                }
                break;
            }
        }
    }
    unreachable!("a step function reaches finitely many boundaries")
}

/// Run `case` at each boundary its first side reaches, with the second run to its end or
/// blocked, ending once a case shows the first side reaches none past the one it stopped at, or,
/// for a first side that unwinds from its stop, once it ends before it. Returns the boundaries
/// the first side stopped at and each pair run.
pub fn every_boundary(
    case: &mut dyn FnMut((usize, usize)) -> [Reached; 2],
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
        if !a.may_pass(i) {
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

/// Call `hook` with each phase of every later rebuild of an index as it begins, on the job's
/// thread, in place of any step hook set before.
#[inline]
pub fn on_reindex<R: Runtime>(db: &Database<R>, body: impl Fn(Reindex) + Send + Sync + 'static) {
    on_phase(db, Box::new(body));
}

fn on_phase<R: Runtime>(db: &Database<R>, hook: Box<dyn Fn(Reindex) + Send + Sync>) {
    db.testing().on_step(move |step| {
        if let Step::Reindex(phase) = step {
            hook(phase);
        }
    });
}
