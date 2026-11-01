use adtb_reference::Value;
use adtb_testkit::fixture::Fixture;

use super::shared::{Fields, bindings, finish, parse, take};

pub(super) enum Probe {
    Count {
        source: String,
        expected: usize,
    },
    Excludes {
        source: String,
        excluded: Value,
    },
    Record {
        source: String,
        tag: u32,
        fields: Vec<(String, Value)>,
    },
}

impl Probe {
    pub(super) fn read(fixture: &Fixture, mode: &str, mut fields: Fields) -> Self {
        let source = take(&mut fields, "source");
        let probe = match mode {
            "query-count" => Self::Count {
                source,
                expected: take(&mut fields, "expected")
                    .parse()
                    .expect("query count is usize"),
            },
            "one-excludes" => {
                let ty = take(&mut fields, "type");
                let rows = take(&mut fields, "excluded");
                assert!(!rows.is_empty(), "excluded native values are nonempty");
                Self::Excludes {
                    source,
                    excluded: Value::set(rows.lines().map(|row| parse(fixture, &ty, row))),
                }
            }
            "one-record" => {
                let tag = take(&mut fields, "tag")
                    .parse()
                    .expect("constructor tag is u32");
                let expected = bindings(fixture, Some(take(&mut fields, "fields")));
                assert!(!expected.is_empty(), "record assertions name fields");
                Self::Record {
                    source,
                    tag,
                    fields: expected.into_iter().map(|(n, _, v)| (n, v)).collect(),
                }
            }
            _ => panic!("unknown write probe mode {mode}"),
        };
        finish(&fields);
        probe
    }

    pub(super) fn run(self, fixture: &Fixture) {
        match self {
            Self::Count { source, expected } => {
                let value = fixture.value(&source);
                assert_eq!(
                    expected,
                    value.as_set().expect("query returns a set").len(),
                    "exact query count"
                );
            }
            Self::Excludes { source, excluded } => {
                let value = fixture.one(&source);
                assert!(
                    !excluded
                        .as_set()
                        .expect("excluded values are a set")
                        .contains(&value),
                    "singleton excludes specified native values"
                );
            }
            Self::Record {
                source,
                tag,
                fields,
            } => {
                let value = fixture.one(&source);
                assert_eq!(Some(tag), value.tag(), "exact raw constructor tag");
                for (name, expected) in fields {
                    assert_eq!(
                        Some(&expected),
                        value.field(&name),
                        "exact native field {name}"
                    );
                }
            }
        }
    }
}
