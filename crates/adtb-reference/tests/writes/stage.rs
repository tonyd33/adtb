use adtb_lang::types::{RelId, TypeId};
use adtb_reference::{Rejection, Stage};
use adtb_testkit::fixture::Fixture;

use super::shared::{Fields, finish, take};

pub(super) enum Step {
    Closed { key: (TypeId, u32), stage: Stage },
    Relation { key: (RelId, u32), stage: Stage },
    Value { key: (TypeId, u32), stage: Stage },
    Refuse { source: String, expected: Rejection },
}

fn stage(fields: &mut Fields) -> Stage {
    match take(fields, "stage").as_str() {
        "Declared" => Stage::Declared,
        "Deprecated" => Stage::Deprecated,
        other => panic!("unknown closed constructor stage {other}"),
    }
}

fn schema_stage(stage: Stage) -> adtb_lang::schema::Stage {
    match stage {
        Stage::Declared => adtb_lang::schema::Stage::Declared,
        Stage::Deprecated => adtb_lang::schema::Stage::Deprecated,
    }
}

impl Step {
    pub(super) fn read(fixture: &Fixture, mode: &str, mut fields: Fields) -> Self {
        let stage = stage(&mut fields);
        let step = match mode {
            "stage-closed" | "stage-relation" | "stage-value" => {
                let ty = take(&mut fields, "type");
                let ctor = take(&mut fields, "ctor");
                let key = fixture.ctor(&ty, &ctor);
                match mode {
                    "stage-closed" => Self::Closed { key, stage },
                    "stage-relation" => Self::Relation {
                        key: (fixture.rel(&take(&mut fields, "relation")), key.1),
                        stage,
                    },
                    "stage-value" => Self::Value { key, stage },
                    _ => unreachable!("the stage hook mode was matched"),
                }
            }
            "reject-not-open" => Self::Refuse {
                source: take(&mut fields, "source"),
                expected: Rejection::NotOpen {
                    ctor: take(&mut fields, "ctor"),
                    stage,
                },
            },
            _ => panic!("unknown constructor stage step {mode}"),
        };
        finish(&fields);
        step
    }

    pub(super) fn run(self, fixture: &mut Fixture) {
        match self {
            Self::Closed { key, stage } => {
                fixture.closed.insert(key, stage);
            }
            Self::Relation { key, stage } => {
                fixture.schema.stages.insert(key, schema_stage(stage));
            }
            Self::Value { key, stage } => {
                fixture.schema.value_stages.insert(key, schema_stage(stage));
            }
            Self::Refuse { source, expected } => {
                assert_eq!(expected, fixture.rejected(&source), "exact NotOpen refusal");
            }
        }
    }
}
