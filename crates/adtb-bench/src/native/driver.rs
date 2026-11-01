use super::counter;
use super::engine::Adtb;
use super::model::{self, Action, Failure, Id, Output, Prefix, Snapshot};
use super::sqlite::Sqlite;
use adtb_sys::os::{Backend, OsRuntime};
use adtb_sys::runtime::Clock;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Peer {
    Adtb,
    Sqlite,
}

pub enum Adapter {
    Adtb(Box<Adtb>),
    Sqlite(Sqlite),
}

impl Adapter {
    fn open(peer: Peer, root: &Path, writers: usize, create: bool) -> Result<Self, Failure> {
        match peer {
            Peer::Adtb => Ok(Self::Adtb(Box::new(Adtb::open(root, writers, create)?))),
            Peer::Sqlite => Ok(Self::Sqlite(Sqlite::open(
                &root.join("baseline.sqlite"),
                writers,
                create,
            )?)),
        }
    }
    fn seed(&mut self, state: &Snapshot) -> Result<(), Failure> {
        match self {
            Self::Adtb(a) => a.seed(state),
            Self::Sqlite(a) => a.seed(state),
        }
    }
    fn snapshot(&self) -> Result<Snapshot, Failure> {
        match self {
            Self::Adtb(a) => a.snapshot(),
            Self::Sqlite(a) => a.snapshot(),
        }
    }
    fn settings(&self) -> Result<String, Failure> {
        match self {
            Self::Adtb(a) => a.settings(),
            Self::Sqlite(a) => a.settings(),
        }
    }
    fn prepare(&self) -> Result<Prepared<'_>, Failure> {
        match self {
            Self::Adtb(a) => Ok(Prepared::Adtb(a)),
            Self::Sqlite(a) => Ok(Prepared::Sqlite(Box::new(a.prepare()?))),
        }
    }
    fn close(self) -> Result<(), Failure> {
        match self {
            Self::Adtb(a) => a.close(),
            Self::Sqlite(a) => a.close(),
        }
    }
    fn actor(&self, root: &Path, writers: usize) -> Result<Self, Failure> {
        match self {
            Self::Adtb(a) => Ok(Self::Adtb(Box::new(a.actor()?))),
            Self::Sqlite(_) => Self::open(Peer::Sqlite, root, writers, false),
        }
    }
}

enum Prepared<'a> {
    Adtb(&'a Adtb),
    Sqlite(Box<super::sqlite::Prepared<'a>>),
}

impl Prepared<'_> {
    fn call(
        &mut self,
        action: Action,
        remaining: Duration,
    ) -> Result<(Output, Option<adtb::Position>), Failure> {
        match self {
            Self::Adtb(a) => a.call(action, remaining),
            Self::Sqlite(a) => a.call(action, remaining).map(|out| (out, None)),
        }
    }
}

#[derive(Clone, Debug)]
struct CallId(usize);

#[derive(Clone, Debug)]
struct Event {
    call: CallId,
    action: Action,
    attempt: u32,
    result: Result<(Output, Option<adtb::Position>), Failure>,
}

#[derive(Debug)]
struct Actor {
    events: Vec<Event>,
    error: Option<String>,
}

fn actions(n: i64, writers: usize, lifecycle: bool, requests: usize) -> Vec<Vec<Action>> {
    let mut queues = vec![Vec::new(); writers];
    if lifecycle {
        for id in model::permutation(n) {
            let actor = if writers == 1 {
                0
            } else {
                usize::try_from(model::campaign(id) - 1).expect("campaign fits actor index")
            };
            queues[actor].extend([
                Action::Start(id),
                Action::ReadLive(id),
                Action::Finish(id),
                Action::ReadPlayed(id),
            ]);
        }
    } else {
        let order = model::permutation(n);
        for i in 0..requests {
            queues[(i + i / order.len()) % writers].push(Action::Bump(order[i % order.len()]));
        }
    }
    queues
}

fn execute(
    adapter: Adapter,
    queue: Vec<Action>,
    gate: Arc<Barrier>,
    cancel: Arc<AtomicBool>,
    clock: OsRuntime,
) -> Actor {
    let mut actor = Actor {
        events: Vec::new(),
        error: None,
    };
    let prepared = adapter.prepare();
    gate.wait();
    let mut prepared = match prepared {
        Ok(p) => p,
        Err(e) => {
            actor.error = Some(format!("prepare: {e:?}"));
            cancel.store(true, Ordering::Release);
            return actor;
        }
    };
    let actor_deadline = clock.now() + Duration::from_secs(120);
    for (call, action) in queue.into_iter().enumerate() {
        if cancel.load(Ordering::Acquire) {
            break;
        }
        for attempt in 1..=100 {
            if clock.now() >= actor_deadline {
                actor.error = Some("cooperative actor queue budget exhausted".into());
                cancel.store(true, Ordering::Release);
                break;
            }
            let result = prepared.call(action, actor_deadline.saturating_since(clock.now()));
            let retry = matches!(result, Err(Failure::Retry(_)));
            let error = match &result {
                Ok((out, _))
                    if !matches!(action, Action::Bump(_))
                        && *out != model::expected_output(action) =>
                {
                    Some(format!("output {action:?}: {out:?}"))
                }
                Err(e) if !retry => Some(format!("call {action:?}: {e:?}")),
                _ => None,
            };
            actor.events.push(Event {
                call: CallId(call),
                action,
                attempt,
                result,
            });
            if let Some(error) = error {
                actor.error = Some(error);
                cancel.store(true, Ordering::Release);
                break;
            }
            if !retry {
                break;
            }
            if attempt == 100 {
                actor.error = Some("100 rollback-confirmed attempts exhausted".into());
                cancel.store(true, Ordering::Release);
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        if actor.error.is_some() {
            break;
        }
    }
    actor
}

fn audit(
    fixture: &Snapshot,
    actual: &Snapshot,
    actors: &[Actor],
    lifecycle: bool,
) -> Result<usize, String> {
    let mut expected = fixture.clone();
    let mut prefixes = BTreeMap::new();
    let mut uncertain = BTreeMap::new();
    let mut completed = 0;
    for event in actors.iter().flat_map(|a| &a.events) {
        match &event.result {
            Ok((output, _)) => match event.action {
                Action::Bump(id) => {
                    if !matches!(output, Output::Number(_)) {
                        return Err("bump did not return number".into());
                    }
                    if !expected.counters.iter().any(|c| c.id == id.0) {
                        return Err("unknown counter".into());
                    }
                    completed += 1;
                }
                action => {
                    let id = match action {
                        Action::Start(id)
                        | Action::ReadLive(id)
                        | Action::Finish(id)
                        | Action::ReadPlayed(id) => id,
                        Action::Bump(_) => return Err("unexpected bump".into()),
                    };
                    let prefix = prefixes.entry(id).or_insert(Prefix::Planned);
                    *prefix = prefix.advance(action)?;
                    if matches!(action, Action::ReadPlayed(_)) {
                        completed += 1;
                    }
                }
            },
            Err(Failure::Ambiguous(_)) => {
                uncertain
                    .entry(target(event.action))
                    .or_insert_with(Vec::new)
                    .push(event.action);
            }
            Err(_) => {}
        }
    }
    if expected.campaigns != actual.campaigns || expected.entities != actual.entities {
        return Err("support relation changed".into());
    }
    if lifecycle {
        expected.sessions = expected
            .sessions
            .iter()
            .map(|s| {
                let id = Id(model::session_id(s));
                model::session(id, *prefixes.get(&id).unwrap_or(&Prefix::Planned))
            })
            .collect();
        if actual.counters != fixture.counters || actual.sessions.len() != expected.sessions.len() {
            return Err("relation cardinality changed".into());
        }
        for (want, got) in expected.sessions.iter().zip(&actual.sessions) {
            if want == got {
                continue;
            }
            let id = Id(model::session_id(want));
            let permitted = match uncertain.get(&id).map(Vec::as_slice) {
                Some([Action::Start(_)]) => Some(model::session(id, Prefix::Started)),
                Some([Action::Finish(_)]) => Some(model::session(id, Prefix::Finished)),
                _ => None,
            };
            if permitted.as_ref() != Some(got) {
                return Err(format!("session {id:?}: expected {want:?}, got {got:?}"));
            }
        }
    } else {
        if actual.sessions != fixture.sessions || actual.counters.len() != expected.counters.len() {
            return Err("relation cardinality changed".into());
        }
        if expected
            .counters
            .iter()
            .map(|c| c.id)
            .ne(actual.counters.iter().map(|c| c.id))
        {
            return Err("counter identity changed".into());
        }
        let histories = counter_histories(actors)?;
        let final_values = actual
            .counters
            .iter()
            .map(|c| (Id(c.id), c.value))
            .collect();
        counter::check(&histories, &final_values).map_err(|e| format!("counter history: {e:?}"))?;
    }
    Ok(completed)
}

fn counter_histories(actors: &[Actor]) -> Result<Vec<Vec<counter::Observation>>, String> {
    actors
        .iter()
        .map(|actor| {
            let mut observations = Vec::new();
            for event in &actor.events {
                let Action::Bump(key) = event.action else {
                    return Err("counter ledger contains a lifecycle call".into());
                };
                let outcome = match &event.result {
                    Ok((Output::Number(value), _)) => counter::Outcome::Acknowledged(*value),
                    Ok(_) => return Err("bump did not return number".into()),
                    Err(Failure::Ambiguous(_)) => counter::Outcome::Uncertain,
                    Err(_) => continue,
                };
                observations.push(counter::Observation { key, outcome });
            }
            Ok(observations)
        })
        .collect()
}

fn target(action: Action) -> Id {
    match action {
        Action::Bump(id)
        | Action::Start(id)
        | Action::ReadLive(id)
        | Action::Finish(id)
        | Action::ReadPlayed(id) => id,
    }
}

pub fn correctness(
    peer: Peer,
    root: PathBuf,
    n: i64,
    writers: usize,
    lifecycle: bool,
    requests: usize,
) -> Result<String, String> {
    if ![1, 4].contains(&writers) || !(n == 1000 || n == 10000 || n == 4) {
        return Err("N must be 4 (smoke),1000,10000; writers 1 or 4".into());
    }
    std::fs::create_dir(&root).map_err(|e| e.to_string())?;
    let fixture = model::fixture(n, lifecycle);
    let failure = |e| format!("{e:?}");
    let mut adapter = Adapter::open(peer, &root, writers, true).map_err(failure)?;
    let settings = adapter.settings().map_err(failure)?;
    adapter.seed(&fixture).map_err(failure)?;
    if adapter.snapshot().map_err(failure)? != fixture {
        return Err("loaded fixture differs from independent oracle".into());
    }
    let clock = match &adapter {
        Adapter::Adtb(a) => a.clock.clone(),
        Adapter::Sqlite(_) => {
            OsRuntime::try_new(&root, Backend::Pool { threads: 1 }).map_err(|e| e.to_string())?
        }
    };
    let queues = actions(n, writers, lifecycle, requests);
    let expected_queues = queues.clone();
    let adapters: Vec<_> = (0..writers)
        .map(|_| adapter.actor(&root, writers))
        .collect::<Result<_, _>>()
        .map_err(failure)?;
    let coordinator = if peer == Peer::Sqlite {
        adapter.close().map_err(failure)?;
        None
    } else {
        Some(adapter)
    };
    let gate = Arc::new(Barrier::new(writers));
    let cancel = Arc::new(AtomicBool::new(false));
    let actors = std::thread::scope(|scope| {
        let joins: Vec<_> = adapters
            .into_iter()
            .zip(queues)
            .map(|(a, q)| {
                let gate = gate.clone();
                let cancel = cancel.clone();
                let clock = clock.clone();
                scope.spawn(move || execute(a, q, gate, cancel, clock))
            })
            .collect();
        joins
            .into_iter()
            .map(|j| j.join().map_err(|_| "actor panic".to_owned()))
            .collect::<Result<Vec<_>, _>>()
    })?;
    for (actor, queue) in actors.iter().zip(&expected_queues) {
        for event in &actor.events {
            if queue.get(event.call.0) != Some(&event.action) {
                return Err("ledger call identity differs from frozen request list".into());
            }
        }
    }
    let adapter = match coordinator {
        Some(adapter) => adapter,
        None => Adapter::open(peer, &root, writers, false).map_err(failure)?,
    };
    let mut ledger = std::fs::File::create(root.join("ledger.txt")).map_err(|e| e.to_string())?;
    writeln!(ledger, "{settings}").map_err(|e| e.to_string())?;
    for (actor, a) in actors.iter().enumerate() {
        for e in &a.events {
            writeln!(
                ledger,
                "actor={actor} call={} action={:?} attempt={} result={:?}",
                e.call.0, e.action, e.attempt, e.result
            )
            .map_err(|e| e.to_string())?;
        }
    }
    ledger.sync_all().map_err(|e| e.to_string())?;
    audit(
        &fixture,
        &adapter.snapshot().map_err(failure)?,
        &actors,
        lifecycle,
    )?;
    adapter.close().map_err(failure)?;
    let reopened = Adapter::open(peer, &root, writers, false).map_err(failure)?;
    let completed = audit(
        &fixture,
        &reopened.snapshot().map_err(failure)?,
        &actors,
        lifecycle,
    )?;
    let final_state = reopened.snapshot().map_err(failure)?;
    reopened.close().map_err(failure)?;
    if actors.iter().any(|a| a.error.is_some()) {
        return Err(format!(
            "incomplete/invalid; retained acknowledged-prefix oracle passed; actor errors: {:?}",
            actors.iter().map(|a| &a.error).collect::<Vec<_>>()
        ));
    }
    let required = if lifecycle {
        usize::try_from(n).map_err(|e| e.to_string())?
    } else {
        requests
    };
    if completed != required {
        return Err(format!("incomplete: {completed}/{required} jobs"));
    }
    let count_per_key = i64::try_from(requests).map_err(|e| e.to_string())? / n;
    if !lifecycle
        && final_state
            .counters
            .iter()
            .any(|c| c.value != count_per_key)
    {
        return Err(
            "complete control differs from closed-form requests/N increments per key".into(),
        );
    }
    Ok(format!(
        "correctness complete: {peer:?} N={n} writers={writers} jobs={completed}; full-state and reopen passed; counters=unavailable; measurements=not-run; {settings}"
    ))
}

#[cfg(test)]
#[path = "driver_tests.rs"]
mod tests;
