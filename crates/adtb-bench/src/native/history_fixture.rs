use super::counter::{Observation, Outcome};
use super::model::Id;
use std::collections::BTreeMap;

pub fn history(text: &str) -> Vec<Vec<Observation>> {
    text.split(';')
        .map(|actor| {
            actor
                .split_whitespace()
                .map(|operation| {
                    let (key, value) = operation.split_once(':').expect("key:outcome");
                    Observation {
                        key: Id(key.parse().expect("integer key")),
                        outcome: if value == "?" {
                            Outcome::Uncertain
                        } else {
                            Outcome::Acknowledged(value.parse().expect("integer return value"))
                        },
                    }
                })
                .collect()
        })
        .collect()
}

pub fn finals(text: &str) -> BTreeMap<Id, i64> {
    text.split_whitespace()
        .enumerate()
        .map(|(index, value)| {
            (
                Id(i64::try_from(index + 1).expect("fixture key fits")),
                value.parse().expect("integer final value"),
            )
        })
        .collect()
}
