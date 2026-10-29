use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::task::{Wake, Waker};

use adtb_sys::lock::Lock;

use crate::runtime::{Instant, Io, Op, Runtime, ThreadId, Ticket};

/// Work a worker runs to completion.
pub type Task = Box<dyn FnOnce() + Send + 'static>;

/// The priority class of a call, lowest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    AdHoc,
    Background,
    Regular,
}

/// What bounds the calls admitted and not yet started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Admission {
    pub entries: usize,
    /// The sum of the queued calls' [`Entry::bytes`].
    pub bytes: usize,
}

impl Default for Admission {
    fn default() -> Admission {
        Admission {
            entries: 1024,
            bytes: 64 << 20,
        }
    }
}

/// Where an admitted call queues, and when it stops waiting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub priority: Priority,
    /// The call's priority within its class, higher first.
    pub rank: u8,
    /// The start of the call's transaction.
    pub start: Instant,
    /// The memory the call's owned arguments and metadata hold.
    pub bytes: usize,
    /// The instant from which the call stops with `DeadlineExceeded` instead of starting or
    /// resuming.
    pub deadline: Option<Instant>,
}

/// How a step of a call ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Done,
    /// The call registered its waker with a dependency and runs again once woken.
    Wait,
    /// As `Wait`, and the call is also woken at the instant.
    WaitUntil(Instant),
}

/// Why a call was stopped without running again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stopped {
    Cancelled,
    DeadlineExceeded,
    ShutDown,
}

/// Why [`Scheduler::admit`] refused a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The admission queue was full. The same call may be admitted later.
    Overloaded,
    /// The call's bytes alone pass the admission byte bound. It is never admitted.
    TooLarge,
    ShutDown,
}

/// A call [`Scheduler::admit`] refused, handed back to be stopped or admitted again.
pub struct Refused {
    pub reason: Refusal,
    pub call: Box<dyn Resumable>,
}

impl std::fmt::Debug for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Refused")
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}

/// A call the scheduler runs in steps on its workers.
pub trait Resumable: Send + 'static {
    /// Run until the call finishes, or until it must wait for a dependency.
    ///
    /// Postconditions: on `Wait`, `waker` is registered with the dependency, and no latch is
    /// held. Every step of one call is passed the same waker.
    fn run(&mut self, waker: &Waker) -> Step;

    /// End a call that will not run again: it was cancelled, its deadline passed or the
    /// scheduler shut down while it was queued, ready or waiting, or while it ran a step that
    /// then returned `Wait`. Withdraw any registration of its waker.
    fn stop(self: Box<Self>, why: Stopped);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct CallId(u64);

/// Highest class first, then highest rank, then the oldest transaction, then the earliest
/// admitted.
type Order = (Reverse<Priority>, Reverse<u8>, Instant, CallId);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Queued,
    Ready,
    Running { woken: bool, stop: Option<Stopped> },
    Waiting,
}

struct Slot {
    entry: Entry,
    state: State,
    /// `None` while the call runs.
    call: Option<Box<dyn Resumable>>,
    waker: Waker,
    /// When a waiting call is woken, whether or not its dependency is ready.
    alarm: Option<Instant>,
}

#[derive(Default)]
struct Calls {
    slots: BTreeMap<CallId, Slot>,
    /// Queued and ready calls.
    ready: BTreeSet<Order>,
    /// The deadline of each call with one that is not running.
    deadlines: BTreeSet<(Instant, CallId)>,
    /// The alarm of each waiting call with one.
    alarms: BTreeSet<(Instant, CallId)>,
    queued: usize,
    queued_bytes: usize,
    next: CallId,
    /// The earliest timer the deadline thread has in flight.
    armed: Option<Instant>,
    /// Set by shutdown: no call is admitted or runs another step.
    closed: bool,
}

impl Calls {
    fn order(&self, id: CallId) -> Order {
        let e = &self.slots[&id].entry;
        (Reverse(e.priority), Reverse(e.rank), e.start, id)
    }

    /// Remove call `id`, which is not running. Returns it.
    fn remove(&mut self, id: CallId) -> Box<dyn Resumable> {
        let order = self.order(id);
        let slot = self.slots.remove(&id).expect("the call has a slot");
        self.ready.remove(&order);
        if let Some(d) = slot.entry.deadline {
            self.deadlines.remove(&(d, id));
        }
        if let Some(a) = slot.alarm {
            self.alarms.remove(&(a, id));
        }
        if slot.state == State::Queued {
            self.queued -= 1;
            self.queued_bytes -= slot.entry.bytes;
        }
        slot.call
            .expect("a call that is not running is held by its slot")
    }

    /// Returns the first ready call whose deadline has not passed at `now`, marked running.
    /// Queue on `tasks` the stop of each ready call before it whose deadline has passed.
    fn take(
        &mut self,
        now: Instant,
        tasks: &mut VecDeque<Task>,
    ) -> Option<(CallId, Box<dyn Resumable>, Waker)> {
        let id = loop {
            let (_, _, _, id) = self.ready.pop_first()?;
            if self.slots[&id].entry.deadline.is_some_and(|d| d <= now) {
                tasks.push_back(stopping(self.remove(id), Stopped::DeadlineExceeded));
            } else {
                break id;
            }
        };
        let slot = self.slots.get_mut(&id).expect("a ready call has a slot");
        if slot.state == State::Queued {
            self.queued -= 1;
            self.queued_bytes -= slot.entry.bytes;
        }
        if let Some(d) = slot.entry.deadline {
            self.deadlines.remove(&(d, id));
        }
        slot.state = State::Running {
            woken: false,
            stop: None,
        };
        let call = slot.call.take().expect("a ready call is held by its slot");
        Some((id, call, slot.waker.clone()))
    }

    /// Record `id`'s deadline. Returns whether it is earlier than every armed timer.
    fn watch_deadline(&mut self, id: CallId) -> bool {
        let Some(d) = self.slots[&id].entry.deadline else {
            return false;
        };
        self.deadlines.insert((d, id));
        self.armed.is_none_or(|a| d < a)
    }

    /// Make waiting call `id` ready, withdrawing its alarm.
    fn ready(&mut self, id: CallId) {
        let slot = self.slots.get_mut(&id).expect("a waiting call has a slot");
        slot.state = State::Ready;
        if let Some(a) = slot.alarm.take() {
            self.alarms.remove(&(a, id));
        }
        let order = self.order(id);
        self.ready.insert(order);
    }

    /// Returns the earliest deadline or alarm.
    fn next_timer(&self) -> Option<Instant> {
        let d = self.deadlines.first().map(|(d, _)| *d);
        let a = self.alarms.first().map(|(a, _)| *a);
        d.into_iter().chain(a).min()
    }
}

fn stopping(call: Box<dyn Resumable>, why: Stopped) -> Task {
    Box::new(move || call.stop(why))
}

struct Inner {
    queue: Lock<VecDeque<Task>>,
    calls: Lock<Calls>,
    /// Workers parked, or about to park, on an empty queue.
    idle: Lock<Vec<ThreadId>>,
    stop: AtomicBool,
    ran: AtomicUsize,
    admission: Admission,
    deadline_thread: Lock<Option<ThreadId>>,
}

/// Worker threads that take tasks from one queue, morsels of every running query, and steps of
/// admitted calls.
///
/// Lock order: the task queue, then the calls, then the idle workers.
pub struct Scheduler<R: Runtime> {
    rt: R,
    inner: Arc<Inner>,
    workers: Lock<Vec<ThreadId>>,
    /// The worker and deadline threads it started.
    threads: Vec<ThreadId>,
}

impl<R: Runtime> std::fmt::Debug for Scheduler<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler").finish_non_exhaustive()
    }
}

/// An admitted call.
pub struct Admitted<R: Runtime> {
    rt: R,
    inner: Weak<Inner>,
    id: CallId,
}

impl<R: Runtime> Clone for Admitted<R> {
    fn clone(&self) -> Self {
        Admitted {
            rt: self.rt.clone(),
            inner: self.inner.clone(),
            id: self.id,
        }
    }
}

impl<R: Runtime> std::fmt::Debug for Admitted<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Admitted")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl<R: Runtime> Admitted<R> {
    /// Stop the call with `Cancelled`: at once if it is queued or waiting, else when its
    /// running step returns `Wait` or `WaitUntil`. Does nothing once the call has finished or
    /// stopped.
    pub fn cancel(&self) {
        if let Some(inner) = self.inner.upgrade() {
            inner.stop_call(&self.rt, self.id, Stopped::Cancelled);
        }
    }
}

struct Wakeup<R: Runtime> {
    rt: R,
    inner: Weak<Inner>,
    id: CallId,
}

impl<R: Runtime> Wake for Wakeup<R> {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        if let Some(inner) = self.inner.upgrade() {
            inner.wake(&self.rt, self.id);
        }
    }
}

impl<R: Runtime> Scheduler<R> {
    /// Start `workers` worker threads through the runtime, admitting calls within the default
    /// bounds.
    pub fn start(rt: R, workers: usize) -> Arc<Scheduler<R>> {
        Scheduler::start_with(rt, workers, Admission::default())
    }

    /// Start `workers` worker threads and a deadline thread through the runtime, with
    /// admission bounded by `admission`.
    pub fn start_with(rt: R, workers: usize, admission: Admission) -> Arc<Scheduler<R>> {
        let inner = Arc::new(Inner {
            queue: Lock::new(VecDeque::new()),
            calls: Lock::default(),
            idle: Lock::new(Vec::new()),
            stop: AtomicBool::new(false),
            ran: AtomicUsize::new(0),
            admission,
            deadline_thread: Lock::new(None),
        });
        let mut ids: Vec<ThreadId> = (0..workers)
            .map(|i| {
                let (rt2, inner) = (rt.clone(), inner.clone());
                rt.spawn(&format!("morsel worker {i}"), move || work(&rt2, &inner))
            })
            .collect();
        let (rt2, inner2) = (rt.clone(), inner.clone());
        let deadlines = rt.spawn("call deadlines", move || keep_deadlines(&rt2, &inner2));
        *inner.deadline_thread.lock(&rt) = Some(deadlines);
        ids.push(deadlines);
        Arc::new(Scheduler {
            rt,
            inner,
            workers: Lock::new(ids.clone()),
            threads: ids,
        })
    }

    /// Returns whether `thread` is one of the scheduler's own threads.
    pub fn runs_on(&self, thread: ThreadId) -> bool {
        self.threads.contains(&thread)
    }

    pub fn runtime(&self) -> &R {
        &self.rt
    }

    pub fn admission(&self) -> Admission {
        self.inner.admission
    }

    pub fn submit(&self, task: Task) {
        self.inner.submit(&self.rt, task);
    }

    /// Queue `call` to run once it is first in the ready order and a worker is free.
    ///
    /// Refuse `call`, queueing nothing and handing it back: with `ShutDown` once the scheduler
    /// has shut down, with `TooLarge` when its bytes alone pass the admission byte bound, and
    /// with `Overloaded` when the calls admitted and not yet started would pass the admission
    /// bounds.
    pub fn admit(&self, entry: Entry, call: Box<dyn Resumable>) -> Result<Admitted<R>, Refused> {
        let bound = self.inner.admission;
        let (id, early) = {
            let mut c = self.inner.calls.lock(&self.rt);
            let refusal = if c.closed {
                Some(Refusal::ShutDown)
            } else if entry.bytes > bound.bytes {
                Some(Refusal::TooLarge)
            } else if c.queued >= bound.entries || c.queued_bytes + entry.bytes > bound.bytes {
                Some(Refusal::Overloaded)
            } else {
                None
            };
            if let Some(reason) = refusal {
                return Err(Refused { reason, call });
            }
            let id = c.next;
            c.next = CallId(id.0 + 1);
            let waker = Waker::from(Arc::new(Wakeup {
                rt: self.rt.clone(),
                inner: Arc::downgrade(&self.inner),
                id,
            }));
            c.slots.insert(
                id,
                Slot {
                    entry,
                    state: State::Queued,
                    call: Some(call),
                    waker,
                    alarm: None,
                },
            );
            c.queued += 1;
            c.queued_bytes += entry.bytes;
            let order = c.order(id);
            c.ready.insert(order);
            (id, c.watch_deadline(id))
        };
        self.inner.unpark_idle(&self.rt);
        if early {
            self.inner.unpark_deadlines(&self.rt);
        }
        Ok(Admitted {
            rt: self.rt.clone(),
            inner: Arc::downgrade(&self.inner),
            id,
        })
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

    /// Stop every queued, ready and waiting call with `ShutDown`, and every running call once
    /// its step returns `Wait` or `WaitUntil`; refuse every later call. Then stop the workers
    /// once no task is left, wait for every one but the calling thread to return, and run any
    /// task still queued on the calling thread.
    pub fn shutdown(&self) {
        {
            let mut q = self.inner.queue.lock(&self.rt);
            let mut c = self.inner.calls.lock(&self.rt);
            c.closed = true;
            let idle: Vec<CallId> = c
                .slots
                .iter()
                .filter(|(_, s)| !matches!(s.state, State::Running { .. }))
                .map(|(id, _)| *id)
                .collect();
            for id in idle {
                q.push_back(stopping(c.remove(id), Stopped::ShutDown));
            }
        }
        self.inner.stop.store(true, Ordering::Release);
        let workers = std::mem::take(&mut *self.workers.lock(&self.rt));
        for &w in &workers {
            self.rt.unpark(w);
        }
        let me = self.rt.current();
        for w in workers {
            if w != me {
                self.rt.join(w);
            }
        }
        while self.help() {}
    }
}

impl<R: Runtime> Drop for Scheduler<R> {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Inner {
    fn submit<R: Runtime>(&self, rt: &R, task: Task) {
        self.queue.lock(rt).push_back(task);
        self.unpark_idle(rt);
    }

    fn unpark_idle<R: Runtime>(&self, rt: &R) {
        let idle = std::mem::take(&mut *self.idle.lock(rt));
        for t in idle {
            rt.unpark(t);
        }
    }

    fn unpark_deadlines<R: Runtime>(&self, rt: &R) {
        let thread = *self.deadline_thread.lock(rt);
        if let Some(t) = thread {
            rt.unpark(t);
        }
    }

    fn wake<R: Runtime>(&self, rt: &R, id: CallId) {
        {
            let mut c = self.calls.lock(rt);
            let Some(slot) = c.slots.get_mut(&id) else {
                return;
            };
            match &mut slot.state {
                State::Waiting => {}
                State::Running { woken, .. } => {
                    *woken = true;
                    return;
                }
                State::Queued | State::Ready => return,
            }
            c.ready(id);
        }
        self.unpark_idle(rt);
    }

    fn stop_call<R: Runtime>(&self, rt: &R, id: CallId, why: Stopped) {
        {
            let mut q = self.queue.lock(rt);
            let mut c = self.calls.lock(rt);
            let Some(slot) = c.slots.get_mut(&id) else {
                return;
            };
            if let State::Running { stop, .. } = &mut slot.state {
                stop.get_or_insert(why);
                return;
            }
            q.push_back(stopping(c.remove(id), why));
        }
        self.unpark_idle(rt);
    }

    /// Run one step of call `id`, then queue it again, leave it waiting, stop it or drop it.
    fn step<R: Runtime>(&self, rt: &R, id: CallId, mut call: Box<dyn Resumable>, waker: &Waker) {
        let step = call.run(waker);
        let mut c = self.calls.lock(rt);
        let closed = c.closed;
        let slot = c.slots.get_mut(&id).expect("a running call keeps its slot");
        let State::Running { woken, stop } = slot.state else {
            unreachable!("only the worker running a call changes it from running");
        };
        if step == Step::Done {
            c.slots.remove(&id);
            return;
        }
        let passed = slot.entry.deadline.is_some_and(|d| d <= rt.now());
        let why = stop
            .or(passed.then_some(Stopped::DeadlineExceeded))
            .or(closed.then_some(Stopped::ShutDown));
        if let Some(why) = why {
            c.slots.remove(&id);
            drop(c);
            call.stop(why);
            return;
        }
        slot.call = Some(call);
        slot.state = State::Waiting;
        let alarm = match step {
            Step::WaitUntil(at) if !woken => Some(at),
            _ => None,
        };
        slot.alarm = alarm;
        let mut early = false;
        if woken {
            c.ready(id);
        } else if let Some(at) = alarm {
            c.alarms.insert((at, id));
            early = c.armed.is_none_or(|a| at < a);
        }
        early |= c.watch_deadline(id);
        drop(c);
        if early {
            self.unpark_deadlines(rt);
        }
    }
}

enum Next {
    Task(Task),
    Call(CallId, Box<dyn Resumable>, Waker),
    Park,
}

fn work<R: Runtime>(rt: &R, inner: &Inner) {
    loop {
        let (next, stopped) = {
            let mut q = inner.queue.lock(rt);
            match q.pop_front() {
                Some(t) => (Next::Task(t), false),
                None => {
                    let taken = inner.calls.lock(rt).take(rt.now(), &mut q);
                    let stopped = !q.is_empty();
                    let next = match taken {
                        Some((id, call, waker)) => Next::Call(id, call, waker),
                        None if stopped => continue,
                        None if inner.stop.load(Ordering::Acquire) => return,
                        None => {
                            inner.idle.lock(rt).push(rt.current());
                            Next::Park
                        }
                    };
                    (next, stopped)
                }
            }
        };
        if stopped {
            inner.unpark_idle(rt);
        }
        match next {
            Next::Task(t) => {
                t();
                inner.ran.fetch_add(1, Ordering::AcqRel);
            }
            Next::Call(id, call, waker) => inner.step(rt, id, call, &waker),
            Next::Park => rt.park(),
        }
    }
}

/// Stop queued, ready and waiting calls as their deadlines pass, sleeping on runtime timers
/// between them.
fn keep_deadlines<R: Runtime>(rt: &R, inner: &Inner) {
    let mut io = rt.io();
    let mut timers: BTreeMap<Ticket, Instant> = BTreeMap::new();
    loop {
        while let Some(done) = io.poll() {
            timers.remove(&done.ticket);
        }
        if inner.stop.load(Ordering::Acquire) {
            return;
        }
        let expired = {
            let mut q = inner.queue.lock(rt);
            let mut c = inner.calls.lock(rt);
            let now = rt.now();
            let mut expired = false;
            while let Some(&(d, id)) = c.deadlines.first()
                && d <= now
            {
                q.push_back(stopping(c.remove(id), Stopped::DeadlineExceeded));
                expired = true;
            }
            while let Some(&(a, id)) = c.alarms.first()
                && a <= now
            {
                c.ready(id);
                expired = true;
            }
            if let Some(at) = c.next_timer()
                && timers.values().all(|&t| at < t)
            {
                timers.insert(io.submit(Op::Timer { at }), at);
            }
            c.armed = timers.values().min().copied();
            expired
        };
        if expired {
            inner.unpark_idle(rt);
        }
        rt.park();
    }
}
