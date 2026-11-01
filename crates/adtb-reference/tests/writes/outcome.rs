use adtb_lang::types::RelId;
use adtb_reference::{Database, Outcome, Value};
use adtb_testkit::fixture::Fixture;

use super::shared::{Fields, bindings, finish, parse, take};

pub(super) struct Success {
    source: String,
    params: Vec<(String, String, Value)>,
    clock: Option<i64>,
    expected: Option<Value>,
    changes: Vec<ChangeCheck>,
    unchanged: bool,
}

#[derive(Clone, Copy)]
enum Side {
    Inserted,
    Deleted,
}

impl Side {
    fn read(name: &str) -> Self {
        match name {
            "inserted" => Self::Inserted,
            "deleted" => Self::Deleted,
            _ => panic!("unknown change side {name}"),
        }
    }

    fn rows(self, out: &Outcome) -> &Database {
        match self {
            Self::Inserted => &out.change.inserted,
            Self::Deleted => &out.change.deleted,
        }
    }
}

enum ChangeCheck {
    Empty(Side),
    Count(Side, (RelId, u32), usize),
    Present(Side, (RelId, u32)),
}

impl ChangeCheck {
    fn read(fixture: &Fixture, line: &str) -> Self {
        let parts: Vec<_> = line.split('|').collect();
        match parts.as_slice() {
            [side, "empty"] => Self::Empty(Side::read(side)),
            [side, "count", relation, ctor, expected] => Self::Count(
                Side::read(side),
                (
                    fixture.rel(relation),
                    ctor.parse().expect("constructor tag is u32"),
                ),
                expected.parse().expect("change count is usize"),
            ),
            [side, "present", relation, ctor] => Self::Present(
                Side::read(side),
                (
                    fixture.rel(relation),
                    ctor.parse().expect("constructor tag is u32"),
                ),
            ),
            _ => panic!("invalid change assertion {line}"),
        }
    }

    fn check(self, out: &Outcome) {
        match self {
            Self::Empty(side) => assert!(side.rows(out).is_empty(), "complete change map is empty"),
            Self::Count(side, key, expected) => {
                let actual = side
                    .rows(out)
                    .get(&key)
                    .map_or(0, std::collections::BTreeSet::len);
                assert_eq!(expected, actual, "exact constructor change count");
            }
            Self::Present(side, key) => {
                assert!(
                    side.rows(out).contains_key(&key),
                    "native constructor change entry exists"
                );
            }
        }
    }
}

impl Success {
    pub(super) fn read(fixture: &Fixture, mut fields: Fields) -> Self {
        let source = take(&mut fields, "source");
        let params = bindings(fixture, fields.remove("params"));
        let clock = fields
            .remove("clock")
            .map(|s| s.parse().expect("clock is i64"));
        let expected = fields
            .remove("type")
            .map(|ty| parse(fixture, &ty, &take(&mut fields, "expected")));
        let changes = fields
            .remove("changes")
            .into_iter()
            .flat_map(|s| {
                s.lines()
                    .map(|line| ChangeCheck::read(fixture, line))
                    .collect::<Vec<_>>()
            })
            .collect();
        let unchanged = fields.remove("unchanged").is_some_and(|s| {
            assert_eq!(s, "true", "unchanged requests a database equality check");
            true
        });
        finish(&fields);
        Self {
            source,
            params,
            clock,
            expected,
            changes,
            unchanged,
        }
    }

    pub(super) fn run(self, fixture: &mut Fixture) {
        if let Some(clock) = self.clock {
            fixture.supply.clock = clock;
        }
        let before = self.unchanged.then(|| fixture.db.clone());
        let borrowed: Vec<_> = self
            .params
            .iter()
            .map(|(n, t, v)| (n.as_str(), t.as_str(), v.clone()))
            .collect();
        let out = fixture
            .run_with(&self.source, &borrowed)
            .expect("the write succeeds");
        if let Some(expected) = self.expected {
            assert_eq!(expected, out.result, "exact write result");
        }
        for check in self.changes {
            check.check(&out);
        }
        if let Some(before) = before {
            assert_eq!(before, out.db, "query preserves the pre-statement database");
        }
    }
}
