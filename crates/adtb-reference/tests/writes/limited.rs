use std::cell::Cell;

use adtb_lang::check::check_adhoc;
use adtb_lang::parse::{Options, parse_adhoc};
use adtb_reference::{Abort, Rejection, Value};
use adtb_testkit::fixture::Fixture;

use super::shared::{Fields, finish, take};

pub(super) struct Step {
    source: String,
    limit: usize,
    expected: Option<(String, String)>,
}

impl Step {
    pub(super) fn read(mode: &str, mut fields: Fields) -> Self {
        let source = take(&mut fields, "source");
        let limit = take(&mut fields, "limit")
            .parse()
            .expect("a row limit is usize");
        let expected = (mode == "reject-row-limit")
            .then(|| (take(&mut fields, "relation"), take(&mut fields, "ctor")));
        finish(&fields);
        Self {
            source,
            limit,
            expected,
        }
    }

    pub(super) fn run(self, fixture: &mut Fixture) {
        let items = parse_adhoc(&self.source, Options::default())
            .ok()
            .expect("the limited statement parses");
        let checked = check_adhoc(
            &fixture.schema,
            &fixture.program,
            &items,
            &Default::default(),
        )
        .expect("the limited statement checks");
        let [item] = checked.items.as_slice() else {
            panic!("a row-limit case holds exactly one statement")
        };
        let normal = adtb_lang::norm::adhoc(&fixture.schema, &fixture.program, &checked)
            .expect("the limited statement normalizes");
        let measured = Cell::new(0);
        let measure = |relation, _, row: &Value| {
            measured.set(measured.get() + 1);
            let ty = fixture.schema.relation(relation).elem.clone();
            let size = adtb_reference::textual::print(&fixture.schema, row, &ty)
                .expect("a measured row prints")
                .len();
            (size, self.limit)
        };
        let before = fixture.db.clone();
        let out = fixture.interp().with_row_limit(&measure).run(
            &checked.terms,
            *item,
            &[],
            &mut fixture.supply.clone(),
        );
        assert!(
            measured.get() > 0,
            "the original statement measures a written row"
        );
        measured.set(0);
        let normalized = fixture.interp().with_row_limit(&measure).run(
            &normal.terms,
            normal.items[0],
            &[],
            &mut fixture.supply.clone(),
        );
        assert!(
            measured.get() > 0,
            "the normalized statement measures a written row"
        );
        assert_eq!(
            out, normalized,
            "limited original and normal outcomes agree"
        );
        match self.expected {
            Some((relation, ctor)) => {
                let Err(Abort::Reject(Rejection::RowTooLarge {
                    relation: actual_relation,
                    ctor: actual_ctor,
                    size,
                    limit,
                })) = out
                else {
                    panic!("expected RowTooLarge, got {out:?}")
                };
                assert_eq!(
                    (relation, ctor),
                    (actual_relation, actual_ctor),
                    "exact oversized row identity"
                );
                assert_eq!(
                    i64::try_from(self.limit).expect("the limit fits i64"),
                    limit,
                    "exact row limit"
                );
                assert!(size > limit, "reported size {size} exceeds limit {limit}");
                assert_eq!(before, fixture.db, "a limited refusal changes nothing");
            }
            None => {
                let out = out.expect("the written row fits");
                assert_eq!(Value::unit(), out.result, "exact limited write result");
                fixture.db = out.db;
            }
        }
    }
}
