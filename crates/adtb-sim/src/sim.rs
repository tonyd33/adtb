use std::any::Any;
use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write as _;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime};

use adtb::runtime::{
    AlignedBuf, Clock, Completion, Done, Entropy, FileId, Instant, Io, Op, Runtime, ThreadId,
    Threads, Ticket,
};

use crate::disk::{Ctx, Disk, Faults, Geometry, InFlight, Ino, Recording};
use crate::rng::Rng;

const WALL_ORIGIN: Duration = Duration::from_secs(1_767_225_600);

#[derive(Clone, Debug)]
pub struct Config {
    pub faults: Faults,
    /// The device's logical block size; every read and write is aligned to it.
    pub sector: u64,
    /// The largest write the device lands whole or not at all.
    pub atomic_unit: u64,
    /// The range of completion delays when `faults.delay_completions` is set.
    pub latency: (Duration, Duration),
    /// The most simulated time one scheduling step takes.
    pub jitter: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            faults: Faults::default(),
            sector: 512,
            atomic_unit: 512,
            latency: (Duration::from_micros(1), Duration::from_micros(500)),
            jitter: Duration::from_micros(1),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Finished,
    Crashed,
}

struct Halt;

enum Phase {
    Idle,
    Running,
    Finished,
    Crashed,
    Panicked(Box<dyn Any + Send>),
    Deadlocked(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Runnable,
    Parked,
    Joining(usize),
    Done,
}

struct SimThread {
    name: String,
    status: Status,
    token: bool,
}

struct Event {
    ring: usize,
    ticket: u64,
    submitter: usize,
    op: Op,
    upto: u64,
}

struct State {
    config: Config,
    sched: Rng,
    faults: Rng,
    entropy: Rng,
    now: u64,
    disk: Disk,
    trace: String,
    crash_at: Option<u64>,
    process: u64,
    phase: Phase,
    threads: Vec<SimThread>,
    current: usize,
    events: BTreeMap<(u64, u64), Event>,
    rings: Vec<VecDeque<Completion>>,
    handles: Vec<Ino>,
    next_ticket: u64,
}

struct Shared {
    state: Mutex<State>,
    turn: Condvar,
    os: Mutex<Vec<JoinHandle<()>>>,
}

type Guard<'a> = MutexGuard<'a, State>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).expect("duration overflows u64 nanoseconds")
}

/// A deterministic simulation of one machine: its clock, randomness, threads and storage.
///
/// The storage outlives each run; a run after a crash restarts the process over what the crash left.
pub struct Sim {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Sim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sim")
            .field("now", &self.now())
            .finish_non_exhaustive()
    }
}

impl Sim {
    /// Returns a simulation whose device holds `files`, every byte forced.
    pub fn with_files(seed: u64, config: Config, files: BTreeMap<String, Vec<u8>>) -> Self {
        let sim = Sim::new(seed, config);
        lock(&sim.shared.state).disk = Disk::from_files(files);
        sim
    }

    /// Record every write, name change and force from now on, taking everything before as forced.
    pub fn record(&mut self) {
        lock(&self.shared.state).disk.record();
    }

    pub fn recording(&self) -> Recording {
        lock(&self.shared.state)
            .disk
            .recording()
            .cloned()
            .unwrap_or_default()
    }

    pub fn new(seed: u64, config: Config) -> Self {
        assert!(
            config.sector > 0 && (AlignedBuf::ALIGN as u64).is_multiple_of(config.sector),
            "sector size must divide the I/O buffer alignment"
        );
        assert!(
            config.atomic_unit.is_multiple_of(config.sector),
            "atomic unit must be whole sectors"
        );
        let mut root = Rng::new(seed);
        let state = State {
            config,
            sched: root.split(),
            faults: root.split(),
            entropy: root.split(),
            now: 0,
            disk: Disk::default(),
            trace: String::new(),
            crash_at: None,
            process: 0,
            phase: Phase::Idle,
            threads: Vec::new(),
            current: 0,
            events: BTreeMap::new(),
            rings: Vec::new(),
            handles: Vec::new(),
            next_ticket: 0,
        };
        Sim {
            shared: Arc::new(Shared {
                state: Mutex::new(state),
                turn: Condvar::new(),
                os: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn now(&self) -> Instant {
        Instant::from_nanos(lock(&self.shared.state).now)
    }

    /// Returns every line the runs so far wrote: schedule decisions, ops, completions and faults.
    pub fn trace(&self) -> String {
        lock(&self.shared.state).trace.clone()
    }

    /// Returns the contents of every file as a reader would see it now.
    pub fn files(&self) -> BTreeMap<String, Vec<u8>> {
        lock(&self.shared.state).disk.files()
    }

    pub fn set_faults(&mut self, faults: Faults) {
        lock(&self.shared.state).config.faults = faults;
    }

    /// Crash the next run once the clock reaches `at`.
    pub fn crash_at(&mut self, at: Instant) {
        lock(&self.shared.state).crash_at = Some(at.as_nanos());
    }

    /// Run `body` as the first thread of a new process, until every thread has returned or the process crashes.
    ///
    /// Panics with the payload of any thread that panics, or when every live thread is blocked with no
    /// completion or timer pending.
    pub fn run<F: FnOnce(SimRuntime)>(&mut self, body: F) -> Outcome {
        let process = {
            let mut st = lock(&self.shared.state);
            st.process += 1;
            st.phase = Phase::Running;
            st.threads = vec![SimThread {
                name: "main".into(),
                status: Status::Runnable,
                token: false,
            }];
            st.current = 0;
            st.rings.clear();
            st.handles.clear();
            let (now, process) = (st.now, st.process);
            let _ = writeln!(st.trace, "t={now} process {process} starts");
            process
        };
        let rt = SimRuntime {
            shared: self.shared.clone(),
            process,
        };
        let result = panic::catch_unwind(AssertUnwindSafe(|| body(rt.clone())));
        settle(&self.shared, 0, result);
        {
            let st = lock(&self.shared.state);
            drop(
                self.shared
                    .turn
                    .wait_while(st, |st| matches!(st.phase, Phase::Running)),
            );
        }
        let handles = std::mem::take(&mut *lock(&self.shared.os));
        for h in handles {
            let _ = h.join();
        }
        let mut st = lock(&self.shared.state);
        st.crash_at = None;
        match std::mem::replace(&mut st.phase, Phase::Idle) {
            Phase::Finished => Outcome::Finished,
            Phase::Crashed => Outcome::Crashed,
            Phase::Panicked(payload) => {
                drop(st);
                panic::resume_unwind(payload)
            }
            Phase::Deadlocked(why) => {
                drop(st);
                panic!("deadlock: {why}")
            }
            Phase::Idle | Phase::Running => unreachable!(),
        }
    }
}

fn settle(shared: &Shared, me: usize, result: thread::Result<()>) {
    let mut st = lock(&shared.state);
    if !matches!(st.phase, Phase::Running) {
        return;
    }
    match result {
        Ok(()) => {
            st.threads[me].status = Status::Done;
            for t in &mut st.threads {
                if t.status == Status::Joining(me) {
                    t.status = Status::Runnable;
                }
            }
            drop(switch(shared, st, me));
        }
        Err(payload) if payload.is::<Halt>() => {}
        Err(payload) => {
            st.crash();
            st.phase = Phase::Panicked(payload);
            shared.turn.notify_all();
        }
    }
}

/// Pass the processor from the current thread `me` to the next, and wait for it to come back.
///
/// Returns `Err(Halt)` when the process has crashed or stopped meanwhile, having released the lock.
fn switch<'a>(shared: &'a Shared, mut st: Guard<'a>, me: usize) -> Result<Guard<'a>, Halt> {
    let jitter = nanos(st.config.jitter);
    let step = st.sched.between(0, jitter);
    let target = st.now + step;
    if !st.advance(target) {
        shared.turn.notify_all();
        return Err(Halt);
    }
    let next = loop {
        let runnable: Vec<usize> = (0..st.threads.len())
            .filter(|&i| st.threads[i].status == Status::Runnable)
            .collect();
        if !runnable.is_empty() {
            let i = st.sched.below(runnable.len() as u64) as usize;
            break runnable[i];
        }
        if let Some((&(due, _), _)) = st.events.first_key_value() {
            if !st.advance(due) {
                shared.turn.notify_all();
                return Err(Halt);
            }
            continue;
        }
        if st.threads.iter().all(|t| t.status == Status::Done) {
            st.phase = Phase::Finished;
        } else {
            let blocked: Vec<String> = st
                .threads
                .iter()
                .enumerate()
                .filter(|(_, t)| t.status != Status::Done)
                .map(|(i, t)| format!("thread {i} {:?} {:?}", t.name, t.status))
                .collect();
            st.phase = Phase::Deadlocked(blocked.join(", "));
        }
        shared.turn.notify_all();
        return Err(Halt);
    };
    if next != me {
        let now = st.now;
        let _ = writeln!(st.trace, "t={now} run {next}");
    }
    st.current = next;
    shared.turn.notify_all();
    if st.threads[me].status == Status::Done {
        return Ok(st);
    }
    let st = shared
        .turn
        .wait_while(st, |st| {
            matches!(st.phase, Phase::Running) && st.current != me
        })
        .unwrap_or_else(|e| e.into_inner());
    if matches!(st.phase, Phase::Running) {
        Ok(st)
    } else {
        Err(Halt)
    }
}

impl State {
    /// Move the clock to `target`, delivering every event due by then.
    ///
    /// Returns false if the crash time came first and the process crashed.
    fn advance(&mut self, target: u64) -> bool {
        let stop = self.crash_at.map_or(target, |c| c.min(target));
        while let Some(entry) = self.events.first_entry() {
            let (due, _) = *entry.key();
            if due > stop {
                break;
            }
            let event = entry.remove();
            self.now = self.now.max(due);
            self.deliver(event);
        }
        self.now = self.now.max(stop);
        if self.crash_at.is_some_and(|c| c <= target) {
            self.crash();
            return false;
        }
        true
    }

    fn ctx(&mut self) -> (&mut Disk, Ctx<'_>) {
        let geometry = Geometry {
            sector: self.config.sector,
            atomic_unit: self.config.atomic_unit,
        };
        (
            &mut self.disk,
            Ctx {
                rng: &mut self.faults,
                faults: &self.config.faults,
                geometry,
                trace: &mut self.trace,
            },
        )
    }

    fn ino(&self, file: FileId) -> Ino {
        *self
            .handles
            .get(file.0 as usize)
            .unwrap_or_else(|| panic!("{file:?} was not opened by this process"))
    }

    fn deliver(&mut self, event: Event) {
        let Event {
            ring,
            ticket,
            submitter,
            op,
            upto,
        } = event;
        let now = self.now;
        let _ = writeln!(self.trace, "t={now} complete {ticket}");
        let (buf, result) = match op {
            Op::Open { path, create } => {
                let r = self.disk.open(&path, create).map(|(ino, len)| {
                    self.handles.push(ino);
                    Done::Opened {
                        file: FileId((self.handles.len() - 1) as u32),
                        len,
                    }
                });
                (None, r)
            }
            Op::Read {
                file,
                offset,
                mut buf,
            } => {
                let ino = self.ino(file);
                let (disk, cx) = self.ctx();
                let r = disk.read(ino, offset, &mut buf, cx).map(Done::Read);
                (Some(buf), r)
            }
            Op::Write { file, offset, buf } => {
                let ino = self.ino(file);
                let (disk, cx) = self.ctx();
                let r = disk.write(ino, offset, &buf, cx).map(|()| Done::Written);
                (Some(buf), r)
            }
            Op::Allocate { file, len } => {
                let ino = self.ino(file);
                self.disk.allocate(ino, len, self.config.sector);
                (None, Ok(Done::Allocated))
            }
            Op::Sync { file } => {
                let ino = self.ino(file);
                let (disk, cx) = self.ctx();
                (None, disk.sync(ino, upto, cx).map(|()| Done::Synced))
            }
            Op::SyncDir { dir } => {
                let (disk, cx) = self.ctx();
                (None, disk.sync_dir(&dir, upto, cx).map(|()| Done::Synced))
            }
            Op::List { dir } => (
                None,
                Ok(Done::Listed(self.disk.list(&dir, &mut self.faults))),
            ),
            Op::Remove { path } => (None, self.disk.remove(&path).map(|()| Done::Removed)),
            Op::Timer { .. } => (None, Ok(Done::Fired)),
        };
        let _ = writeln!(self.trace, "  {result:?}");
        self.rings[ring].push_back(Completion {
            ticket: Ticket(ticket),
            buf,
            result,
        });
        self.unpark(submitter);
    }

    fn unpark(&mut self, t: usize) {
        let thread = &mut self.threads[t];
        match thread.status {
            Status::Parked => thread.status = Status::Runnable,
            Status::Done => {}
            _ => thread.token = true,
        }
    }

    fn crash(&mut self) {
        let now = self.now;
        let _ = writeln!(self.trace, "t={now} crash");
        let events = std::mem::take(&mut self.events);
        let in_flight = events
            .into_values()
            .filter_map(|e| match e.op {
                Op::Write { file, offset, buf } => Some(InFlight {
                    ino: self.ino(file),
                    offset,
                    data: buf.to_vec(),
                }),
                _ => None,
            })
            .collect();
        let (disk, cx) = self.ctx();
        disk.crash(in_flight, cx);
        self.rings.clear();
        self.phase = Phase::Crashed;
    }
}

/// The runtime handle the engine runs on inside a [`Sim`].
#[derive(Clone)]
pub struct SimRuntime {
    shared: Arc<Shared>,
    process: u64,
}

impl std::fmt::Debug for SimRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimRuntime")
            .field("process", &self.process)
            .finish_non_exhaustive()
    }
}

impl SimRuntime {
    /// Returns the state lock if the process is running and the caller may act on it.
    fn enter(&self) -> Option<Guard<'_>> {
        if thread::panicking() {
            return None;
        }
        let st = lock(&self.shared.state);
        if !matches!(st.phase, Phase::Running) || st.process != self.process {
            return None;
        }
        Some(st)
    }

    fn reschedule<'a>(&'a self, st: Guard<'a>) -> Guard<'a> {
        let me = st.current;
        match switch(&self.shared, st, me) {
            Ok(st) => st,
            Err(Halt) => panic::resume_unwind(Box::new(Halt)),
        }
    }
}

impl SimRuntime {
    /// Returns the number of events recorded so far, or 0 when the simulation is not recording.
    pub fn recorded(&self) -> usize {
        lock(&self.shared.state)
            .disk
            .recording()
            .map_or(0, |r| r.len())
    }
}

impl Clock for SimRuntime {
    fn now(&self) -> Instant {
        Instant::from_nanos(lock(&self.shared.state).now)
    }

    fn wall(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH + WALL_ORIGIN + Duration::from_nanos(lock(&self.shared.state).now)
    }
}

impl Entropy for SimRuntime {
    fn next_u64(&self) -> u64 {
        lock(&self.shared.state).entropy.next_u64()
    }
}

impl Threads for SimRuntime {
    fn spawn<F>(&self, name: &str, body: F) -> ThreadId
    where
        F: FnOnce() + Send + 'static,
    {
        let Some(mut st) = self.enter() else {
            return ThreadId(u64::MAX);
        };
        let id = st.threads.len();
        st.threads.push(SimThread {
            name: name.to_owned(),
            status: Status::Runnable,
            token: false,
        });
        let now = st.now;
        let _ = writeln!(st.trace, "t={now} spawn {id} {name:?}");
        let shared = self.shared.clone();
        #[allow(
            clippy::disallowed_methods,
            reason = "each simulated thread runs on a host thread"
        )]
        let handle = thread::Builder::new()
            .name(format!("sim-{id}-{name}"))
            .spawn(move || thread_main(&shared, id, body))
            .expect("the host refused a thread");
        lock(&self.shared.os).push(handle);
        drop(self.reschedule(st));
        ThreadId(id as u64)
    }

    fn current(&self) -> ThreadId {
        ThreadId(lock(&self.shared.state).current as u64)
    }

    fn park(&self) {
        let Some(mut st) = self.enter() else { return };
        let me = st.current;
        let thread = &mut st.threads[me];
        if thread.token {
            thread.token = false;
        } else {
            thread.status = Status::Parked;
        }
        drop(self.reschedule(st));
    }

    fn unpark(&self, thread: ThreadId) {
        let Some(mut st) = self.enter() else { return };
        st.unpark(thread.0 as usize);
        drop(self.reschedule(st));
    }

    fn yield_now(&self) {
        let Some(st) = self.enter() else { return };
        drop(self.reschedule(st));
    }

    fn join(&self, thread: ThreadId) {
        let Some(mut st) = self.enter() else { return };
        let me = st.current;
        let target = thread.0 as usize;
        assert!(target != me, "a thread cannot join itself");
        if st.threads[target].status != Status::Done {
            st.threads[me].status = Status::Joining(target);
        }
        drop(self.reschedule(st));
    }
}

fn thread_main<F: FnOnce()>(shared: &Shared, id: usize, body: F) {
    {
        let st = lock(&shared.state);
        let st = shared
            .turn
            .wait_while(st, |st| {
                matches!(st.phase, Phase::Running) && st.current != id
            })
            .unwrap_or_else(|e| e.into_inner());
        if !matches!(st.phase, Phase::Running) {
            return;
        }
    }
    let result = panic::catch_unwind(AssertUnwindSafe(body));
    settle(shared, id, result);
}

impl Runtime for SimRuntime {
    type Io = SimIo;

    fn io(&self) -> SimIo {
        let mut st = lock(&self.shared.state);
        st.rings.push(VecDeque::new());
        SimIo {
            rt: self.clone(),
            ring: st.rings.len() - 1,
        }
    }
}

/// One I/O queue of a [`SimRuntime`].
#[derive(Debug)]
pub struct SimIo {
    rt: SimRuntime,
    ring: usize,
}

impl Io for SimIo {
    fn submit(&mut self, op: Op) -> Ticket {
        let Some(mut st) = self.rt.enter() else {
            return Ticket(u64::MAX);
        };
        let ticket = st.next_ticket;
        st.next_ticket += 1;
        let now = st.now;
        let due = match op {
            Op::Timer { at } => now.max(at.as_nanos()),
            _ if st.config.faults.delay_completions => {
                let (lo, hi) = st.config.latency;
                now + st.sched.between(nanos(lo), nanos(hi))
            }
            _ => now,
        };
        let _ = writeln!(
            st.trace,
            "t={now} submit {ticket} ring {} due {due} {op:?}",
            self.ring
        );
        let upto = st.disk.seq();
        let submitter = st.current;
        st.events.insert(
            (due, ticket),
            Event {
                ring: self.ring,
                ticket,
                submitter,
                op,
                upto,
            },
        );
        drop(self.rt.reschedule(st));
        Ticket(ticket)
    }

    fn poll(&mut self) -> Option<Completion> {
        let st = self.rt.enter()?;
        let mut st = self.rt.reschedule(st);
        st.rings[self.ring].pop_front()
    }
}
