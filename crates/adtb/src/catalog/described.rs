//! The catalog relations that describe a schema's objects, each read and written by its codec.

use adtb_lang::ids::engine_ctor as k;

use super::codec::{Ctor, Decode, Field, Fields, Relation, Row, Sum, field, relation, sum};
use super::ids::CatCtorId;

#[cfg(test)]
#[path = "described_tests.rs"]
mod tests;

/// The fields of the header of `catalog::TypeDef`.
#[derive(Clone, Copy)]
pub struct TypeHead {
    id: Field,
    module: Field,
    name: Field,
    params: Field,
    dropped: Field,
}

impl TypeHead {
    pub const fn id(self) -> Field {
        self.id
    }

    pub const fn module(self) -> Field {
        self.module
    }

    pub const fn name(self) -> Field {
        self.name
    }

    pub const fn params(self) -> Field {
        self.params
    }

    pub const fn dropped(self) -> Field {
        self.dropped
    }
}

const fn type_head(fields: Fields) -> TypeHead {
    TypeHead {
        id: field(fields, "id"),
        module: field(fields, "module"),
        name: field(fields, "name"),
        params: field(fields, "params"),
        dropped: field(fields, "dropped"),
    }
}

const TYPE_SUM: Fields = &["id", "module", "name", "params", "dropped", "ordered"];
const TYPE_NEWTYPE: Fields = &["id", "module", "name", "params", "dropped", "over"];
pub const TYPE_SUM_HEAD: TypeHead = type_head(TYPE_SUM);
pub const TYPE_NEWTYPE_HEAD: TypeHead = type_head(TYPE_NEWTYPE);
pub const TYPE_ORDERED: Field = field(TYPE_SUM, "ordered");
pub const TYPE_OVER: Field = field(TYPE_NEWTYPE, "over");

/// What a row of `catalog::types` declares beside its header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeKind {
    Sum { ordered: bool },
    Newtype { over: String },
}

/// A row of `catalog::types`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeRow {
    id: i64,
    module: String,
    name: String,
    params: i64,
    dropped: Option<i64>,
    kind: TypeKind,
}

fn type_row(r: &Row<'_>, head: TypeHead, kind: TypeKind) -> TypeRow {
    TypeRow {
        id: r.int(head.id),
        module: r.text(head.module).to_owned(),
        name: r.text(head.name).to_owned(),
        params: r.int32(head.params),
        dropped: r.optional_int(head.dropped),
        kind,
    }
}

impl Relation for relation::Types {
    type Row = TypeRow;
}

impl Decode for TypeRow {
    type Relation = relation::Types;
    const CTORS: &'static [Ctor<TypeRow>] = &[
        Ctor::new(k::TYPE_SUM, TYPE_SUM, |r| {
            let ordered = r.flag(TYPE_ORDERED);
            Ok(type_row(r, TYPE_SUM_HEAD, TypeKind::Sum { ordered }))
        }),
        Ctor::new(k::TYPE_NEWTYPE, TYPE_NEWTYPE, |r| {
            let over = r.text(TYPE_OVER).to_owned();
            Ok(type_row(r, TYPE_NEWTYPE_HEAD, TypeKind::Newtype { over }))
        }),
    ];
}

const CONSTRUCTOR: Fields = &["id", "sum", "name", "position", "dropped"];
pub const CONSTRUCTOR_ID: Field = field(CONSTRUCTOR, "id");
pub const CONSTRUCTOR_SUM: Field = field(CONSTRUCTOR, "sum");
pub const CONSTRUCTOR_NAME: Field = field(CONSTRUCTOR, "name");
pub const CONSTRUCTOR_POSITION: Field = field(CONSTRUCTOR, "position");
pub const CONSTRUCTOR_DROPPED: Field = field(CONSTRUCTOR, "dropped");

/// A row of `catalog::constructors`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstructorRow {
    id: i64,
    sum: i64,
    name: String,
    position: i64,
    dropped: Option<i64>,
}

impl Relation for relation::Constructors {
    type Row = ConstructorRow;
}

impl Decode for ConstructorRow {
    type Relation = relation::Constructors;
    const CTORS: &'static [Ctor<ConstructorRow>] = &[Ctor::new(k::CONSTRUCTOR, CONSTRUCTOR, |r| {
        Ok(ConstructorRow {
            id: r.int(CONSTRUCTOR_ID),
            sum: r.int(CONSTRUCTOR_SUM),
            name: r.text(CONSTRUCTOR_NAME).to_owned(),
            position: r.int32(CONSTRUCTOR_POSITION),
            dropped: r.optional_int(CONSTRUCTOR_DROPPED),
        })
    })];
}

const STAGES: &[(CatCtorId, &[crate::key::Scalar])] = &[
    (k::STAGE_DECLARED, &[]),
    (k::STAGE_ENABLED, &[]),
    (k::STAGE_DEPRECATED, &[]),
];

const CTOR_STATE: Fields = &["relation", "ctor", "stage"];
pub const CTOR_STATE_RELATION: Field = field(CTOR_STATE, "relation");
pub const CTOR_STATE_CTOR: Field = field(CTOR_STATE, "ctor");
pub const CTOR_STATE_STAGE: Sum = sum(field(CTOR_STATE, "stage"), STAGES);

/// A row of `catalog::ctor_states`, its stage by the fixed id of its `CtorStage` constructor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CtorStateRow {
    relation: i64,
    ctor: i64,
    stage: CatCtorId,
}

impl Relation for relation::CtorStates {
    type Row = CtorStateRow;
}

impl Decode for CtorStateRow {
    type Relation = relation::CtorStates;
    const CTORS: &'static [Ctor<CtorStateRow>] = &[Ctor::new(k::CTOR_STATE, CTOR_STATE, |r| {
        Ok(CtorStateRow {
            relation: r.int(CTOR_STATE_RELATION),
            ctor: r.int(CTOR_STATE_CTOR),
            stage: r.ctor(CTOR_STATE_STAGE).0,
        })
    })
    .with_sums(&[CTOR_STATE_STAGE])];
}

const VALUE_STATE: Fields = &["ctor", "stage"];
pub const VALUE_STATE_CTOR: Field = field(VALUE_STATE, "ctor");
pub const VALUE_STATE_STAGE: Sum = sum(field(VALUE_STATE, "stage"), STAGES);

/// A row of `catalog::value_states`, its stage by the fixed id of its `CtorStage` constructor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueStateRow {
    ctor: i64,
    stage: CatCtorId,
}

impl Relation for relation::ValueStates {
    type Row = ValueStateRow;
}

impl Decode for ValueStateRow {
    type Relation = relation::ValueStates;
    const CTORS: &'static [Ctor<ValueStateRow>] = &[Ctor::new(k::VALUE_STATE, VALUE_STATE, |r| {
        Ok(ValueStateRow {
            ctor: r.int(VALUE_STATE_CTOR),
            stage: r.ctor(VALUE_STATE_STAGE).0,
        })
    })
    .with_sums(&[VALUE_STATE_STAGE])];
}

/// The fields of the header of `catalog::Field`.
#[derive(Clone, Copy)]
pub struct FieldHead {
    id: Field,
    name: Field,
    field_type: Field,
    position: Field,
    dropped: Field,
}

impl FieldHead {
    pub const fn id(self) -> Field {
        self.id
    }

    pub const fn name(self) -> Field {
        self.name
    }

    pub const fn field_type(self) -> Field {
        self.field_type
    }

    pub const fn position(self) -> Field {
        self.position
    }

    pub const fn dropped(self) -> Field {
        self.dropped
    }
}

const fn field_head(fields: Fields) -> FieldHead {
    FieldHead {
        id: field(fields, "id"),
        name: field(fields, "name"),
        field_type: field(fields, "field_type"),
        position: field(fields, "position"),
        dropped: field(fields, "dropped"),
    }
}

const FIELD_HEADER: Fields = &["id", "name", "field_type", "position", "dropped", "sum"];
const FIELD_PAYLOAD: Fields = &["id", "name", "field_type", "position", "dropped", "ctor"];
pub const FIELD_HEADER_HEAD: FieldHead = field_head(FIELD_HEADER);
pub const FIELD_PAYLOAD_HEAD: FieldHead = field_head(FIELD_PAYLOAD);
pub const FIELD_SUM: Field = field(FIELD_HEADER, "sum");
pub const FIELD_CTOR: Field = field(FIELD_PAYLOAD, "ctor");

/// What holds the field a row of `catalog::fields` declares: a sum's header or a constructor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldHolder {
    Header { sum: i64 },
    Payload { ctor: i64 },
}

/// A row of `catalog::fields`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldRow {
    id: i64,
    name: String,
    field_type: String,
    position: i64,
    dropped: Option<i64>,
    holder: FieldHolder,
}

fn field_row(r: &Row<'_>, head: FieldHead, holder: FieldHolder) -> FieldRow {
    FieldRow {
        id: r.int(head.id),
        name: r.text(head.name).to_owned(),
        field_type: r.text(head.field_type).to_owned(),
        position: r.int32(head.position),
        dropped: r.optional_int(head.dropped),
        holder,
    }
}

impl Relation for relation::Fields {
    type Row = FieldRow;
}

impl Decode for FieldRow {
    type Relation = relation::Fields;
    const CTORS: &'static [Ctor<FieldRow>] = &[
        Ctor::new(k::FIELD_HEADER, FIELD_HEADER, |r| {
            let sum = r.int(FIELD_SUM);
            Ok(field_row(r, FIELD_HEADER_HEAD, FieldHolder::Header { sum }))
        }),
        Ctor::new(k::FIELD_PAYLOAD, FIELD_PAYLOAD, |r| {
            let ctor = r.int(FIELD_CTOR);
            Ok(field_row(
                r,
                FIELD_PAYLOAD_HEAD,
                FieldHolder::Payload { ctor },
            ))
        }),
    ];
}

const RELATION: Fields = &["id", "module", "name", "sum", "kind", "dropped"];
pub const RELATION_ID: Field = field(RELATION, "id");
pub const RELATION_MODULE: Field = field(RELATION, "module");
pub const RELATION_NAME: Field = field(RELATION, "name");
pub const RELATION_SUM: Field = field(RELATION, "sum");
pub const RELATION_KIND: Sum = sum(
    field(RELATION, "kind"),
    &[
        (k::REL_BASE, &[]),
        (k::REL_STORED, &[]),
        (k::REL_CATALOG, &[]),
        (k::REL_VOLATILE, &[]),
    ],
);
pub const RELATION_DROPPED: Field = field(RELATION, "dropped");

/// A row of `catalog::relations`, its kind by the fixed id of its `RelKind` constructor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationRow {
    id: i64,
    module: String,
    name: String,
    sum: i64,
    kind: CatCtorId,
    dropped: Option<i64>,
}

impl Relation for relation::Relations {
    type Row = RelationRow;
}

impl Decode for RelationRow {
    type Relation = relation::Relations;
    const CTORS: &'static [Ctor<RelationRow>] = &[Ctor::new(k::RELATION, RELATION, |r| {
        Ok(RelationRow {
            id: r.int(RELATION_ID),
            module: r.text(RELATION_MODULE).to_owned(),
            name: r.text(RELATION_NAME).to_owned(),
            sum: r.int(RELATION_SUM),
            kind: r.ctor(RELATION_KIND).0,
            dropped: r.optional_int(RELATION_DROPPED),
        })
    })
    .with_sums(&[RELATION_KIND])];
}

const NAMED_SET: Fields = &["id", "relation", "name", "dropped"];
pub const NAMED_SET_ID: Field = field(NAMED_SET, "id");
pub const NAMED_SET_RELATION: Field = field(NAMED_SET, "relation");
pub const NAMED_SET_NAME: Field = field(NAMED_SET, "name");
pub const NAMED_SET_DROPPED: Field = field(NAMED_SET, "dropped");

/// A row of `catalog::named_sets`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedSetRow {
    id: i64,
    relation: i64,
    name: String,
    dropped: Option<i64>,
}

impl Relation for relation::NamedSets {
    type Row = NamedSetRow;
}

impl Decode for NamedSetRow {
    type Relation = relation::NamedSets;
    const CTORS: &'static [Ctor<NamedSetRow>] = &[Ctor::new(k::NAMED_SET, NAMED_SET, |r| {
        Ok(NamedSetRow {
            id: r.int(NAMED_SET_ID),
            relation: r.int(NAMED_SET_RELATION),
            name: r.text(NAMED_SET_NAME).to_owned(),
            dropped: r.optional_int(NAMED_SET_DROPPED),
        })
    })];
}

const SET_MEMBER: Fields = &["set", "ctor"];
pub const SET_MEMBER_SET: Field = field(SET_MEMBER, "set");
pub const SET_MEMBER_CTOR: Field = field(SET_MEMBER, "ctor");

/// A row of `catalog::set_members`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetMemberRow {
    set: i64,
    ctor: i64,
}

impl Relation for relation::SetMembers {
    type Row = SetMemberRow;
}

impl Decode for SetMemberRow {
    type Relation = relation::SetMembers;
    const CTORS: &'static [Ctor<SetMemberRow>] = &[Ctor::new(k::SET_MEMBER, SET_MEMBER, |r| {
        Ok(SetMemberRow {
            set: r.int(SET_MEMBER_SET),
            ctor: r.int(SET_MEMBER_CTOR),
        })
    })];
}

const INDEX: Fields = &["id", "relation", "kind", "clause", "dropped"];
pub const INDEX_ID: Field = field(INDEX, "id");
pub const INDEX_RELATION: Field = field(INDEX, "relation");
pub const INDEX_KIND: Sum = sum(
    field(INDEX, "kind"),
    &[
        (k::INDEX_PRIMARY_KEY, &[]),
        (k::INDEX_UNIQUE, &[]),
        (k::INDEX_PLAIN, &[]),
        (k::INDEX_INTERVAL, &[]),
        (k::INDEX_IMPLIED, &[]),
    ],
);
pub const INDEX_CLAUSE: Field = field(INDEX, "clause");
pub const INDEX_DROPPED: Field = field(INDEX, "dropped");

/// A row of `catalog::indexes`, its kind by the fixed id of its `IndexKind` constructor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexRow {
    id: i64,
    relation: i64,
    kind: CatCtorId,
    clause: String,
    dropped: Option<i64>,
}

impl Relation for relation::Indexes {
    type Row = IndexRow;
}

impl Decode for IndexRow {
    type Relation = relation::Indexes;
    const CTORS: &'static [Ctor<IndexRow>] = &[Ctor::new(k::INDEX, INDEX, |r| {
        Ok(IndexRow {
            id: r.int(INDEX_ID),
            relation: r.int(INDEX_RELATION),
            kind: r.ctor(INDEX_KIND).0,
            clause: r.text(INDEX_CLAUSE).to_owned(),
            dropped: r.optional_int(INDEX_DROPPED),
        })
    })
    .with_sums(&[INDEX_KIND])];
}

const INDEX_CTOR: Fields = &["index", "ctor"];
pub const INDEX_CTOR_INDEX: Field = field(INDEX_CTOR, "index");
pub const INDEX_CTOR_CTOR: Field = field(INDEX_CTOR, "ctor");

/// A row of `catalog::index_ctors`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexCtorRow {
    index: i64,
    ctor: i64,
}

impl Relation for relation::IndexCtors {
    type Row = IndexCtorRow;
}

impl Decode for IndexCtorRow {
    type Relation = relation::IndexCtors;
    const CTORS: &'static [Ctor<IndexCtorRow>] = &[Ctor::new(k::INDEX_CTOR, INDEX_CTOR, |r| {
        Ok(IndexCtorRow {
            index: r.int(INDEX_CTOR_INDEX),
            ctor: r.int(INDEX_CTOR_CTOR),
        })
    })];
}

const CONSTRAINT: Fields = &["id", "module", "name", "text", "dropped"];
pub const CONSTRAINT_ID: Field = field(CONSTRAINT, "id");
pub const CONSTRAINT_MODULE: Field = field(CONSTRAINT, "module");
pub const CONSTRAINT_NAME: Field = field(CONSTRAINT, "name");
pub const CONSTRAINT_TEXT: Field = field(CONSTRAINT, "text");
pub const CONSTRAINT_DROPPED: Field = field(CONSTRAINT, "dropped");

/// A row of `catalog::constraints`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstraintRow {
    id: i64,
    module: String,
    name: String,
    text: String,
    dropped: Option<i64>,
}

impl Relation for relation::Constraints {
    type Row = ConstraintRow;
}

impl Decode for ConstraintRow {
    type Relation = relation::Constraints;
    const CTORS: &'static [Ctor<ConstraintRow>] = &[Ctor::new(k::CONSTRAINT, CONSTRAINT, |r| {
        Ok(ConstraintRow {
            id: r.int(CONSTRAINT_ID),
            module: r.text(CONSTRAINT_MODULE).to_owned(),
            name: r.text(CONSTRAINT_NAME).to_owned(),
            text: r.text(CONSTRAINT_TEXT).to_owned(),
            dropped: r.optional_int(CONSTRAINT_DROPPED),
        })
    })];
}

/// The fields of the header of `catalog::Function`.
#[derive(Clone, Copy)]
pub struct FunctionHead {
    id: Field,
    module: Field,
    name: Field,
    signature: Field,
    deterministic: Field,
    hash: Field,
    dropped: Field,
}

impl FunctionHead {
    pub const fn id(self) -> Field {
        self.id
    }

    pub const fn module(self) -> Field {
        self.module
    }

    pub const fn name(self) -> Field {
        self.name
    }

    pub const fn signature(self) -> Field {
        self.signature
    }

    pub const fn deterministic(self) -> Field {
        self.deterministic
    }

    pub const fn hash(self) -> Field {
        self.hash
    }

    pub const fn dropped(self) -> Field {
        self.dropped
    }
}

const fn function_head(fields: Fields) -> FunctionHead {
    FunctionHead {
        id: field(fields, "id"),
        module: field(fields, "module"),
        name: field(fields, "name"),
        signature: field(fields, "signature"),
        deterministic: field(fields, "deterministic"),
        hash: field(fields, "hash"),
        dropped: field(fields, "dropped"),
    }
}

const LANGUAGE: Fields = &[
    "id",
    "module",
    "name",
    "signature",
    "deterministic",
    "hash",
    "dropped",
    "body",
];
const WASM: Fields = &[
    "id",
    "module",
    "name",
    "signature",
    "deterministic",
    "hash",
    "dropped",
    "code",
    "export",
];
const HOST: Fields = &[
    "id",
    "module",
    "name",
    "signature",
    "deterministic",
    "hash",
    "dropped",
    "symbol",
    "version",
];
const AGGREGATE: Fields = &[
    "id",
    "module",
    "name",
    "signature",
    "deterministic",
    "hash",
    "dropped",
    "body",
];
pub const LANGUAGE_HEAD: FunctionHead = function_head(LANGUAGE);
pub const WASM_HEAD: FunctionHead = function_head(WASM);
pub const HOST_HEAD: FunctionHead = function_head(HOST);
pub const AGGREGATE_HEAD: FunctionHead = function_head(AGGREGATE);
pub const LANGUAGE_BODY: Field = field(LANGUAGE, "body");
pub const WASM_CODE: Field = field(WASM, "code");
pub const WASM_EXPORT: Field = field(WASM, "export");
pub const HOST_SYMBOL: Field = field(HOST, "symbol");
pub const HOST_VERSION: Field = field(HOST, "version");
pub const AGGREGATE_BODY: Field = field(AGGREGATE, "body");

/// The code of a row of `catalog::functions`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FunctionCode {
    Language { body: String },
    Wasm { code: Vec<u8>, export: String },
    Host { symbol: String, version: String },
    Aggregate { body: String },
}

/// A row of `catalog::functions`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionRow {
    id: i64,
    module: String,
    name: String,
    signature: String,
    deterministic: bool,
    hash: Vec<u8>,
    dropped: Option<i64>,
    code: FunctionCode,
}

fn function_row(r: &Row<'_>, head: FunctionHead, code: FunctionCode) -> FunctionRow {
    FunctionRow {
        id: r.int(head.id),
        module: r.text(head.module).to_owned(),
        name: r.text(head.name).to_owned(),
        signature: r.text(head.signature).to_owned(),
        deterministic: r.flag(head.deterministic),
        hash: r.bytes(head.hash).to_vec(),
        dropped: r.optional_int(head.dropped),
        code,
    }
}

impl Relation for relation::Functions {
    type Row = FunctionRow;
}

impl Decode for FunctionRow {
    type Relation = relation::Functions;
    const CTORS: &'static [Ctor<FunctionRow>] = &[
        Ctor::new(k::FUNCTION_LANGUAGE, LANGUAGE, |r| {
            let body = r.text(LANGUAGE_BODY).to_owned();
            Ok(function_row(
                r,
                LANGUAGE_HEAD,
                FunctionCode::Language { body },
            ))
        }),
        Ctor::new(k::FUNCTION_WASM, WASM, |r| {
            let code = FunctionCode::Wasm {
                code: r.bytes(WASM_CODE).to_vec(),
                export: r.text(WASM_EXPORT).to_owned(),
            };
            Ok(function_row(r, WASM_HEAD, code))
        }),
        Ctor::new(k::FUNCTION_HOST, HOST, |r| {
            let code = FunctionCode::Host {
                symbol: r.text(HOST_SYMBOL).to_owned(),
                version: r.text(HOST_VERSION).to_owned(),
            };
            Ok(function_row(r, HOST_HEAD, code))
        }),
        Ctor::new(k::FUNCTION_AGGREGATE, AGGREGATE, |r| {
            let body = r.text(AGGREGATE_BODY).to_owned();
            Ok(function_row(
                r,
                AGGREGATE_HEAD,
                FunctionCode::Aggregate { body },
            ))
        }),
    ];
}

const DEFAULT: Fields = &["id", "relation", "field", "text", "dropped"];
pub const DEFAULT_ID: Field = field(DEFAULT, "id");
pub const DEFAULT_RELATION: Field = field(DEFAULT, "relation");
pub const DEFAULT_FIELD: Field = field(DEFAULT, "field");
pub const DEFAULT_TEXT: Field = field(DEFAULT, "text");
pub const DEFAULT_DROPPED: Field = field(DEFAULT, "dropped");

/// A row of `catalog::defaults`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultRow {
    id: i64,
    relation: i64,
    field: i64,
    text: String,
    dropped: Option<i64>,
}

impl Relation for relation::Defaults {
    type Row = DefaultRow;
}

impl Decode for DefaultRow {
    type Relation = relation::Defaults;
    const CTORS: &'static [Ctor<DefaultRow>] = &[Ctor::new(k::DEFAULT, DEFAULT, |r| {
        Ok(DefaultRow {
            id: r.int(DEFAULT_ID),
            relation: r.int(DEFAULT_RELATION),
            field: r.int(DEFAULT_FIELD),
            text: r.text(DEFAULT_TEXT).to_owned(),
            dropped: r.optional_int(DEFAULT_DROPPED),
        })
    })];
}

const COMPUTED: Fields = &["id", "relation", "name", "field_type", "text", "dropped"];
pub const COMPUTED_ID: Field = field(COMPUTED, "id");
pub const COMPUTED_RELATION: Field = field(COMPUTED, "relation");
pub const COMPUTED_NAME: Field = field(COMPUTED, "name");
pub const COMPUTED_FIELD_TYPE: Field = field(COMPUTED, "field_type");
pub const COMPUTED_TEXT: Field = field(COMPUTED, "text");
pub const COMPUTED_DROPPED: Field = field(COMPUTED, "dropped");

/// A row of `catalog::computed`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComputedRow {
    id: i64,
    relation: i64,
    name: String,
    field_type: String,
    text: String,
    dropped: Option<i64>,
}

impl Relation for relation::Computed {
    type Row = ComputedRow;
}

impl Decode for ComputedRow {
    type Relation = relation::Computed;
    const CTORS: &'static [Ctor<ComputedRow>] = &[Ctor::new(k::COMPUTED, COMPUTED, |r| {
        Ok(ComputedRow {
            id: r.int(COMPUTED_ID),
            relation: r.int(COMPUTED_RELATION),
            name: r.text(COMPUTED_NAME).to_owned(),
            field_type: r.text(COMPUTED_FIELD_TYPE).to_owned(),
            text: r.text(COMPUTED_TEXT).to_owned(),
            dropped: r.optional_int(COMPUTED_DROPPED),
        })
    })];
}
