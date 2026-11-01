use std::collections::BTreeSet;

use adtb_reference::{Abort, Fault, Rejection, Value};
use adtb_testkit::fixture::Fixture;

use super::shared::{Fields, bindings, finish, parse, take};

pub(super) struct Refusal {
    source: String,
    params: Vec<(String, String, Value)>,
    expected: Expected,
}

enum Expected {
    Exact(Abort),
    Violated { constraint: String, report: Report },
}

enum Report {
    Unchecked,
    Count(usize),
    Field { name: String, expected: Value },
}

impl Report {
    fn read(fixture: &Fixture, fields: &mut Fields) -> Self {
        match fields.remove("report").as_deref() {
            None => Self::Unchecked,
            Some("count") => Self::Count(
                take(fields, "expected")
                    .parse()
                    .expect("report count is usize"),
            ),
            Some("field") => {
                let name = take(fields, "field");
                let ty = take(fields, "type");
                let rows = take(fields, "expected");
                Self::Field {
                    name,
                    expected: Value::set(rows.lines().map(|row| parse(fixture, &ty, row))),
                }
            }
            Some(mode) => panic!("unknown report assertion {mode}"),
        }
    }

    fn check(self, rows: BTreeSet<Value>) {
        match self {
            Self::Unchecked => {}
            Self::Count(expected) => assert_eq!(expected, rows.len(), "exact report count"),
            Self::Field { name, expected } => {
                let actual = Value::set(rows.iter().map(|row| {
                    row.field(&name)
                        .unwrap_or_else(|| panic!("report row has field {name}"))
                        .clone()
                }));
                assert_eq!(expected, actual, "exact report field set {name}");
            }
        }
    }
}

impl Refusal {
    pub(super) fn read(fixture: &Fixture, mode: &str, mut fields: Fields) -> Self {
        let source = take(&mut fields, "source");
        let params = bindings(fixture, fields.remove("params"));
        let expected = match mode {
            "reject-user" => Expected::Exact(Abort::Reject(Rejection::UserError {
                message: take(&mut fields, "message"),
            })),
            "reject-bad-input" => Expected::Exact(Abort::Reject(Rejection::BadInput {
                source: take(&mut fields, "input-source"),
                line: take(&mut fields, "line")
                    .parse()
                    .expect("input line is i64"),
                detail: take(&mut fields, "detail"),
            })),
            "reject-key-changed" => Expected::Exact(Abort::Reject(Rejection::KeyChanged {
                relation: take(&mut fields, "relation"),
            })),
            "fault" => {
                let fault = match take(&mut fields, "fault").as_str() {
                    "DivisionByZero" => Fault::DivisionByZero,
                    other => panic!("unsupported fault {other}"),
                };
                Expected::Exact(Abort::Fault(fault))
            }
            "reject-violated" => Expected::Violated {
                constraint: take(&mut fields, "constraint"),
                report: Report::read(fixture, &mut fields),
            },
            _ => panic!("unknown refusal mode {mode}"),
        };
        finish(&fields);
        Self {
            source,
            params,
            expected,
        }
    }

    pub(super) fn run(self, fixture: &mut Fixture) {
        let borrowed: Vec<_> = self
            .params
            .iter()
            .map(|(n, t, v)| (n.as_str(), t.as_str(), v.clone()))
            .collect();
        let before = fixture.db.clone();
        let actual = fixture
            .run_with(&self.source, &borrowed)
            .map(|out| out.result);
        assert_eq!(before, fixture.db, "a refused statement changes nothing");
        match self.expected {
            Expected::Exact(expected) => assert_eq!(Err(expected), actual, "exact abort"),
            Expected::Violated { constraint, report } => {
                let Err(Abort::Reject(Rejection::Violated {
                    constraint: actual,
                    report: rows,
                })) = actual
                else {
                    panic!("expected constraint violation, got {actual:?}");
                };
                assert_eq!(constraint, actual, "exact constraint name");
                report.check(rows);
            }
        }
    }
}
