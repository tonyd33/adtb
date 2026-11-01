use super::*;
use serde_json::{Value, json};
use std::sync::{Condvar, Mutex};
use std::time::Instant;

mod artifact;
mod storage;

#[derive(Clone, Copy)]
pub(super) struct MetricClock(Instant);

impl MetricClock {
    #[expect(
        clippy::disallowed_methods,
        reason = "remove when host phase timing has a clock independent of the engine runtime"
    )]
    pub(super) fn new() -> Self {
        Self(Instant::now())
    }

    pub(super) fn stamp(self) -> u64 {
        u64::try_from(self.0.elapsed().as_nanos())
            .expect("the bounded reporting run fits the u64 nanosecond schema")
    }
}

pub(super) struct RunGate {
    pub ready: Barrier,
    released: Mutex<bool>,
    wake: Condvar,
}

impl RunGate {
    pub fn new(writers: usize) -> Self {
        Self {
            ready: Barrier::new(writers + 1),
            released: Mutex::new(false),
            wake: Condvar::new(),
        }
    }

    pub fn release(&self, clock: MetricClock) -> u64 {
        let mut released = self.released.lock().expect("release mutex is not poisoned");
        assert!(!*released, "only the coordinator releases actors once");
        let stamp = clock.stamp();
        *released = true;
        self.wake.notify_all();
        stamp
    }

    pub fn wait(&self) {
        let released = self.released.lock().expect("release mutex is not poisoned");
        drop(
            self.wake
                .wait_while(released, |released| !*released)
                .expect("release mutex is not poisoned"),
        );
    }
}

#[derive(Clone, Copy)]
pub(super) struct Config {
    pub peer: Peer,
    pub n: i64,
    pub writers: usize,
    pub lifecycle: bool,
    pub requests: usize,
}

pub(super) struct Span {
    pub name: &'static str,
    pub actor: Option<counter::ActorId>,
    pub start_ns: u64,
    pub end_ns: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Terminal {
    Completed,
    Failed(&'static str),
    Uncertain,
}

impl Terminal {
    fn outcome(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed(_) => "failed",
            Self::Uncertain => "uncertain",
        }
    }

    fn failure(self) -> Option<&'static str> {
        match self {
            Self::Completed => None,
            Self::Failed(kind) => Some(kind),
            Self::Uncertain => Some("uncertain"),
        }
    }
}

pub(super) struct CallSample {
    pub actor: counter::ActorId,
    pub call: CallId,
    pub action: Action,
    pub offered_ns: u64,
    pub ended_ns: u64,
    pub terminal: Terminal,
    pub attempts: u32,
}

pub(super) struct AttemptSample {
    pub actor: counter::ActorId,
    pub call: CallId,
    pub attempt: u32,
    pub started_ns: u64,
    pub ended_ns: u64,
    pub outcome: &'static str,
}

pub(super) struct ActorTiming {
    pub preparation: Span,
    pub calls: Vec<CallSample>,
    pub attempts: Vec<AttemptSample>,
}

pub(super) struct Execution {
    pub actor: Actor,
    pub timing: ActorTiming,
}

pub(super) struct Report {
    pub clock: MetricClock,
    pub config: Config,
    pub plan: Vec<(counter::ActorId, CallId, Action)>,
    pub actors: Vec<ActorTiming>,
    pub phases: Vec<Span>,
    pub released: Option<u64>,
    pub settings: String,
    pub validation: [bool; 4],
    pub active: Vec<Value>,
    pub closed: Vec<Value>,
    pub incomplete: bool,
}

impl Report {
    pub fn new(config: Config) -> Self {
        Self {
            clock: MetricClock::new(),
            config,
            plan: Vec::new(),
            actors: Vec::new(),
            phases: Vec::new(),
            released: None,
            settings: String::new(),
            validation: [false; 4],
            active: Vec::new(),
            closed: Vec::new(),
            incomplete: false,
        }
    }

    pub fn measure<T>(
        &mut self,
        name: &'static str,
        f: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let start_ns = self.clock.stamp();
        let result = f();
        self.phases.push(Span {
            name,
            actor: None,
            start_ns,
            end_ns: self.clock.stamp(),
        });
        result
    }

    pub fn inventory(&mut self, root: &Path, active: bool) -> Result<(), String> {
        let name = if active {
            "active_inventory"
        } else {
            "closed_inventory"
        };
        let rows = self.measure(name, || storage::inventory(root))?;
        if active {
            self.active = rows;
        } else {
            self.closed = rows;
        }
        Ok(())
    }
}

fn action_name(action: Action) -> &'static str {
    match action {
        Action::Bump(_) => "bump",
        Action::Start(_) => "start",
        Action::ReadLive(_) => "read_live",
        Action::Finish(_) => "finish",
        Action::ReadPlayed(_) => "read_played",
    }
}

fn workflow(action: Action) -> Option<i64> {
    if matches!(action, Action::Bump(_)) {
        None
    } else {
        Some(target(action).0)
    }
}
