use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::check::check_adhoc;
use adtb_lang::kernel::Kind;
use adtb_lang::parse::{Options, parse_adhoc};
use adtb_reference::{Abort, Rejection, Value};
use adtb_testkit::fixture::Fixture;

use super::shared::{Fields, finish, parse, take};

pub(super) struct Step {
    source: String,
    old_type: adtb_lang::types::Ty,
    expected: Abort,
}

impl Step {
    pub(super) fn read(fixture: &Fixture, mode: &str, mut fields: Fields) -> Self {
        let source = take(&mut fields, "source");
        let old_type = fixture.ty(&take(&mut fields, "old-type"));
        let rejection = match mode {
            "restricted-old-key" => Rejection::KeyViolation {
                relation: take(&mut fields, "relation"),
                key: take(&mut fields, "key"),
            },
            "restricted-old-user" => Rejection::UserError {
                message: take(&mut fields, "message"),
            },
            "restricted-old-violated" => {
                let constraint = take(&mut fields, "constraint");
                let ty = take(&mut fields, "type");
                let report = take(&mut fields, "expected");
                Rejection::Violated {
                    constraint,
                    report: report
                        .lines()
                        .map(|row| parse(fixture, &ty, row))
                        .collect::<BTreeSet<Value>>(),
                }
            }
            _ => panic!("unknown restricted-old refusal {mode}"),
        };
        finish(&fields);
        Self {
            source,
            old_type,
            expected: Abort::Reject(rejection),
        }
    }

    pub(super) fn run(self, fixture: &mut Fixture) {
        let items = parse_adhoc(&self.source, Options::default())
            .ok()
            .expect("the upsert parses");
        let mut checked = check_adhoc(&fixture.schema, &fixture.program, &items, &BTreeMap::new())
            .expect("the upsert checks");
        let olds: Vec<_> = checked
            .terms
            .nodes
            .iter()
            .filter_map(|node| match node.kind {
                Kind::Upsert { old, .. } => Some(old),
                _ => None,
            })
            .collect();
        let [old] = olds.as_slice() else {
            panic!("exactly one upsert old variable")
        };
        let [item] = checked.items.as_slice() else {
            panic!("exactly one restricted-old statement")
        };
        checked.terms.vars[old.0 as usize].ty = self.old_type;
        let mut supply = fixture.supply.clone();
        let actual = fixture
            .interp()
            .run(&checked.terms, *item, &[], &mut supply);
        assert_eq!(Err(self.expected), actual, "exact restricted-old abort");
    }
}
