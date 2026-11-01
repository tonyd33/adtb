use std::collections::{BTreeMap, BTreeSet};

use super::model::Id;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActorId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NodeId(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Acknowledged(i64),
    Uncertain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observation {
    pub key: Id,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    ActorCount(usize),
    UncertaintyNotTerminal(ActorId),
    UnknownKey(Id),
    Incompatible,
}

#[derive(Clone, Copy, Debug)]
struct Increment {
    actor: ActorId,
    key: Id,
    value: i64,
}

#[derive(Clone, Copy, Debug)]
struct Uncertain {
    actor: ActorId,
    key: Id,
}

pub fn check(actors: &[Vec<Observation>], final_values: &BTreeMap<Id, i64>) -> Result<(), Error> {
    if actors.is_empty() || actors.len() > 4 {
        return Err(Error::ActorCount(actors.len()));
    }
    let (mut known, uncertain) = observations(actors, final_values)?;
    let mut values: BTreeMap<Id, BTreeSet<i64>> = BTreeMap::new();
    for increment in &known {
        if increment.value < 1
            || increment.value > final_values[&increment.key]
            || !values
                .entry(increment.key)
                .or_default()
                .insert(increment.value)
        {
            return Err(Error::Incompatible);
        }
    }
    let mut missing = Vec::new();
    for (&key, &final_value) in final_values {
        let acknowledged = values.get(&key).map_or(0, BTreeSet::len);
        let possible = uncertain.iter().filter(|u| u.key == key).count();
        let maximum = i64::try_from(acknowledged + possible).map_err(|_| Error::Incompatible)?;
        if final_value < 0 || final_value > maximum {
            return Err(Error::Incompatible);
        }
        for value in 1..=final_value {
            if !values.get(&key).is_some_and(|known| known.contains(&value)) {
                missing.push((key, value));
            }
        }
    }
    if assign(
        &missing,
        &uncertain,
        &mut [false; 4],
        &mut known,
        actors.len(),
    ) {
        Ok(())
    } else {
        Err(Error::Incompatible)
    }
}

fn observations(
    actors: &[Vec<Observation>],
    final_values: &BTreeMap<Id, i64>,
) -> Result<(Vec<Increment>, Vec<Uncertain>), Error> {
    let mut known = Vec::new();
    let mut uncertain = Vec::new();
    for (actor, history) in actors.iter().enumerate() {
        for (index, observation) in history.iter().enumerate() {
            if !final_values.contains_key(&observation.key) {
                return Err(Error::UnknownKey(observation.key));
            }
            match observation.outcome {
                Outcome::Acknowledged(value) => known.push(Increment {
                    actor: ActorId(actor),
                    key: observation.key,
                    value,
                }),
                Outcome::Uncertain => {
                    if index + 1 != history.len() {
                        return Err(Error::UncertaintyNotTerminal(ActorId(actor)));
                    }
                    uncertain.push(Uncertain {
                        actor: ActorId(actor),
                        key: observation.key,
                    });
                }
            }
        }
    }
    Ok((known, uncertain))
}

fn assign(
    missing: &[(Id, i64)],
    uncertain: &[Uncertain],
    used: &mut [bool; 4],
    known: &mut Vec<Increment>,
    actors: usize,
) -> bool {
    let Some((&(key, value), rest)) = missing.split_first() else {
        return compatible(known, actors);
    };
    for (index, call) in uncertain.iter().enumerate() {
        if call.key == key && !used[index] {
            used[index] = true;
            known.push(Increment {
                actor: call.actor,
                key,
                value,
            });
            let accepted = assign(rest, uncertain, used, known, actors);
            known
                .pop()
                .expect("the assigned increment remains until its branch returns");
            used[index] = false;
            if accepted {
                return true;
            }
        }
    }
    false
}

fn compatible(increments: &[Increment], actors: usize) -> bool {
    let mut successors = vec![Vec::new(); increments.len()];
    let mut indegree = vec![0; increments.len()];
    let mut last = vec![None; actors];
    let mut keys: BTreeMap<Id, BTreeMap<i64, NodeId>> = BTreeMap::new();
    for (index, increment) in increments.iter().enumerate() {
        let node = NodeId(index);
        if let Some(previous) = last[increment.actor.0] {
            edge(previous, node, &mut successors, &mut indegree);
        }
        last[increment.actor.0] = Some(node);
        keys.entry(increment.key)
            .or_default()
            .insert(increment.value, node);
    }
    for values in keys.values() {
        let mut previous = None;
        for &node in values.values() {
            if let Some(before) = previous {
                edge(before, node, &mut successors, &mut indegree);
            }
            previous = Some(node);
        }
    }
    let mut ready: Vec<_> = indegree
        .iter()
        .enumerate()
        .filter_map(|(index, &degree)| (degree == 0).then_some(NodeId(index)))
        .collect();
    let mut visited = 0;
    while let Some(node) = ready.pop() {
        visited += 1;
        for next in &successors[node.0] {
            indegree[next.0] -= 1;
            if indegree[next.0] == 0 {
                ready.push(*next);
            }
        }
    }
    visited == increments.len()
}

fn edge(from: NodeId, to: NodeId, successors: &mut [Vec<NodeId>], indegree: &mut [usize]) {
    successors[from.0].push(to);
    indegree[to.0] += 1;
}

#[cfg(test)]
#[path = "counter_tests.rs"]
mod tests;
