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
}

/// Why a call was stopped without running again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stopped {
    Cancelled,
    DeadlineExceeded,
}

/// The admission queue was full.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Overloaded;

/// A call the scheduler runs in steps on its workers.
pub trait Resumable: Send + 'static {
    /// Run until the call finishes, or until it must wait for a dependency.
    ///
    /// Postconditions: on `Wait`, `waker` is registered with the dependency, and no latch is
    /// held. Every step of one call is passed the same waker.
    fn run(&mut self, waker: &Waker) -> Step;

    /// End a call that will not run again: it was cancelled, or its deadline passed, while it
    /// was queued or waiting. Withdraw any registration of its waker.
    fn stop(self: Box<Self>, why: Stopped);
}

type CallId = u64;

/// Highest priority first, then the oldest transaction, then the earliest admitted.
type Order = (Reverse<Priority>, Instant, CallId);

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
}

#[derive(Default)]
struct Calls {
    slots: BTreeMap<CallId, Slot>,
    /// Queued and ready calls.
    ready: BTreeSet<Order>,
    /// The deadline of each call with one that is not running.
    deadlines: BTreeSet<(Instant, CallId)>,
    queued: usize,
    queued_bytes: usize,
    next: CallId,
    /// The earliest timer the deadline thread has in flight.
    armed: Option<Instant>,
}

impl Calls {
    fn order(&self, id: CallId) -> Order {
        let e = &self.slots[&id].entry;
        (Reverse(e.priority), e.start, id)
    }

    /// Remove call `id`, which is not running. Returns it.
    fn remove(&mut self, id: CallId) -> Box<dyn Resumable> {
        let order = self.order(id);
        let slot = self.slots.remove(&id).expect("the call has a slot");
        self.ready.remove(&order);
        if let Some(d) = slot.entry.deadline {
            self.deadlines.remove(&(d, id));
        }
        if slot.state == State::Queued {
            self.queued -= 1;
            self.queued_bytes -= slot.entry.bytes;
        }
        slot.call
            .expect("a call that is not running is held by its slot")
    }

    /// Returns the first ready call, marked running.
    fn take(&mut self) -> Option<(CallId, Box<dyn Resumable>, Waker)> {
        let (_, _, id) = self.ready.pop_first()?;
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

impl<R: Runtime> std::fmt::Debug for Admitted<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Admitted")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl<R: Runtime> Admitted<R> {
    /// Stop the call with `Cancelled`: at once if it is queued or waiting, else when its
    /// running step returns `Wait`. Does nothing once the call has finished or stopped.
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
            workers: Lock::new(ids),
        })
    }

    pub fn runtime(&self) -> &R {
        &self.rt
    }

    pub fn submit(&self, task: Task) {
        self.inner.submit(&self.rt, task);
    }

    /// Queue `call` to run once it is first in the ready order and a worker is free.
    ///
    /// Refuse `call`, queueing nothing, when the calls admitted and not yet started would pass
    /// the scheduler's admission bounds.
    pub fn admit(&self, entry: Entry, call: Box<dyn Resumable>) -> Result<Admitted<R>, Overloaded> {
        let bound = self.inner.admission;
        let (id, early) = {
            let mut c = self.inner.calls.lock(&self.rt);
            if c.queued >= bound.entries || c.queued_bytes + entry.bytes > bound.bytes {
                return Err(Overloaded);
            }
            let id = c.next;
            c.next += 1;
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

    /// Stop the workers once no task or ready call is left, and wait for them to return.
    /// Calls still waiting are dropped without running again.
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
        if let Some(t) = *self.deadline_thread.lock(rt) {
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
                State::Waiting => slot.state = State::Ready,
                State::Running { woken, .. } => {
                    *woken = true;
                    return;
                }
                State::Queued | State::Ready => return,
            }
            let order = c.order(id);
            c.ready.insert(order);
        }
        self.unpark_idle(rt);
    }

    fn stop_call<R: Runtime>(&self, rt: &R, id: CallId, why: Stopped) {
        let call = {
            let mut c = self.calls.lock(rt);
            let Some(slot) = c.slots.get_mut(&id) else {
                return;
            };
            if let State::Running { stop, .. } = &mut slot.state {
                stop.get_or_insert(why);
                return;
            }
            c.remove(id)
        };
        self.submit(rt, Box::new(move || call.stop(why)));
    }

    /// Run one step of call `id`, then queue it again, leave it waiting, stop it or drop it.
    fn step<R: Runtime>(&self, rt: &R, id: CallId, mut call: Box<dyn Resumable>, waker: &Waker) {
        let step = call.run(waker);
        let mut c = self.calls.lock(rt);
        let slot = c.slots.get_mut(&id).expect("a running call keeps its slot");
        let State::Running { woken, stop } = slot.state else {
            unreachable!("only the worker running a call changes it from running");
        };
        if step == Step::Done {
            c.slots.remove(&id);
            return;
        }
        let passed = slot.entry.deadline.is_some_and(|d| d <= rt.now());
        if let Some(why) = stop.or(passed.then_some(Stopped::DeadlineExceeded)) {
            c.slots.remove(&id);
            drop(c);
            call.stop(why);
            return;
        }
        slot.call = Some(call);
        if woken {
            slot.state = State::Ready;
            let order = c.order(id);
            c.ready.insert(order);
        } else {
            slot.state = State::Waiting;
        }
        let early = c.watch_deadline(id);
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
        let next = {
            let mut q = inner.queue.lock(rt);
            match q.pop_front() {
                Some(t) => Next::Task(t),
                None => match inner.calls.lock(rt).take() {
                    Some((id, call, waker)) => Next::Call(id, call, waker),
                    None if inner.stop.load(Ordering::Acquire) => return,
                    None => {
                        inner.idle.lock(rt).push(rt.current());
                        Next::Park
                    }
                },
            }
        };
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
            let mut c = inner.calls.lock(rt);
            let now = rt.now();
            let mut expired = Vec::new();
            while let Some(&(d, id)) = c.deadlines.first()
                && d <= now
            {
                expired.push(c.remove(id));
            }
            if let Some(&(at, _)) = c.deadlines.first()
                && timers.values().all(|&t| at < t)
            {
                timers.insert(io.submit(Op::Timer { at }), at);
            }
            c.armed = timers.values().min().copied();
            expired
        };
        for call in expired {
            inner.submit(rt, Box::new(move || call.stop(Stopped::DeadlineExceeded)));
        }
        rt.park();
    }
}
