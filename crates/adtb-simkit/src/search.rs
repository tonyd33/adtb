//! Search a workload's failure paths: fail each op it submits, take each image a crash at each of
//! its device events can leave, or act once at each op.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use adtb::runtime::{
    Clock, Completion, Done, Entropy, FileId, Instant, Io, IoError, Op, Runtime, ThreadId, Threads,
    Ticket,
};
use adtb_sim::{Config, Outcome, Sim, SimIo, SimRuntime};
use adtb_testkit::profile::{assert_reached, count, every_case};

use crate::{api, common, crash};

pub type Files = BTreeMap<String, Vec<u8>>;

/// How many times the images a crash search draws around a rename, where the order of names counts.
const RENAME_IMAGES: usize = 4;

type Before = Arc<dyn Fn(&Op, Option<&str>) -> Option<IoError> + Send + Sync>;
type During = Box<dyn FnOnce() + Send>;
type Eligible = Arc<dyn Fn(&Op, Option<&str>) -> bool + Send + Sync>;

/// What happens at the planned op.
pub enum Action {
    /// Fail the op: a write or an allocation with `Full`, any other with `Failed`.
    Fail,
    /// Run the closure, then submit the op.
    Run(During),
    /// Complete a write as written without submitting it.
    Lose,
}

impl std::fmt::Debug for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Action::Fail => "Fail",
            Action::Run(_) => "Run",
            Action::Lose => "Lose",
        })
    }
}

#[derive(Default)]
struct Plan {
    eligible: Option<Eligible>,
    seen: u64,
    names: Vec<&'static str>,
    at: Option<u64>,
    action: Option<Action>,
    also: BTreeMap<u64, Action>,
    struck: Option<String>,
    write_or_sync: Option<WriteOrSync>,
    first_event: usize,
}

/// A runtime over the simulator's that fails each op `before` answers an error for, counts the files
/// open, runs `during` once while the first force after it is armed is in flight and never completes
/// a later force of that file, and numbers the eligible ops of a workload to act at one.
#[derive(Clone)]
pub struct Hooked {
    pub rt: SimRuntime,
    before: Arc<Mutex<Before>>,
    during: Arc<Mutex<Option<During>>>,
    held: Arc<Mutex<Option<FileId>>>,
    open: Arc<AtomicI64>,
    paths: Arc<Mutex<HashMap<FileId, String>>>,
    plan: Arc<Mutex<Plan>>,
}

impl std::fmt::Debug for Hooked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hooked").finish_non_exhaustive()
    }
}

impl Hooked {
    pub fn new(rt: &SimRuntime) -> Hooked {
        let pass: Before = Arc::new(|_, _| None);
        Hooked {
            rt: rt.clone(),
            before: Arc::new(Mutex::new(pass)),
            during: Arc::default(),
            held: Arc::default(),
            open: Arc::default(),
            paths: Arc::default(),
            plan: Arc::default(),
        }
    }

    /// Returns a hook on `rt`, a later process of the same simulation, that shares this hook's
    /// failures, plan and numbering of eligible ops.
    pub fn rebound(&self, rt: &SimRuntime) -> Hooked {
        Hooked {
            rt: rt.clone(),
            before: self.before.clone(),
            during: Arc::default(),
            held: Arc::default(),
            open: Arc::default(),
            paths: Arc::default(),
            plan: self.plan.clone(),
        }
    }

    pub fn fail_when(&self, before: impl Fn(&Op) -> Option<IoError> + Send + Sync + 'static) {
        *self.before.lock().unwrap() = Arc::new(move |op, _| before(op));
    }

    /// Fail each op `before` answers an error for, given the op and the path it names.
    pub fn fail_named(
        &self,
        before: impl Fn(&Op, Option<&str>) -> Option<IoError> + Send + Sync + 'static,
    ) {
        *self.before.lock().unwrap() = Arc::new(before);
    }

    /// Run `during` once while the next force is in flight, and hold every later force of its file.
    pub fn during_next_force(&self, during: impl FnOnce() + Send + 'static) {
        *self.during.lock().unwrap() = Some(Box::new(during));
    }

    pub fn files_open(&self) -> i64 {
        self.open.load(Ordering::SeqCst)
    }

    /// Take `action` at eligible op number `at`, once [`arm`](Self::arm) starts the count.
    pub fn plan(&self, at: u64, action: Action) {
        let mut plan = self.plan.lock().unwrap();
        plan.at = Some(at);
        plan.action = Some(action);
    }

    /// Take `action` at eligible op `at` as well as at the op [`Hooked::plan`] names, leaving the
    /// op the search reports struck unchanged.
    pub fn plan_also(&self, at: u64, action: Action) {
        self.plan.lock().unwrap().also.insert(at, action);
    }

    /// Start numbering the ops `eligible` accepts, given each with the path it names, and the device
    /// events a crash search covers.
    pub fn arm(&self, eligible: impl Fn(&Op, Option<&str>) -> bool + Send + Sync + 'static) {
        let mut plan = self.plan.lock().unwrap();
        plan.eligible = Some(Arc::new(eligible));
        plan.first_event = self.rt.recorded();
    }

    /// Stop numbering ops and return how many were eligible.
    pub fn disarm(&self) -> u64 {
        let mut plan = self.plan.lock().unwrap();
        plan.eligible = None;
        plan.action = None;
        plan.also.clear();
        plan.seen
    }

    /// Returns the op the plan acted at, as text.
    pub fn struck(&self) -> Option<String> {
        self.plan.lock().unwrap().struck.clone()
    }

    fn path_of(&self, op: &Op) -> Option<String> {
        let by_file = |file: &FileId| self.paths.lock().unwrap().get(file).cloned();
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
            | Op::Close { file } => by_file(file),
            Op::Timer { .. } => None,
        }
    }

    /// Returns the planned action when `op` is the op it is planned at.
    fn strike(&self, op: &Op) -> Option<Action> {
        let path = self.path_of(op);
        let mut plan = self.plan.lock().unwrap();
        let eligible = plan.eligible.clone()?;
        if !eligible(op, path.as_deref()) {
            return None;
        }
        let number = plan.seen;
        plan.seen += 1;
        plan.names.push(op_name(op));
        if plan.at != Some(number) {
            return plan.also.remove(&number);
        }
        plan.struck = Some(format!("{} of {path:?}", op_name(op)));
        plan.write_or_sync = match op {
            Op::Write { .. } => Some(WriteOrSync::Write),
            Op::Sync { .. } => Some(WriteOrSync::Sync),
            _ => None,
        };
        plan.action.take()
    }
}

fn op_name(op: &Op) -> &'static str {
    match op {
        Op::Open { .. } => "Open",
        Op::CreateNew { .. } => "CreateNew",
        Op::Read { .. } => "Read",
        Op::Write { .. } => "Write",
        Op::Size { .. } => "Size",
        Op::Allocate { .. } => "Allocate",
        Op::Sync { .. } => "Sync",
        Op::SyncDir { .. } => "SyncDir",
        Op::List { .. } => "List",
        Op::Remove { .. } => "Remove",
        Op::RemoveDir { .. } => "RemoveDir",
        Op::Rename { .. } => "Rename",
        Op::CreateDir { .. } => "CreateDir",
        Op::Close { .. } => "Close",
        Op::Timer { .. } => "Timer",
    }
}

#[derive(Debug)]
pub struct HookedIo {
    io: SimIo,
    hook: Hooked,
    failed: VecDeque<Completion>,
    tickets: u64,
    opening: HashMap<Ticket, String>,
}

impl Clock for Hooked {
    fn now(&self) -> Instant {
        self.rt.now()
    }

    fn wall(&self) -> std::time::SystemTime {
        self.rt.wall()
    }
}

impl Threads for Hooked {
    fn spawn<F>(&self, name: &str, body: F) -> ThreadId
    where
        F: FnOnce() + Send + 'static,
    {
        self.rt.spawn(name, body)
    }

    fn spawn_daemon<F>(&self, name: &str, body: F) -> ThreadId
    where
        F: FnOnce() + Send + 'static,
    {
        self.rt.spawn_daemon(name, body)
    }

    fn current(&self) -> ThreadId {
        self.rt.current()
    }

    fn park(&self) {
        self.rt.park();
    }

    fn unpark(&self, thread: ThreadId) {
        self.rt.unpark(thread);
    }

    fn yield_now(&self) {
        self.rt.yield_now();
    }

    fn join(&self, thread: ThreadId) {
        self.rt.join(thread);
    }
}

impl Entropy for Hooked {
    fn next_u64(&self) -> u64 {
        self.rt.next_u64()
    }
}

impl Runtime for Hooked {
    type Io = HookedIo;

    fn io(&self) -> HookedIo {
        HookedIo {
            io: self.rt.io(),
            hook: self.clone(),
            failed: VecDeque::new(),
            tickets: u64::MAX / 2,
            opening: HashMap::new(),
        }
    }
}

impl HookedIo {
    fn fail(&mut self, op: Op, error: IoError) -> Ticket {
        self.answer(op, Err(error))
    }

    fn answer(&mut self, op: Op, result: Result<Done, IoError>) -> Ticket {
        self.tickets += 1;
        let ticket = Ticket(self.tickets);
        let buf = match op {
            Op::Read { buf, .. } | Op::Write { buf, .. } => Some(buf),
            _ => None,
        };
        self.failed.push_back(Completion {
            ticket,
            buf,
            result,
        });
        ticket
    }
}

impl Io for HookedIo {
    fn submit(&mut self, op: Op) -> Ticket {
        let before = self.hook.before.lock().unwrap().clone();
        if let Some(error) = before(&op, self.hook.path_of(&op).as_deref()) {
            return self.fail(op, error);
        }
        match self.hook.strike(&op) {
            Some(Action::Fail) => {
                let error = match op {
                    Op::Write { .. } | Op::Allocate { .. } => IoError::Full,
                    _ => IoError::Failed,
                };
                return self.fail(op, error);
            }
            Some(Action::Run(body)) => body(),
            Some(Action::Lose) if matches!(op, Op::Write { .. }) => {
                return self.answer(op, Ok(Done::Written));
            }
            Some(Action::Lose) => panic!("only a write can be lost, not a {}", op_name(&op)),
            None => {}
        }
        if let Op::Close { file } = op {
            self.hook.open.fetch_sub(1, Ordering::SeqCst);
            self.hook.paths.lock().unwrap().remove(&file);
        }
        let opening = match &op {
            Op::Open { path, .. } | Op::CreateNew { path } => Some(path.clone()),
            _ => None,
        };
        let Op::Sync { file } = op else {
            let ticket = self.io.submit(op);
            if let Some(path) = opening {
                self.opening.insert(ticket, path);
            }
            return ticket;
        };
        if *self.hook.held.lock().unwrap() == Some(file) {
            loop {
                self.hook.rt.park();
            }
        }
        let during = self.hook.during.lock().unwrap().take();
        if during.is_some() {
            *self.hook.held.lock().unwrap() = Some(file);
        }
        let ticket = self.io.submit(op);
        if let Some(during) = during {
            during();
            let mut io = self.hook.rt.io();
            common::sleep(&self.hook.rt, &mut io, std::time::Duration::from_millis(50));
        }
        ticket
    }

    fn poll(&mut self) -> Option<Completion> {
        if let Some(failed) = self.failed.pop_front() {
            return Some(failed);
        }
        let done = self.io.poll()?;
        if let Ok(Done::Opened { file, .. }) = done.result {
            self.hook.open.fetch_add(1, Ordering::SeqCst);
            if let Some(path) = self.opening.remove(&done.ticket) {
                self.hook.paths.lock().unwrap().insert(file, path);
            }
        }
        Some(done)
    }
}

/// Returns up to `most` of `items`, evenly spread and holding both ends.
fn spread(items: &[u64], most: u64) -> Vec<u64> {
    let len = items.len() as u64;
    if len <= most || most < 2 {
        return items
            .iter()
            .copied()
            .take(most.max(len.min(1)) as usize)
            .collect();
    }
    (0..most)
        .map(|i| items[(i * (len - 1) / (most - 1)) as usize])
        .collect()
}

fn is_bulk(name: &str) -> bool {
    matches!(name, "Read" | "Write" | "Size")
}

fn numbered(names: &[&'static str], pick: impl Fn(&str) -> bool) -> Vec<u64> {
    (0..names.len() as u64)
        .filter(|&i| pick(names[i as usize]))
        .collect()
}

/// Returns each rename and the ops beside it, up to `most` of the ops that change no bytes and two
/// of those that do or only read.
fn around_renames(names: &[&'static str], most: u64) -> BTreeSet<u64> {
    let last = names.len() as u64;
    let renames = numbered(names, |name| name == "Rename");
    let mut out: BTreeSet<u64> = renames
        .into_iter()
        .flat_map(|at| [at.saturating_sub(1), at, at + 1])
        .filter(|&at| at < last)
        .collect();
    out.extend(spread(&numbered(names, |n| !is_bulk(n)), count(most)));
    out.extend(spread(&numbered(names, is_bulk), count(2)));
    out
}

/// Returns the op numbers to fail: all of them under the profile `deep`, else
/// [`around_renames`]'s.
fn chosen_ops(names: &[&'static str], most: u64) -> Vec<u64> {
    if every_case() {
        return numbered(names, |_| true);
    }
    around_renames(names, most).into_iter().collect()
}

/// Returns the op numbers to start a second run at: [`around_renames`]'s, and under the profile
/// `deep` each op that changes no bytes.
fn chosen_starts(names: &[&'static str], most: u64) -> Vec<u64> {
    let mut out = around_renames(names, most);
    if every_case() {
        out.extend(numbered(names, |n| !is_bulk(n)));
    }
    out.into_iter().collect()
}

/// The write or file force the failure search struck.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteOrSync {
    Write,
    Sync,
}

/// One workload run with a failure at one op.
#[derive(Debug)]
pub struct Failed<T> {
    /// The number of the failed op.
    pub at: u64,
    /// The failed op, as text.
    pub op: String,
    /// The struck operation when it was a write or file force.
    pub write_or_sync: Option<WriteOrSync>,
    /// What the workload returned.
    pub out: T,
    /// The device after the workload, as a reader sees it.
    pub files: Files,
    pub dirs: BTreeSet<String>,
}

/// Returns the op `workload` numbers on a device holding `files`, run with no fault, by name.
fn eligible_ops<T>(
    files: &Files,
    seed: u64,
    workload: &impl Fn(&Hooked) -> T,
) -> Vec<&'static str> {
    let mut sim = Sim::with_files(seed, Config::default(), files.clone());
    api::run(&mut sim, |rt| {
        let hook = Hooked::new(&rt);
        workload(&hook);
        std::mem::take(&mut hook.plan.lock().unwrap().names)
    })
}

/// Run `workload` on a device holding `files` failing each chosen op, and give `check` each result.
///
/// Preconditions:
/// - `workload` calls [`Hooked::arm`] where the search starts and [`Hooked::disarm`] where it ends,
///   and is deterministic for a seed up to the failure.
pub fn each_failure<T>(
    files: &Files,
    seed: u64,
    most: u64,
    workload: impl Fn(&Hooked) -> T,
    mut check: impl FnMut(Failed<T>),
) {
    failure_runs(files, seed, most, &[], workload, false, |failed| {
        check(Failed {
            at: failed.at,
            op: failed.op,
            write_or_sync: failed.write_or_sync,
            out: failed.out.expect("the workload finishes"),
            files: failed.files,
            dirs: failed.dirs,
        });
    });
}

/// Search failures as [`each_failure`] does, accepting typed fatal I/O after the simulator's
/// crash lifecycle. Return `None` instead of a workload result when the process crashes.
/// Sample positions in the default profile, always with `also`, and search every eligible position
/// under `deep`.
pub fn each_fatal_failure<T>(
    files: &Files,
    seed: u64,
    most: u64,
    also: &[u64],
    workload: impl Fn(&Hooked) -> T,
    check: impl FnMut(Failed<Option<T>>),
) {
    failure_runs(files, seed, most, also, workload, true, check);
}

fn failure_runs<T>(
    files: &Files,
    seed: u64,
    most: u64,
    also: &[u64],
    workload: impl Fn(&Hooked) -> T,
    fatal: bool,
    mut check: impl FnMut(Failed<Option<T>>),
) {
    let names = eligible_ops(files, seed, &workload);
    assert!(!names.is_empty(), "the workload arms the search");
    let total = names.len();
    let mut positions: BTreeSet<_> = chosen_ops(&names, most).into_iter().collect();
    if fatal {
        positions.extend([0, (total - 1) as u64]);
        positions.extend(also);
    }
    for at in positions {
        let mut sim = Sim::with_files(seed, Config::default(), files.clone());
        let mut observed = None;
        let mut out = None;
        let body = |rt| {
            let hook = Hooked::new(&rt);
            hook.plan(at, Action::Fail);
            observed = Some(hook.clone());
            out = Some(workload(&hook));
        };
        let outcome = if fatal {
            crash::run_process(&mut sim, body)
        } else {
            sim.run(body)
        };
        match outcome {
            Outcome::Finished => assert!(out.is_some(), "the workload returns"),
            Outcome::Crashed => {
                assert!(fatal, "the workload finishes");
                out = None;
            }
        }
        let hook = observed.expect("the workload starts");
        let op = hook
            .struck()
            .unwrap_or_else(|| panic!("op {at} of {total} came up in the workload"));
        let write_or_sync = hook.plan.lock().unwrap().write_or_sync;
        check(Failed {
            at,
            op,
            write_or_sync,
            out,
            files: sim.files(),
            dirs: sim.dirs(),
        });
    }
}

/// Run `workload` once and give `check` each image a crash at each chosen device event after it arms
/// the search can leave, up to `images` for each event, with the event's number. Fail unless the
/// images include one `published` accepts and one it does not.
pub fn each_crash<T>(
    files: &Files,
    seed: u64,
    most: u64,
    images: usize,
    workload: impl FnOnce(&Hooked) -> T,
    published: impl Fn(&Files) -> bool,
    mut check: impl FnMut(usize, Files),
) {
    let (mut before, mut after) = (0, 0);
    let run = |hook: &Hooked| {
        workload(hook);
        Vec::new()
    };
    each_crash_image(files, seed, most, images, run, |at, image| {
        if published(&image) {
            after += 1;
        } else {
            before += 1;
        }
        check(at, image);
    });
    assert_reached("an image before the result is published", before, 1);
    assert_reached("an image with the result published", after, 1);
}

/// Run `workload` once and give `check` each image a crash at each chosen device event after it arms
/// the search can leave, up to `images` for each event, with the event's number. The chosen events
/// include each armed one `workload` returns.
pub fn each_crash_image(
    files: &Files,
    seed: u64,
    most: u64,
    images: usize,
    workload: impl FnOnce(&Hooked) -> Vec<usize>,
    mut check: impl FnMut(usize, Files),
) {
    let mut sim = Sim::with_files(seed, Config::default(), files.clone());
    sim.record();
    let (first, also) = api::run(&mut sim, |rt| {
        let hook = Hooked::new(&rt);
        let also = workload(&hook);
        (hook.plan.lock().unwrap().first_event, also)
    });
    let recording = sim.recording();
    let every: Vec<u64> = (first as u64..=recording.len() as u64).collect();
    let renames = recording.renames().into_iter().map(|at| at as u64);
    let around: BTreeSet<u64> = renames
        .flat_map(|at| [at - 1, at, at + 1])
        .filter(|at| every.contains(at))
        .collect();
    let points: BTreeSet<u64> = if every_case() {
        every.iter().copied().collect()
    } else {
        around
            .iter()
            .copied()
            .chain(spread(&every, count(most)))
            .chain(also.iter().map(|&at| at as u64))
            .collect()
    };
    for at in &also {
        assert!(every.contains(&(*at as u64)), "event {at} is armed");
    }
    for at in points {
        let drawn = if around.contains(&at) {
            images * RENAME_IMAGES
        } else {
            images
        };
        let at = at as usize;
        for image in recording.images(at, drawn, at as u64) {
            check(at, image);
        }
    }
}

/// Run `workload` once with no op to act at, to count, then once per chosen op with its number. The
/// workload plans its own action with [`Hooked::plan`].
pub fn each_start(
    files: &Files,
    seed: u64,
    most: u64,
    workload: impl Fn(&Hooked, Option<u64>),
    mut after: impl FnMut(u64, Files),
) {
    let names = eligible_ops(files, seed, &|hook: &Hooked| workload(hook, None));
    assert!(!names.is_empty(), "the workload arms the search");
    let total = names.len();
    for at in chosen_starts(&names, most) {
        let mut sim = Sim::with_files(seed, Config::default(), files.clone());
        let struck = api::run(&mut sim, |rt| {
            let hook = Hooked::new(&rt);
            workload(&hook, Some(at));
            hook.struck()
        });
        assert!(
            struck.is_some(),
            "op {at} of {total} came up in the workload"
        );
        after(at, sim.files());
    }
}
