use adtb_reference::{Rejection, Value};
use adtb_testkit::fixture::Fixture;

use super::shared::{Fields, bindings, finish, parse, take};

pub(super) enum Step {
    Run {
        source: String,
        params: Vec<(String, String, Value)>,
        expected: Option<Value>,
    },
    RejectKey {
        source: String,
        expected: Rejection,
    },
    One {
        source: String,
        expected: Value,
    },
    Set {
        source: String,
        expected: Value,
    },
    TableCount {
        relation: String,
        expected: usize,
    },
}

fn value(fixture: &Fixture, fields: &mut Fields) -> Value {
    let ty = take(fields, "type");
    parse(fixture, &ty, &take(fields, "expected"))
}

impl Step {
    pub(super) fn read(fixture: &Fixture, mode: &str, mut fields: Fields) -> Self {
        let step = match mode {
            "run" | "result" => Self::Run {
                source: take(&mut fields, "source"),
                params: bindings(fixture, fields.remove("params")),
                expected: if mode == "result" {
                    Some(value(fixture, &mut fields))
                } else {
                    None
                },
            },
            "reject-key" => Self::RejectKey {
                source: take(&mut fields, "source"),
                expected: Rejection::KeyViolation {
                    relation: take(&mut fields, "relation"),
                    key: take(&mut fields, "key"),
                },
            },
            "one" => Self::One {
                source: take(&mut fields, "source"),
                expected: value(fixture, &mut fields),
            },
            "set" => {
                let source = take(&mut fields, "source");
                let ty = take(&mut fields, "type");
                let rows = take(&mut fields, "expected");
                Self::Set {
                    source,
                    expected: Value::set(rows.lines().map(|line| parse(fixture, &ty, line))),
                }
            }
            "count" => Self::TableCount {
                relation: take(&mut fields, "relation"),
                expected: take(&mut fields, "expected")
                    .parse()
                    .expect("a table count is usize"),
            },
            _ => panic!("unknown write step mode {mode}"),
        };
        finish(&fields);
        step
    }

    pub(super) fn run(self, fixture: &mut Fixture) {
        match self {
            Self::Run {
                source,
                params,
                expected,
            } => {
                let borrowed: Vec<_> = params
                    .iter()
                    .map(|(n, t, v)| (n.as_str(), t.as_str(), v.clone()))
                    .collect();
                let out = fixture
                    .run_with(&source, &borrowed)
                    .expect("the write succeeds");
                if let Some(expected) = expected {
                    assert_eq!(expected, out.result, "exact write result");
                }
            }
            Self::RejectKey { source, expected } => {
                assert_eq!(expected, fixture.rejected(&source), "exact key refusal");
            }
            Self::One { source, expected } => {
                assert_eq!(expected, fixture.one(&source), "exact singleton value");
            }
            Self::Set { source, expected } => {
                assert_eq!(expected, fixture.value(&source), "exact set value");
            }
            Self::TableCount { relation, expected } => {
                let rows = fixture.table(&relation);
                assert_eq!(
                    expected,
                    rows.as_set().expect("a relation is a set").len(),
                    "exact table count for {relation}"
                );
            }
        }
    }
}
