use adtb_reference::{Rejection, Value};
use adtb_testkit::fixture::Fixture;

use super::shared::{Fields, finish, parse, take};

pub(super) enum Step {
    Loads(Loads),
    Catalog {
        source: String,
        relation: String,
        prefix: String,
    },
}

pub(super) struct Loads {
    source: String,
    names_query: String,
    names: Value,
    relation: String,
    count: usize,
    reject_query: String,
    metadata: Value,
    raw_contains: String,
    loads_query: String,
    loads_count: usize,
}

impl Step {
    pub(super) fn read(fixture: &Fixture, mode: &str, mut fields: Fields) -> Self {
        let source = take(&mut fields, "source");
        let step = if mode == "reject-catalog-unique" {
            Self::Catalog {
                source,
                relation: take(&mut fields, "relation"),
                prefix: take(&mut fields, "constraint-prefix"),
            }
        } else {
            Self::Loads(Loads {
                source,
                names_query: take(&mut fields, "names-query"),
                names: parse(fixture, "Set Text", &take(&mut fields, "names")),
                relation: take(&mut fields, "relation"),
                count: take(&mut fields, "count")
                    .parse()
                    .expect("row count is usize"),
                reject_query: take(&mut fields, "reject-query"),
                metadata: parse(
                    fixture,
                    "{ line: I64, source: Text, reason: Text }",
                    &take(&mut fields, "metadata"),
                ),
                raw_contains: take(&mut fields, "raw-contains"),
                loads_query: take(&mut fields, "loads-query"),
                loads_count: take(&mut fields, "loads-count")
                    .parse()
                    .expect("load count is usize"),
            })
        };
        finish(&fields);
        step
    }

    pub(super) fn run(self, fixture: &mut Fixture) {
        match self {
            Self::Loads(loads) => loads.run(fixture),
            Self::Catalog {
                source,
                relation,
                prefix,
            } => {
                let Rejection::Violated { constraint, .. } = fixture.rejected(&source) else {
                    panic!("expected a catalog-linked unique violation")
                };
                let relation = fixture.rel(&relation);
                let matched = fixture.schema.ids.index_ids().any(|((at, offset), id)| {
                    at == relation
                        && constraint == format!("{prefix}#{id}")
                        && id.raw() != u64::try_from(offset).expect("index offset fits u64")
                });
                assert!(
                    matched,
                    "{constraint} names a relation-linked catalog ID distinct from its local offset"
                );
            }
        }
    }
}

impl Loads {
    fn run(self, fixture: &mut Fixture) {
        let out = fixture.run(&self.source).expect("loaded");
        assert_eq!(Value::unit(), out.result, "exact load result");
        assert_eq!(
            self.names,
            fixture.value(&self.names_query),
            "exact loaded names"
        );
        assert_eq!(
            self.count,
            fixture.table(&self.relation).as_set().expect("a set").len(),
            "exact loaded cardinality"
        );
        let reject = fixture.one(&self.reject_query);
        let actual = Value::record(["line", "source", "reason"].map(|field| {
            (
                field,
                reject.field(field).expect("reject metadata field").clone(),
            )
        }));
        assert_eq!(self.metadata, actual, "exact reject metadata");
        let Some(Value::Text(raw)) = reject.field("raw") else {
            panic!("a raw record")
        };
        assert!(raw.contains(&self.raw_contains), "{raw}");
        let first = reject.field("load").cloned().expect("a load identity");
        fixture.ok(&self.source);
        let loads = fixture.value(&self.loads_query);
        let loads = loads.as_set().expect("a set");
        assert_eq!(
            self.loads_count,
            loads.len(),
            "exact distinct load cardinality"
        );
        assert!(
            loads.contains(&first),
            "first native load identity survives"
        );
    }
}
