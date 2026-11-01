use super::super::history_fixture::{finals, history};
use super::*;
use std::path::Path;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(name),
    )
    .expect("read runtime history fixture")
}

#[test]
fn exact_histories_preserve_actor_and_increment_order() {
    let mut cases = 0;
    for line in fixture("history-cases.txt").lines() {
        let fields: Vec<_> = line.split('|').collect();
        let [name, operations, final_values, expected] = fields.as_slice() else {
            panic!("four history fields");
        };
        let actors = history(operations);
        let final_values = finals(final_values);
        let expected = match *expected {
            "compatible" => Ok(()),
            "incompatible" => Err(Error::Incompatible),
            other => panic!("unknown expected history outcome {other}"),
        };
        assert_eq!(check(&actors, &final_values), expected, "{name}");
        assert_eq!(serial(&actors, &final_values), expected == Ok(()), "{name}");
        cases += 1;
    }
    assert_eq!(cases, 9, "all named order and uncertainty cases ran");
}

fn serial(actors: &[Vec<Observation>], final_values: &BTreeMap<Id, i64>) -> bool {
    let mut state: BTreeMap<Id, i64> = final_values.keys().map(|&key| (key, 0)).collect();
    interleave(actors, &mut vec![0; actors.len()], &mut state, final_values)
}

fn interleave(
    actors: &[Vec<Observation>],
    positions: &mut [usize],
    state: &mut BTreeMap<Id, i64>,
    final_values: &BTreeMap<Id, i64>,
) -> bool {
    if actors
        .iter()
        .zip(positions.iter())
        .all(|(actor, &at)| at == actor.len())
    {
        return state == final_values;
    }
    for (actor, operations) in actors.iter().enumerate() {
        let Some(observation) = operations.get(positions[actor]) else {
            continue;
        };
        let old = state[&observation.key];
        positions[actor] += 1;
        if observation.outcome == Outcome::Uncertain
            && interleave(actors, positions, state, final_values)
        {
            positions[actor] -= 1;
            return true;
        }
        let matches = match observation.outcome {
            Outcome::Acknowledged(value) => value == old + 1,
            Outcome::Uncertain => true,
        };
        if matches {
            state.insert(observation.key, old + 1);
            let accepted = interleave(actors, positions, state, final_values);
            state.insert(observation.key, old);
            if accepted {
                positions[actor] -= 1;
                return true;
            }
        }
        positions[actor] -= 1;
    }
    false
}

fn patterns(keys: i64, maximum: i64) -> Vec<Vec<Observation>> {
    let mut patterns = Vec::new();
    for first_key in 1..=keys {
        for first_value in 1..=maximum {
            for second_key in 1..=keys {
                for second_value in 0..=maximum {
                    patterns.push(vec![
                        Observation {
                            key: Id(first_key),
                            outcome: Outcome::Acknowledged(first_value),
                        },
                        Observation {
                            key: Id(second_key),
                            outcome: if second_value == 0 {
                                Outcome::Uncertain
                            } else {
                                Outcome::Acknowledged(second_value)
                            },
                        },
                    ]);
                }
            }
        }
    }
    patterns
}

#[test]
fn agrees_with_bounded_serial_interleavings() {
    let input = fixture("history-search.txt");
    let values: Vec<usize> = input
        .split_whitespace()
        .map(|n| n.parse().expect("integer search bound"))
        .collect();
    let [
        actors,
        keys,
        calls,
        maximum,
        expected_cases,
        expected_uncertain,
    ] = values.as_slice()
    else {
        panic!("six search fixture fields");
    };
    assert_eq!(
        (*actors, *keys, *calls),
        (2, 2, 2),
        "the independent oracle enumerates this finite boundary"
    );
    let maximum = i64::try_from(*maximum).expect("fixture maximum fits");
    let patterns = patterns(i64::try_from(*keys).expect("fixture keys fit"), maximum);
    let (mut cases, mut uncertain, mut accepted, mut refused) = (0, 0, 0, 0);
    for first in &patterns {
        for second in &patterns {
            let histories = vec![first.clone(), second.clone()];
            for x in 0..=maximum {
                for y in 0..=maximum {
                    let final_values = BTreeMap::from([(Id(1), x), (Id(2), y)]);
                    let possible = serial(&histories, &final_values);
                    let expected = if possible {
                        Ok(())
                    } else {
                        Err(Error::Incompatible)
                    };
                    assert_eq!(
                        check(&histories, &final_values),
                        expected,
                        "{histories:?} {final_values:?}"
                    );
                    cases += 1;
                    uncertain += usize::from(
                        histories
                            .iter()
                            .flatten()
                            .any(|o| o.outcome == Outcome::Uncertain),
                    );
                    if possible {
                        accepted += 1;
                    } else {
                        refused += 1;
                    }
                }
            }
        }
    }
    assert_eq!((cases, uncertain), (*expected_cases, *expected_uncertain));
    assert!(
        accepted > 0 && refused > 0,
        "both compatible and incompatible histories were reached"
    );
}

#[test]
fn rejects_unbounded_or_nonterminal_uncertainty() {
    let mut checked = 0;
    for line in fixture("history-guards.txt").lines() {
        let fields: Vec<_> = line.split('|').collect();
        let [name, operations, final_values, expected] = fields.as_slice() else {
            panic!("four history guard fields");
        };
        let expected = match *expected {
            "terminal:0" => Error::UncertaintyNotTerminal(ActorId(0)),
            "actors:5" => Error::ActorCount(5),
            "key:3" => Error::UnknownKey(Id(3)),
            "incompatible" => Error::Incompatible,
            other => panic!("unknown history guard outcome {other}"),
        };
        assert_eq!(
            check(&history(operations), &finals(final_values)),
            Err(expected),
            "{name}"
        );
        checked += 1;
    }
    assert_eq!(checked, 5, "all uncertainty and pre-scan guards ran");
}
