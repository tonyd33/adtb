//! Schedule simulated threads and I/O deterministically from a seed.

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write as _;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime};

use adtb::runtime::{
    AlignedBuf, Clock, Completion, Done, Entropy, Faults, FileId, Instant, Io, IoError, Op, OpKind,
    Runtime, ThreadId, Threads, Ticket,
};
use adtb_sys::pages;

use crate::disk::{Ctx, Disk, InFlight, Ino, Recording};
use crate::plan::{Action, Count, Match, Numbered, OpLog, Plan, Planner};
use crate::rng::Rng;

fn check_geometry(faults: &Faults) {
    assert!(
        faults.sector > 0 && (AlignedBuf::ALIGN as u64).is_multiple_of(faults.sector),
        "sector size must divide the I/O buffer alignment"
    );
    assert!(
        faults.atomic_unit.is_multiple_of(faults.sector),
        "atomic unit must be whole sectors"
    );
}

const WALL_ORIGIN: Duration = Duration::from_secs(1_767_225_600);

#[derive(Clone, Debug)]
pub struct Config {
    /// The device's faults and geometry. Its `sector` is the device's logical block size, to
    /// which every read and write is aligned, and it stays fixed for the simulation.
    pub faults: Faults,
    /// Complete ops after random delays, and so out of submission order.
    pub delay_completions: bool,
    /// The range of completion delays when `delay_completions` is set.
    pub latency: (Duration, Duration),
    /// The most simulated time one scheduling step takes.
    pub jitter: Duration,
    /// The chance that a thread offers the processor to another at each switch point it passes
    /// in page memory.
    pub switch_rate: f64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            faults: Faults::none(512),
            delay_completions: true,
            latency: (Duration::from_micros(1), Duration::from_micros(500)),
            jitter: Duration::from_micros(1),
            switch_rate: 0.0,
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
    Held(String),
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
    daemon: bool,
    status: Status,
    token: bool,
    turn: Arc<Condvar>,
}

struct Event {
    ring: usize,
    ticket: u64,
    submitter: usize,
    op: Op,
    upto: u64,
    injected: Option<Injected>,
    logged: Vec<(OpLog, usize)>,
}

#[expect(
    clippy::disallowed_methods,
    reason = "permanent: the simulator makes the tickets of the ops submitted to it"
)]
fn ticket(n: u64) -> Ticket {
    Ticket::new(n)
}

/// The outcome a plan gives an op in place of running it on the disk.
#[derive(Clone, Copy, Debug)]
enum Injected {
    Fail(IoError),
    Lose,
}

struct State {
    config: Config,
    sched: Rng,
    faults: Rng,
    entropy: Rng,
    switches: Rng,
    now: u64,
    disk: Disk,
    trace: String,
    crash_at: Option<u64>,
    process: u64,
    phase: Phase,
    threads: Vec<SimThread>,
    current: usize,
    events: BTreeMap<(u64, u64), Event>,
    held: Vec<Event>,
    rings: Vec<VecDeque<Completion>>,
    handles: Vec<Option<(Ino, String)>>,
    next_ticket: u64,
}

/// Lock order: `plan` is never taken while `state` is held.
struct Shared {
    state: Mutex<State>,
    plan: Mutex<Planner>,
    turn: Condvar,
    os: Mutex<Vec<JoinHandle<()>>>,
}

type Guard<'a> = MutexGuard<'a, State>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
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
    ///
    /// Preconditions:
    /// - No crash happens while recording: `images` panics if it replays a lost write.
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
        check_geometry(&config.faults);
        let mut root = Rng::new(seed);
        let state = State {
            config,
            sched: root.split(),
            faults: root.split(),
            entropy: root.split(),
            switches: root.split(),
            now: 0,
            disk: Disk::default(),
            trace: String::new(),
            crash_at: None,
            process: 0,
            phase: Phase::Idle,
            threads: Vec::new(),
            current: 0,
            events: BTreeMap::new(),
            held: Vec::new(),
            rings: Vec::new(),
            handles: Vec::new(),
            next_ticket: 0,
        };
        Sim {
            shared: Arc::new(Shared {
                state: Mutex::new(state),
                plan: Mutex::default(),
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

    /// Returns every directory as a reader would see it now.
    pub fn dirs(&self) -> BTreeSet<String> {
        lock(&self.shared.state).disk.dirs()
    }

    /// Change the faults the device injects from now on.
    ///
    /// Panic if `faults` changes the device's sector or atomic unit.
    pub fn set_faults(&mut self, faults: Faults) {
        lock(&self.shared.state).set_faults(faults);
    }

    /// Crash the next run once the clock reaches `at`.
    pub fn crash_at(&mut self, at: Instant) {
        lock(&self.shared.state).crash_at = Some(at.as_nanos());
    }

    /// Act on the ops of the next run as `plan` states, in place of the plan before. The rules end
    /// with that run; the numbered actions last until taken or disarmed.
    pub fn plan(&mut self, plan: Plan) {
        lock(&self.shared.plan).plan(plan);
    }

    /// Act as [`Sim::plan`] does, and panic at any later plan of this simulation, naming `owner`.
    pub fn own_plan(&mut self, owner: &'static str, plan: Plan) {
        lock(&self.shared.plan).own(owner, plan);
    }

    /// Returns the ops numbered so far, in this run and earlier ones.
    pub fn numbered(&self) -> Numbered {
        lock(&self.shared.plan).numbered()
    }

    /// Returns a count of the ops `ops` matches submitted from now on, in this run and later ones.
    pub fn counted(&mut self, ops: Match) -> Count {
        lock(&self.shared.plan).counted(ops)
    }

    /// Returns a log of the ops `ops` matches submitted from now on, in this run and later ones.
    pub fn logged(&mut self, ops: Match) -> OpLog {
        lock(&self.shared.plan).logged(ops)
    }

    /// Run `body` as the first thread of a new process, until every thread but the idle daemons has
    /// returned or the process crashes. A daemon's timer left pending at the end does not move the
    /// clock.
    ///
    /// Panics with the payload of any thread that panics, or when a live thread that is no daemon is
    /// blocked and only a daemon's timer can still fire, and when its threads return with an op
    /// held, whether or not a crash is scheduled.
    pub fn run<F: FnOnce(SimRuntime)>(&mut self, body: F) -> Outcome {
        let process = {
            let mut st = lock(&self.shared.state);
            st.process += 1;
            st.phase = Phase::Running;
            st.threads = vec![SimThread {
                name: "main".into(),
                daemon: false,
                status: Status::Runnable,
                token: false,
                turn: Arc::new(Condvar::new()),
            }];
            st.current = 0;
            st.rings.clear();
            st.handles.clear();
            let (now, process) = (st.now, st.process);
            trace!(st.trace, "t={now} process {process} starts");
            process
        };
        let rt = SimRuntime {
            shared: self.shared.clone(),
            process,
        };
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            let _switches = rt.switch_points();
            body(rt.clone());
        }));
        settle(&self.shared, 0, result);
        {
            let st = lock(&self.shared.state);
            drop(
                self.shared
                    .turn
                    .wait_while(st, |st| matches!(st.phase, Phase::Running)),
            );
        }
        lock(&self.shared.plan).end_run();
        let handles = std::mem::take(&mut *lock(&self.shared.os));
        for h in handles {
            if let Err(payload) = h.join() {
                panic::resume_unwind(payload);
            }
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
            Phase::Held(ops) => {
                drop(st);
                panic!("the run ends with an op held: {ops}")
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
            wake_all(shared, &st);
        }
    }
}

/// Wake every thread and the waiting run, after the process has left `Phase::Running`.
fn wake_all(shared: &Shared, st: &State) {
    for t in &st.threads {
        t.turn.notify_all();
    }
    shared.turn.notify_all();
}

/// Pass the processor from the current thread `me` to the next, and wait for it to come back.
///
/// Returns `Err(Halt)` when the process has crashed or stopped meanwhile, having released the lock.
fn switch<'a>(shared: &'a Shared, mut st: Guard<'a>, me: usize) -> Result<Guard<'a>, Halt> {
    let jitter = nanos(st.config.jitter);
    let step = st.sched.between(0, jitter);
    let target = st.now + step;
    if !st.advance(target) {
        wake_all(shared, &st);
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
        if st.events.values().any(|e| !st.is_daemon_timer(e))
            && let Some((&(due, _), _)) = st.events.first_key_value()
        {
            if !st.advance(due) {
                wake_all(shared, &st);
                return Err(Halt);
            }
            continue;
        }
        let ended = st
            .threads
            .iter()
            .all(|t| t.daemon || t.status == Status::Done);
        if !ended
            && !st.held.is_empty()
            && let Some(crash) = st.crash_at
        {
            let crashed = !st.advance(crash);
            assert!(crashed, "advancing the clock to the crash crashes");
            wake_all(shared, &st);
            return Err(Halt);
        }
        st.events.clear();
        if !st.held.is_empty() {
            let held = std::mem::take(&mut st.held);
            let ops: Vec<String> = held
                .iter()
                .map(|e| {
                    format!(
                        "{:?} {:?}",
                        e.op.kind(),
                        st.path_of(&e.op).unwrap_or_default()
                    )
                })
                .collect();
            st.phase = Phase::Held(ops.join(", "));
        } else if ended {
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
        wake_all(shared, &st);
        return Err(Halt);
    };
    if next != me {
        let now = st.now;
        trace!(st.trace, "t={now} run {next}");
    }
    st.current = next;
    if next != me {
        st.threads[next].turn.notify_one();
    }
    if st.threads[me].status == Status::Done {
        return Ok(st);
    }
    let turn = Arc::clone(&st.threads[me].turn);
    let st = turn
        .wait_while(st, |st| {
            matches!(st.phase, Phase::Running) && st.current != me
        })
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if matches!(st.phase, Phase::Running) {
        Ok(st)
    } else {
        Err(Halt)
    }
}

impl State {
    fn is_daemon_timer(&self, event: &Event) -> bool {
        matches!(event.op, Op::Timer { .. }) && self.threads[event.submitter].daemon
    }

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
        (
            &mut self.disk,
            Ctx {
                rng: &mut self.faults,
                faults: &self.config.faults,
                trace: &mut self.trace,
            },
        )
    }

    fn set_faults(&mut self, faults: Faults) {
        let now = &self.config.faults;
        assert!(
            (faults.sector, faults.atomic_unit) == (now.sector, now.atomic_unit),
            "the device's geometry is fixed for a simulation: sector {} and atomic unit {}, not {} and {}",
            now.sector,
            now.atomic_unit,
            faults.sector,
            faults.atomic_unit
        );
        self.config.faults = faults;
    }

    fn ino(&self, file: FileId) -> Ino {
        self.handles
            .get(file.0 as usize)
            .and_then(|h| h.as_ref())
            .map(|(ino, _)| *ino)
            .unwrap_or_else(|| panic!("{file:?} is not open in this process"))
    }

    /// Returns the path `op` names: the path a file was opened at, and a rename's new name.
    fn path_of(&self, op: &Op) -> Option<String> {
        match op {
            Op::Open { path, .. }
            | Op::CreateNew { path }
            | Op::Remove { path }
            | Op::RemoveDir { path } => Some(path.clone()),
            Op::SyncDir { dir } | Op::List { dir } | Op::CreateDir { dir } => Some(dir.clone()),
            Op::Rename { to, .. } => Some(to.clone()),
            Op::Read { file, .. }
            | Op::Write { file, .. }
            | Op::Size { file }
            | Op::Allocate { file, .. }
            | Op::Sync { file }
            | Op::Close { file } => self
                .handles
                .get(file.0 as usize)
                .and_then(|h| h.as_ref())
                .map(|(_, path)| path.clone()),
            Op::Timer { .. } => None,
        }
    }

    fn opened(&mut self, ino: Ino, path: String, len: u64) -> Done {
        self.handles.push(Some((ino, path)));
        Done::Opened {
            file: FileId((self.handles.len() - 1) as u32),
            len,
        }
    }

    fn deliver(&mut self, event: Event) {
        let Event {
            ring,
            ticket: id,
            submitter,
            op,
            upto,
            injected,
            logged,
        } = event;
        let now = self.now;
        trace!(self.trace, "t={now} complete {id}");
        for (log, at) in logged {
            log.complete(at, Instant::from_nanos(now));
        }
        let (buf, result) = match injected {
            None => self.perform(op, upto),
            Some(injected) => {
                trace!(self.trace, "  planned {injected:?}");
                let buf = match op {
                    Op::Read { buf, .. } | Op::Write { buf, .. } => Some(buf),
                    _ => None,
                };
                let result = match injected {
                    Injected::Fail(error) => Err(error),
                    Injected::Lose => Ok(Done::Written),
                };
                (buf, result)
            }
        };
        trace!(self.trace, "  {result:?}");
        self.rings[ring].push_back(Completion {
            ticket: ticket(id),
            buf,
            result,
        });
        self.unpark(submitter);
    }

    /// Run `op` on the disk. Returns its buffer and result.
    fn perform(&mut self, op: Op, upto: u64) -> (Option<AlignedBuf>, Result<Done, IoError>) {
        match op {
            Op::Open { path, create } => {
                let r = self.disk.open(&path, create);
                (None, r.map(|(ino, len)| self.opened(ino, path, len)))
            }
            Op::CreateNew { path } => {
                let r = self.disk.create_new(&path);
                (None, r.map(|(ino, len)| self.opened(ino, path, len)))
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
            Op::Size { file } => (None, Ok(Done::Size(self.disk.size(self.ino(file))))),
            Op::Allocate { file, len } => {
                let ino = self.ino(file);
                let (disk, cx) = self.ctx();
                (None, disk.allocate(ino, len, cx).map(|()| Done::Allocated))
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
                self.disk.list(&dir, &mut self.faults).map(Done::Listed),
            ),
            Op::Remove { path } => (None, self.disk.remove(&path).map(|()| Done::Removed)),
            Op::RemoveDir { path } => (None, self.disk.remove_dir(&path).map(|()| Done::Removed)),
            Op::Rename { from, to, replace } => (
                None,
                self.disk
                    .rename(&from, &to, replace)
                    .map(|()| Done::Renamed),
            ),
            Op::CreateDir { dir } => (
                None,
                self.disk
                    .create_dir(&dir)
                    .map(|made| if made { Done::Created } else { Done::Present }),
            ),
            Op::Close { file } => {
                self.close(file);
                (None, Ok(Done::Closed))
            }
            Op::Timer { .. } => (None, Ok(Done::Fired)),
        }
    }

    fn close(&mut self, file: FileId) {
        self.ino(file);
        let busy = self.events.values().chain(&self.held).any(|e| match &e.op {
            Op::Read { file: f, .. }
            | Op::Write { file: f, .. }
            | Op::Size { file: f }
            | Op::Allocate { file: f, .. }
            | Op::Sync { file: f }
            | Op::Close { file: f } => *f == file,
            _ => false,
        });
        assert!(!busy, "{file:?} is closed with an op on it in flight");
        self.handles[file.0 as usize] = None;
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
        trace!(self.trace, "t={now} crash");
        let events = std::mem::take(&mut self.events);
        let held = std::mem::take(&mut self.held);
        let in_flight = events
            .into_values()
            .chain(held)
            .filter(|e| e.injected.is_none())
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

    /// Install the calling thread's switch points in page memory if the configuration switches
    /// there.
    fn switch_points(&self) -> Option<pages::SwitchPoints> {
        let rate = lock(&self.shared.state).config.switch_rate;
        let rt = self.clone();
        (rate > 0.0).then(|| pages::switch_points(move || rt.switch_point()))
    }

    fn switch_point(&self) {
        let Some(mut st) = self.enter() else { return };
        let rate = st.config.switch_rate;
        if st.switches.chance(rate) {
            drop(self.reschedule(st));
        }
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
    /// Crash this run once the clock has moved `after` past now, in place of any crash scheduled
    /// before.
    pub fn crash_after(&self, after: Duration) {
        let mut st = lock(&self.shared.state);
        let after = u64::try_from(after.as_nanos()).expect("a crash within the clock's range");
        st.crash_at = Some(st.now + after);
    }

    /// Change the faults the device injects from now on, from inside a run.
    ///
    /// Panic if `faults` changes the device's sector or atomic unit.
    pub fn set_faults(&self, faults: Faults) {
        lock(&self.shared.state).set_faults(faults);
    }

    /// Returns the number of I/O queues opened since the simulation began.
    pub fn queues(&self) -> usize {
        lock(&self.shared.state).rings.len()
    }

    /// Returns the number of events recorded so far, or 0 when the simulation is not recording.
    pub fn recorded(&self) -> usize {
        lock(&self.shared.state)
            .disk
            .recording()
            .map_or(0, Recording::len)
    }

    /// Returns the contents of every file as a reader would see it now.
    pub fn files(&self) -> BTreeMap<String, Vec<u8>> {
        lock(&self.shared.state).disk.files()
    }

    /// Returns the number of files this process has open.
    pub fn files_open(&self) -> usize {
        lock(&self.shared.state).handles.iter().flatten().count()
    }

    /// Act on the ops of the rest of this run as `plan` states, in place of the plan before. The
    /// rules end with the run; the numbered actions last until taken or disarmed.
    pub fn plan(&self, plan: Plan) {
        lock(&self.shared.plan).plan(plan);
    }

    /// Add the rules and numbered actions of `plan` to the plan, its rules after the plan's own.
    pub fn extend(&self, plan: Plan) {
        lock(&self.shared.plan).extend(plan);
    }

    /// Number the ops `ops` matches from now on, continuing the numbers of earlier ops.
    pub fn arm(&self, ops: Match) {
        let recorded = self.recorded();
        lock(&self.shared.plan).arm(ops, recorded);
    }

    /// Stop numbering ops and drop the plan's numbered actions not taken. Returns how many ops
    /// were numbered.
    pub fn disarm(&self) -> u64 {
        lock(&self.shared.plan).disarm()
    }

    /// Returns the ops numbered so far, in this process and earlier ones.
    pub fn numbered(&self) -> Numbered {
        lock(&self.shared.plan).numbered()
    }

    /// Returns a count of the ops `ops` matches submitted from now on.
    pub fn counted(&self, ops: Match) -> Count {
        lock(&self.shared.plan).counted(ops)
    }

    /// Returns a log of the ops `ops` matches submitted from now on.
    pub fn logged(&self, ops: Match) -> OpLog {
        lock(&self.shared.plan).logged(ops)
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

impl SimRuntime {
    fn start_thread<F>(&self, name: &str, daemon: bool, body: F) -> ThreadId
    where
        F: FnOnce() + Send + 'static,
    {
        let Some(mut st) = self.enter() else {
            return ThreadId(u64::MAX);
        };
        let id = st.threads.len();
        st.threads.push(SimThread {
            name: name.to_owned(),
            daemon,
            status: Status::Runnable,
            token: false,
            turn: Arc::new(Condvar::new()),
        });
        let now = st.now;
        trace!(st.trace, "t={now} spawn {id} {name:?}");
        let shared = self.shared.clone();
        let rt = self.clone();
        let body = move || {
            let _switches = rt.switch_points();
            body();
        };
        #[expect(
            clippy::disallowed_methods,
            reason = "goes if simulated threads stop running on host threads"
        )]
        let handle = thread::Builder::new()
            .name(format!("sim-{id}-{name}"))
            .spawn(move || thread_main(&shared, id, body))
            .expect("the host refused a thread");
        lock(&self.shared.os).push(handle);
        drop(self.reschedule(st));
        ThreadId(id as u64)
    }
}

impl Threads for SimRuntime {
    fn spawn<F>(&self, name: &str, body: F) -> ThreadId
    where
        F: FnOnce() + Send + 'static,
    {
        self.start_thread(name, false, body)
    }

    fn spawn_daemon<F>(&self, name: &str, body: F) -> ThreadId
    where
        F: FnOnce() + Send + 'static,
    {
        self.start_thread(name, true, body)
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
        let turn = Arc::clone(&st.threads[id].turn);
        let st = turn
            .wait_while(st, |st| {
                matches!(st.phase, Phase::Running) && st.current != id
            })
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let Some(st) = self.rt.enter() else {
            return Ticket::NONE;
        };
        let path = st.path_of(&op);
        drop(st);
        let mut decision = lock(&self.rt.shared.plan).decide(&op, path.as_deref());
        let (injected, hold) = match decision.take_action() {
            None => (None, false),
            Some(Action::Run(body)) => {
                body();
                (None, false)
            }
            Some(Action::Fail(error)) => (Some(Injected::Fail(error)), false),
            Some(Action::Lose) => {
                assert!(
                    op.kind() == OpKind::Write,
                    "only a write can be lost, not a {:?}",
                    op.kind()
                );
                (Some(Injected::Lose), false)
            }
            Some(Action::Hold) => (None, true),
        };
        let Some(mut st) = self.rt.enter() else {
            return Ticket::NONE;
        };
        let ticket = st.next_ticket;
        st.next_ticket += 1;
        let now = st.now;
        let due = match op {
            Op::Timer { at } => now.max(at.as_nanos()),
            _ if st.config.delay_completions => {
                let (lo, hi) = st.config.latency;
                now + st.sched.between(nanos(lo), nanos(hi))
            }
            _ => now,
        };
        trace!(
            st.trace,
            "t={now} submit {ticket} ring {} due {due} {op:?}", self.ring
        );
        let upto = st.disk.seq();
        let submitter = st.current;
        let recorded = st.disk.recording().map_or(0, Recording::len);
        let entry = decision.entry(
            &op,
            path,
            self::ticket(ticket),
            Instant::from_nanos(now),
            recorded,
        );
        let logged = decision
            .into_logs()
            .into_iter()
            .map(|log| {
                let at = log.push(entry.clone());
                (log, at)
            })
            .collect();
        let event = Event {
            ring: self.ring,
            ticket,
            submitter,
            op,
            upto,
            injected,
            logged,
        };
        if hold {
            trace!(st.trace, "  held");
            st.held.push(event);
        } else {
            st.events.insert((due, ticket), event);
        }
        drop(self.rt.reschedule(st));
        self::ticket(ticket)
    }

    fn poll(&mut self) -> Option<Completion> {
        let st = self.rt.enter()?;
        let mut st = self.rt.reschedule(st);
        st.rings[self.ring].pop_front()
    }
}
