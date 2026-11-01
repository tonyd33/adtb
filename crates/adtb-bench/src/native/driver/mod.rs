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

mod execute;
#[path = "../reporting/mod.rs"]
mod reporting;
mod run;
pub use run::correctness;

#[cfg(test)]
#[path = "../driver_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../control_tests.rs"]
mod control_tests;

#[cfg(test)]
#[path = "../crash_tests/mod.rs"]
mod crash_tests;
