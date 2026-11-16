use adtb_lang::ids::{CatCtorId, engine_ctor as k};
use adtb_lang::schema::Schema;
use adtb_lang::schema::prelude::ctor;

use crate::key::Value;

use super::super::codec::testing::{Sample, Sampled, ids, position, tag};
use super::super::codec::{Codec, Decode};
use super::*;

fn some(v: Value) -> Value {
    Value::Sum {
        ctor: ctor::SOME,
        fields: vec![v],
    }
}

fn none() -> Value {
    Value::Sum {
        ctor: ctor::NONE,
        fields: Vec::new(),
    }
}

fn truth(b: bool) -> Value {
    Value::Sum {
        ctor: ctor::bool(b),
        fields: Vec::new(),
    }
}

fn text(t: &str) -> Value {
    Value::Text(t.into())
}

/// Returns the value of constructor `id` of the nullary sum field `field` of the one
/// constructor `of` of `T`'s relation in `schema`.
fn nullary<T: Decode>(schema: &Schema, of: CatCtorId, field: &str, id: CatCtorId) -> Value {
    let codec = Codec::<T>::new(schema);
    let at = (codec.relation(), position(schema, &codec, of));
    tag(schema, at, field, id, Vec::new())
}

impl Sampled for TypeRow {
    fn samples(_: &Schema) -> Vec<Sample<TypeRow>> {
        let head = |dropped: Option<i64>| {
            vec![
                ("id", Value::I64(11)),
                ("module", text("app")),
                ("name", text("Shape")),
                ("params", Value::I32(2)),
                (
                    "dropped",
                    dropped.map_or_else(none, |d| some(Value::I64(d))),
                ),
            ]
        };
        let want = |dropped, kind| TypeRow {
            id: 11,
            module: "app".into(),
            name: "Shape".into(),
            params: 2,
            dropped,
            kind,
        };
        let mut sum = head(None);
        sum.push(("ordered", truth(true)));
        let mut newtype = head(Some(4));
        newtype.push(("over", text("I64")));
        vec![
            (
                k::TYPE_SUM,
                sum,
                want(None, TypeKind::Sum { ordered: true }),
            ),
            (
                k::TYPE_NEWTYPE,
                newtype,
                want(Some(4), TypeKind::Newtype { over: "I64".into() }),
            ),
        ]
    }
}

impl Sampled for ConstructorRow {
    fn samples(_: &Schema) -> Vec<Sample<ConstructorRow>> {
        let fields = vec![
            ("id", Value::I64(21)),
            ("sum", Value::I64(11)),
            ("name", text("Circle")),
            ("position", Value::I32(1)),
            ("dropped", some(Value::I64(3))),
        ];
        let want = ConstructorRow {
            id: 21,
            sum: 11,
            name: "Circle".into(),
            position: 1,
            dropped: Some(3),
        };
        vec![(k::CONSTRUCTOR, fields, want)]
    }
}

impl Sampled for CtorStateRow {
    fn samples(schema: &Schema) -> Vec<Sample<CtorStateRow>> {
        ids(CTOR_STATE_STAGE)
            .into_iter()
            .map(|stage| {
                let fields = vec![
                    ("relation", Value::I64(31)),
                    ("ctor", Value::I64(21)),
                    (
                        "stage",
                        nullary::<CtorStateRow>(schema, k::CTOR_STATE, "stage", stage),
                    ),
                ];
                let want = CtorStateRow {
                    relation: 31,
                    ctor: 21,
                    stage,
                };
                (k::CTOR_STATE, fields, want)
            })
            .collect()
    }
}

impl Sampled for ValueStateRow {
    fn samples(schema: &Schema) -> Vec<Sample<ValueStateRow>> {
        ids(VALUE_STATE_STAGE)
            .into_iter()
            .map(|stage| {
                let fields = vec![
                    ("ctor", Value::I64(21)),
                    (
                        "stage",
                        nullary::<ValueStateRow>(schema, k::VALUE_STATE, "stage", stage),
                    ),
                ];
                (k::VALUE_STATE, fields, ValueStateRow { ctor: 21, stage })
            })
            .collect()
    }
}

impl Sampled for FieldRow {
    fn samples(_: &Schema) -> Vec<Sample<FieldRow>> {
        let head = || {
            vec![
                ("id", Value::I64(41)),
                ("name", text("radius")),
                ("field_type", text("F64")),
                ("position", Value::I32(2)),
                ("dropped", none()),
            ]
        };
        let want = |holder| FieldRow {
            id: 41,
            name: "radius".into(),
            field_type: "F64".into(),
            position: 2,
            dropped: None,
            holder,
        };
        let mut header = head();
        header.push(("sum", Value::I64(11)));
        let mut payload = head();
        payload.push(("ctor", Value::I64(21)));
        vec![
            (
                k::FIELD_HEADER,
                header,
                want(FieldHolder::Header { sum: 11 }),
            ),
            (
                k::FIELD_PAYLOAD,
                payload,
                want(FieldHolder::Payload { ctor: 21 }),
            ),
        ]
    }
}

impl Sampled for RelationRow {
    fn samples(schema: &Schema) -> Vec<Sample<RelationRow>> {
        ids(RELATION_KIND)
            .into_iter()
            .map(|kind| {
                let fields = vec![
                    ("id", Value::I64(31)),
                    ("module", text("app")),
                    ("name", text("shapes")),
                    ("sum", Value::I64(11)),
                    (
                        "kind",
                        nullary::<RelationRow>(schema, k::RELATION, "kind", kind),
                    ),
                    ("dropped", none()),
                ];
                let want = RelationRow {
                    id: 31,
                    module: "app".into(),
                    name: "shapes".into(),
                    sum: 11,
                    kind,
                    dropped: None,
                };
                (k::RELATION, fields, want)
            })
            .collect()
    }
}

impl Sampled for NamedSetRow {
    fn samples(_: &Schema) -> Vec<Sample<NamedSetRow>> {
        let fields = vec![
            ("id", Value::I64(51)),
            ("relation", Value::I64(31)),
            ("name", text("round")),
            ("dropped", some(Value::I64(2))),
        ];
        let want = NamedSetRow {
            id: 51,
            relation: 31,
            name: "round".into(),
            dropped: Some(2),
        };
        vec![(k::NAMED_SET, fields, want)]
    }
}

impl Sampled for SetMemberRow {
    fn samples(_: &Schema) -> Vec<Sample<SetMemberRow>> {
        let fields = vec![("set", Value::I64(51)), ("ctor", Value::I64(21))];
        vec![(k::SET_MEMBER, fields, SetMemberRow { set: 51, ctor: 21 })]
    }
}

impl Sampled for IndexRow {
    fn samples(schema: &Schema) -> Vec<Sample<IndexRow>> {
        ids(INDEX_KIND)
            .into_iter()
            .map(|kind| {
                let fields = vec![
                    ("id", Value::I64(61)),
                    ("relation", Value::I64(31)),
                    ("kind", nullary::<IndexRow>(schema, k::INDEX, "kind", kind)),
                    ("clause", text("unique (name)")),
                    ("dropped", none()),
                ];
                let want = IndexRow {
                    id: 61,
                    relation: 31,
                    kind,
                    clause: "unique (name)".into(),
                    dropped: None,
                };
                (k::INDEX, fields, want)
            })
            .collect()
    }
}

impl Sampled for IndexCtorRow {
    fn samples(_: &Schema) -> Vec<Sample<IndexCtorRow>> {
        let fields = vec![("index", Value::I64(61)), ("ctor", Value::I64(21))];
        vec![(
            k::INDEX_CTOR,
            fields,
            IndexCtorRow {
                index: 61,
                ctor: 21,
            },
        )]
    }
}

impl Sampled for ConstraintRow {
    fn samples(_: &Schema) -> Vec<Sample<ConstraintRow>> {
        let fields = vec![
            ("id", Value::I64(71)),
            ("module", text("app")),
            ("name", text("positive")),
            ("text", text("constraint positive = ...")),
            ("dropped", none()),
        ];
        let want = ConstraintRow {
            id: 71,
            module: "app".into(),
            name: "positive".into(),
            text: "constraint positive = ...".into(),
            dropped: None,
        };
        vec![(k::CONSTRAINT, fields, want)]
    }
}

impl Sampled for FunctionRow {
    fn samples(_: &Schema) -> Vec<Sample<FunctionRow>> {
        let head = || {
            vec![
                ("id", Value::I64(81)),
                ("module", text("app")),
                ("name", text("area")),
                ("signature", text("(Shape) -> F64")),
                ("deterministic", truth(true)),
                ("hash", Value::Bytes(vec![9; 32])),
                ("dropped", none()),
            ]
        };
        let want = |code| FunctionRow {
            id: 81,
            module: "app".into(),
            name: "area".into(),
            signature: "(Shape) -> F64".into(),
            deterministic: true,
            hash: vec![9; 32],
            dropped: None,
            code,
        };
        let with = |own: Vec<(&'static str, Value)>| {
            let mut fields = head();
            fields.extend(own);
            fields
        };
        vec![
            (
                k::FUNCTION_LANGUAGE,
                with(vec![("body", text("s.r * s.r"))]),
                want(FunctionCode::Language {
                    body: "s.r * s.r".into(),
                }),
            ),
            (
                k::FUNCTION_WASM,
                with(vec![
                    ("code", Value::Bytes(vec![7; 32])),
                    ("export", text("area")),
                ]),
                want(FunctionCode::Wasm {
                    code: vec![7; 32],
                    export: "area".into(),
                }),
            ),
            (
                k::FUNCTION_HOST,
                with(vec![("symbol", text("area")), ("version", text("1"))]),
                want(FunctionCode::Host {
                    symbol: "area".into(),
                    version: "1".into(),
                }),
            ),
            (
                k::FUNCTION_AGGREGATE,
                with(vec![("body", text("sum"))]),
                want(FunctionCode::Aggregate { body: "sum".into() }),
            ),
        ]
    }
}

impl Sampled for DefaultRow {
    fn samples(_: &Schema) -> Vec<Sample<DefaultRow>> {
        let fields = vec![
            ("id", Value::I64(101)),
            ("relation", Value::I64(31)),
            ("field", Value::I64(41)),
            ("text", text("0.0")),
            ("dropped", none()),
        ];
        let want = DefaultRow {
            id: 101,
            relation: 31,
            field: 41,
            text: "0.0".into(),
            dropped: None,
        };
        vec![(k::DEFAULT, fields, want)]
    }
}

impl Sampled for ComputedRow {
    fn samples(_: &Schema) -> Vec<Sample<ComputedRow>> {
        let fields = vec![
            ("id", Value::I64(111)),
            ("relation", Value::I64(31)),
            ("name", text("area")),
            ("field_type", text("F64")),
            ("text", text("r * r")),
            ("dropped", none()),
        ];
        let want = ComputedRow {
            id: 111,
            relation: 31,
            name: "area".into(),
            field_type: "F64".into(),
            text: "r * r".into(),
            dropped: None,
        };
        vec![(k::COMPUTED, fields, want)]
    }
}
