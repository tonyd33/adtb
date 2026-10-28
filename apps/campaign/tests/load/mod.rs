pub mod actors;
pub mod checks;
pub mod fixture;

use adtb::typed::Bundle;
use adtb::{CallOptions, Callable, Committed};
use adtb_sim::SimRuntime;
use campaign::v1_6;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};

pub type Calls = Bundle<v1_6::Bundle, SimRuntime>;

pub fn call<C: Callable<Bundle = v1_6::Bundle>>(calls: &Calls, c: C, args: C::Args) -> C::Output {
    calls.call(c, args, &CallOptions::default()).unwrap()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Campaign(adtb::Key<v1_6::Campaigns>),
    Entity(adtb::Key<v1_6::Entities>),
    Encounter(adtb::Key<v1_6::Encounters>),
    Session(adtb::Key<v1_6::Sessions>),
}

#[derive(Clone, Debug)]
pub struct Event {
    pub actor: usize,
    pub action: &'static str,
    pub target: Target,
    pub step: usize,
    pub attempts: u32,
    pub position: Option<adtb::Position>,
    pub outcome: String,
}

#[derive(Clone, Default)]
pub struct Ledger(Arc<Mutex<Vec<Event>>>);

impl Ledger {
    pub fn commit<T: Debug>(
        &self,
        actor: usize,
        action: &'static str,
        target: Target,
        step: usize,
        result: Result<Committed<T>, v1_6::Error>,
    ) -> T {
        match result {
            Ok(commit) => {
                self.0.lock().unwrap().push(Event {
                    actor,
                    action,
                    target,
                    step,
                    attempts: commit.attempts,
                    position: Some(commit.position),
                    outcome: format!("value={:?}", commit.value),
                });
                commit.value
            }
            Err(error) => {
                self.refused(actor, action, target.clone(), &error);
                panic!("actor={actor} action={action} target={target:?} step={step}: {error:?}");
            }
        }
    }
    pub fn refused(&self, actor: usize, action: &'static str, target: Target, error: &v1_6::Error) {
        self.0.lock().unwrap().push(Event {
            actor,
            action,
            target,
            step: 0,
            attempts: 0,
            position: None,
            outcome: format!("refused {error:?}"),
        });
    }
    pub fn events(&self) -> Vec<Event> {
        self.0.lock().unwrap().clone()
    }
    pub fn report(&self) -> String {
        self.events()
            .iter()
            .map(|e| {
                format!(
                    "actor={} action={} target={:?} step={} attempts={} position={:?} {}\n",
                    e.actor, e.action, e.target, e.step, e.attempts, e.position, e.outcome
                )
            })
            .collect()
    }
}
