use adtb_reference::{Rejection, Value};
use adtb_testkit::fixture::Fixture;

use super::shared::{Fields, bindings, finish, parse, take};

enum Mode {
    Run(Vec<(String, String, Value)>),
    RejectKey(Rejection),
    One(Value),
    Set(Value),
}

pub(super) struct Step {
    source: String,
    mode: Mode,
}

impl Step {
    pub(super) fn read(fixture: &Fixture, mode: &str, mut fields: Fields) -> Self {
        let source = take(&mut fields, "source");
        let mode = match mode {
            "run" => Mode::Run(bindings(fixture, fields.remove("params"))),
            "reject-key" => Mode::RejectKey(Rejection::KeyViolation {
                relation: take(&mut fields, "relation"),
                key: take(&mut fields, "key"),
            }),
            "one" | "set" => {
                let ty = take(&mut fields, "type");
                let expected = take(&mut fields, "expected");
                if mode == "one" {
                    Mode::One(parse(fixture, &ty, &expected))
                } else {
                    Mode::Set(Value::set(
                        expected.lines().map(|line| parse(fixture, &ty, line)),
                    ))
                }
            }
            _ => panic!("unknown write step mode {mode}"),
        };
        finish(&fields);
        Self { source, mode }
    }

    pub(super) fn run(self, fixture: &mut Fixture) {
        match self.mode {
            Mode::Run(params) => {
                let borrowed: Vec<_> = params
                    .iter()
                    .map(|(n, t, v)| (n.as_str(), t.as_str(), v.clone()))
                    .collect();
                fixture
                    .run_with(&self.source, &borrowed)
                    .expect("the write succeeds");
            }
            Mode::RejectKey(expected) => {
                assert_eq!(
                    expected,
                    fixture.rejected(&self.source),
                    "exact key refusal"
                );
            }
            Mode::One(expected) => {
                assert_eq!(expected, fixture.one(&self.source), "exact singleton value");
            }
            Mode::Set(expected) => {
                assert_eq!(expected, fixture.value(&self.source), "exact set value");
            }
        }
    }
}
