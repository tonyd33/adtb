//! The catalog relations of the operator log, each read and written by its codec, with the
//! constructors of the sums its operators and clauses hold.

use adtb_lang::ids::engine_ctor as k;

use crate::key::Scalar;

use super::codec::{
    Ctor, Decode, Field, Fields, Kind, Nested, Payload, Relation, Sum, field, nullary, payload,
    relation, sum,
};

#[cfg(test)]
#[path = "operators_tests.rs"]
mod tests;

macro_rules! payloads {
    (
        $list:ident;
        $($ctor:ident = $id:path [$($handle:ident $name:literal: $kind:expr),* $(,)?];)*
    ) => {
        $(
            pub const $ctor: Payload = payload($id, &[$($name),*], &[$($kind),*]);
            $(pub const $handle: Field = field($ctor.fields(), $name);)*
        )*
        const $list: &[Payload] = &[$($ctor),*];
    };
}

const I64: Kind = Kind::Scalar(Scalar::I64);
const TEXT: Kind = Kind::Scalar(Scalar::Text);
const BYTES: Kind = Kind::Scalar(Scalar::Bytes);
const BOOL: Kind = Kind::Sum(&[nullary(k::BOOL_FALSE), nullary(k::BOOL_TRUE)]);
const OPTION_TERM: Kind = Kind::Sum(&[nullary(k::OPTION_NONE), SOME]);
const CTOR_TARGET: Kind = Kind::Sum(TARGETS);
const PLACE_KIND: Kind = Kind::Sum(PLACES);
const ARM_TARGET_KIND: Kind = Kind::Sum(ARMS);
const INDEX_KIND: Kind = Kind::Sum(&[
    nullary(k::INDEX_PRIMARY_KEY),
    nullary(k::INDEX_UNIQUE),
    nullary(k::INDEX_PLAIN),
    nullary(k::INDEX_INTERVAL),
    nullary(k::INDEX_IMPLIED),
]);

const SOME: Payload = payload(k::OPTION_SOME, &["value"], &[TEXT]);
const SOME_VALUE: Field = field(SOME.fields(), "value");

/// Returns `b` as a value of the prelude's `Bool`.
pub fn flag(b: bool) -> Nested {
    let id = if b { k::BOOL_TRUE } else { k::BOOL_FALSE };
    Nested::of(nullary(id), [])
}

/// Returns `t` as a value of `Option Term`.
pub fn optional(t: Option<&str>) -> Nested {
    match t {
        Some(t) => Nested::of(SOME, [(SOME_VALUE, Nested::text(t))]),
        None => Nested::of(nullary(k::OPTION_NONE), []),
    }
}

payloads! {
    TARGETS;
    IN_RELATION = k::TARGET_IN_RELATION [
        IN_RELATION_RELATION "relation": I64,
        IN_RELATION_CTOR "ctor": I64,
    ];
    OF_TYPE = k::TARGET_OF_TYPE [OF_TYPE_CTOR "ctor": I64];
}
payloads! {
    PLACES;
    LAST = k::PLACE_LAST [];
    BEFORE = k::PLACE_BEFORE [BEFORE_CTOR "ctor": I64];
    AFTER = k::PLACE_AFTER [AFTER_CTOR "ctor": I64];
}
payloads! {
    OPERATORS;
    ADD_MODULE = k::OP_ADD_MODULE [ADD_MODULE_MODULE "module": TEXT];
    DROP_MODULE = k::OP_DROP_MODULE [DROP_MODULE_MODULE "module": TEXT];
    INSTALL = k::OP_INSTALL [
        INSTALL_MODULE "module": TEXT,
        INSTALL_VERSION "version": TEXT,
        INSTALL_PACKAGE "package": BYTES,
    ];
    UNINSTALL = k::OP_UNINSTALL [UNINSTALL_MODULE "module": TEXT];
    ADD_TYPE = k::OP_ADD_TYPE [ADD_TYPE_SUM "sum": I64, ADD_TYPE_DECL "decl": TEXT];
    DROP_TYPE = k::OP_DROP_TYPE [DROP_TYPE_SUM "sum": I64];
    ADD_RELATION = k::OP_ADD_RELATION [
        ADD_RELATION_RELATION "relation": I64,
        ADD_RELATION_DECL "decl": TEXT,
    ];
    DROP_RELATION = k::OP_DROP_RELATION [DROP_RELATION_RELATION "relation": I64];
    ADD_SET = k::OP_ADD_SET [
        ADD_SET_SET "set": I64,
        ADD_SET_RELATION "relation": I64,
        ADD_SET_NAME "name": TEXT,
    ];
    DROP_SET = k::OP_DROP_SET [DROP_SET_SET "set": I64];
    ADD_CONSTRUCTOR = k::OP_ADD_CONSTRUCTOR [
        ADD_CONSTRUCTOR_CTOR "ctor": I64,
        ADD_CONSTRUCTOR_SUM "sum": I64,
        ADD_CONSTRUCTOR_DECL "decl": TEXT,
        ADD_CONSTRUCTOR_PLACE "place": PLACE_KIND,
    ];
    REMOVE_CONSTRUCTOR = k::OP_REMOVE_CONSTRUCTOR [REMOVE_CONSTRUCTOR_CTOR "ctor": I64];
    ENABLE = k::OP_ENABLE [ENABLE_TARGET "target": CTOR_TARGET];
    DISABLE = k::OP_DISABLE [DISABLE_TARGET "target": CTOR_TARGET];
    DEPRECATE = k::OP_DEPRECATE [DEPRECATE_TARGET "target": CTOR_TARGET];
    RENAME_CONSTRUCTOR = k::OP_RENAME_CONSTRUCTOR [
        RENAME_CONSTRUCTOR_CTOR "ctor": I64,
        RENAME_CONSTRUCTOR_NAME "name": TEXT,
    ];
    RENAME_TYPE = k::OP_RENAME_TYPE [RENAME_TYPE_SUM "sum": I64, RENAME_TYPE_NAME "name": TEXT];
    RENAME_RELATION = k::OP_RENAME_RELATION [
        RENAME_RELATION_RELATION "relation": I64,
        RENAME_RELATION_NAME "name": TEXT,
    ];
    RENAME_SET = k::OP_RENAME_SET [RENAME_SET_SET "set": I64, RENAME_SET_NAME "name": TEXT];
    RENAME_FIELD = k::OP_RENAME_FIELD [
        RENAME_FIELD_FIELD "field": I64,
        RENAME_FIELD_NAME "name": TEXT,
    ];
    SPLIT = k::OP_SPLIT [
        SPLIT_CTOR "ctor": I64,
        SPLIT_DECL "decl": TEXT,
        SPLIT_ROUTING "routing": TEXT,
    ];
    MERGE = k::OP_MERGE [
        MERGE_CTOR "ctor": I64,
        MERGE_DECL "decl": TEXT,
        MERGE_ROUTING "routing": OPTION_TERM,
    ];
    NEST = k::OP_NEST [NEST_CTOR "ctor": I64, NEST_DECL "decl": TEXT];
    UNNEST = k::OP_UNNEST [UNNEST_CTOR "ctor": I64];
    ADD_FIELD = k::OP_ADD_FIELD [
        ADD_FIELD_FIELD "field": I64,
        ADD_FIELD_DECL "decl": TEXT,
        ADD_FIELD_FILL "fill": TEXT,
    ];
    DROP_FIELD = k::OP_DROP_FIELD [DROP_FIELD_FIELD "field": I64];
    ALTER_FIELD = k::OP_ALTER_FIELD [
        ALTER_FIELD_FIELD "field": I64,
        ALTER_FIELD_ACTION "action": TEXT,
    ];
    MOVE_FIELD = k::OP_MOVE_FIELD [
        MOVE_FIELD_FIELD "field": I64,
        MOVE_FIELD_HEADER "header": BOOL,
        MOVE_FIELD_FILL "fill": OPTION_TERM,
    ];
    ADD_UNIQUE = k::OP_ADD_UNIQUE [
        ADD_UNIQUE_INDEX "index": I64,
        ADD_UNIQUE_RELATION "relation": I64,
        ADD_UNIQUE_FIELDS "fields": TEXT,
    ];
    DROP_UNIQUE = k::OP_DROP_UNIQUE [DROP_UNIQUE_INDEX "index": I64];
    REKEY = k::OP_REKEY [
        REKEY_RELATION "relation": I64,
        REKEY_KEY "key": I64,
        REKEY_FIELDS "fields": TEXT,
    ];
    ADD_INDEX = k::OP_ADD_INDEX [
        ADD_INDEX_INDEX "index": I64,
        ADD_INDEX_RELATION "relation": I64,
        ADD_INDEX_INTERVAL "interval": BOOL,
        ADD_INDEX_FIELDS "fields": TEXT,
    ];
    DROP_INDEX = k::OP_DROP_INDEX [DROP_INDEX_INDEX "index": I64];
    ADD_DEFAULT = k::OP_ADD_DEFAULT [
        ADD_DEFAULT_DEFAULT "default": I64,
        ADD_DEFAULT_RELATION "relation": I64,
        ADD_DEFAULT_FIELD "field": I64,
        ADD_DEFAULT_VALUE "value": TEXT,
    ];
    DROP_DEFAULT = k::OP_DROP_DEFAULT [DROP_DEFAULT_DEFAULT "default": I64];
    DEFINE_FN = k::OP_DEFINE_FN [DEFINE_FN_FUNCTION "function": I64, DEFINE_FN_DECL "decl": TEXT];
    REDEFINE_FN = k::OP_REDEFINE_FN [
        REDEFINE_FN_FUNCTION "function": I64,
        REDEFINE_FN_DECL "decl": TEXT,
    ];
    DROP_FN = k::OP_DROP_FN [DROP_FN_FUNCTION "function": I64];
    DEFINE_AGGREGATE = k::OP_DEFINE_AGGREGATE [
        DEFINE_AGGREGATE_FUNCTION "function": I64,
        DEFINE_AGGREGATE_DECL "decl": TEXT,
    ];
    DROP_AGGREGATE = k::OP_DROP_AGGREGATE [DROP_AGGREGATE_FUNCTION "function": I64];
    ADD_CONSTRAINT = k::OP_ADD_CONSTRAINT [
        ADD_CONSTRAINT_CONSTRAINT "constraint": I64,
        ADD_CONSTRAINT_BODY "body": TEXT,
    ];
    DROP_CONSTRAINT = k::OP_DROP_CONSTRAINT [DROP_CONSTRAINT_CONSTRAINT "constraint": I64];
    ADD_CHECK = k::OP_ADD_CHECK [
        ADD_CHECK_CONSTRAINT "constraint": I64,
        ADD_CHECK_BODY "body": TEXT,
    ];
    DROP_CHECK = k::OP_DROP_CHECK [DROP_CHECK_CONSTRAINT "constraint": I64];
    STORE = k::OP_STORE [
        STORE_VIEW "view": I64,
        STORE_FUNCTION "function": I64,
        STORE_DECL "decl": TEXT,
    ];
    UNSTORE = k::OP_UNSTORE [UNSTORE_VIEW "view": I64];
    ADD_COMPUTED = k::OP_ADD_COMPUTED [
        ADD_COMPUTED_COMPUTED "computed": I64,
        ADD_COMPUTED_RELATION "relation": I64,
        ADD_COMPUTED_NAME "name": TEXT,
        ADD_COMPUTED_DECL "decl": TEXT,
    ];
    REDEFINE_COMPUTED = k::OP_REDEFINE_COMPUTED [
        REDEFINE_COMPUTED_COMPUTED "computed": I64,
        REDEFINE_COMPUTED_DECL "decl": TEXT,
    ];
    DROP_COMPUTED = k::OP_DROP_COMPUTED [DROP_COMPUTED_COMPUTED "computed": I64];
    GRANT = k::OP_GRANT [
        GRANT_PRIVILEGE "privilege": TEXT,
        GRANT_TARGET "target": OPTION_TERM,
        GRANT_GRANTEE "grantee": TEXT,
        GRANT_GRANT_OPTION "grant_option": BOOL,
    ];
    REVOKE = k::OP_REVOKE [
        REVOKE_PRIVILEGE "privilege": TEXT,
        REVOKE_TARGET "target": OPTION_TERM,
        REVOKE_GRANTEE "grantee": TEXT,
    ];
    ADOPT_TZDB = k::OP_ADOPT_TZDB [
        ADOPT_TZDB_RELEASE "release": TEXT,
        ADOPT_TZDB_DATA "data": BYTES,
    ];
    SCRIPT = k::OP_SCRIPT [SCRIPT_WRITE "write": TEXT];
    REVERT = k::OP_REVERT [REVERT_MIGRATION "migration": I64];
}
payloads! {
    ARMS;
    OF_FN = k::ARM_OF_FN [OF_FN_FUNCTION "function": I64];
    OF_CONSTRAINT = k::ARM_OF_CONSTRAINT [OF_CONSTRAINT_CONSTRAINT "constraint": I64];
    OF_COMPUTED = k::ARM_OF_COMPUTED [OF_COMPUTED_COMPUTED "computed": I64];
}
payloads! {
    CLAUSES;
    JOINS = k::CLAUSE_JOINS [JOINS_SET "set": I64];
    MEMBER = k::CLAUSE_MEMBER [MEMBER_CTOR "ctor": I64];
    ARM = k::CLAUSE_ARM [ARM_TARGET "target": ARM_TARGET_KIND, ARM_ARM "arm": TEXT];
    REPLACEMENT = k::CLAUSE_REPLACEMENT [
        REPLACEMENT_INDEX "index": I64,
        REPLACEMENT_KIND "kind": INDEX_KIND,
        REPLACEMENT_FIELDS "fields": TEXT,
    ];
    CARRY = k::CLAUSE_CARRY [CARRY_FIELD "field": I64];
    FILL = k::CLAUSE_FILL [FILL_FIELD "field": I64, FILL_FILL "fill": TEXT];
    PRIVILEGE = k::CLAUSE_PRIVILEGE [
        PRIVILEGE_PRIVILEGE "privilege": TEXT,
        PRIVILEGE_TARGET "target": OPTION_TERM,
    ];
}

/// A row of `catalog::migration_ops`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationOpRow {
    migration: i64,
    ordinal: i64,
    op: Nested,
}

const MIGRATION_OP: Fields = &["migration", "ordinal", "op"];
pub const MIGRATION_OP_MIGRATION: Field = field(MIGRATION_OP, "migration");
pub const MIGRATION_OP_ORDINAL: Field = field(MIGRATION_OP, "ordinal");
pub const MIGRATION_OP_OP: Sum = sum(field(MIGRATION_OP, "op"), OPERATORS);

impl Relation for relation::MigrationOps {
    type Row = MigrationOpRow;
}

impl Decode for MigrationOpRow {
    type Relation = relation::MigrationOps;
    const CTORS: &'static [Ctor<MigrationOpRow>] =
        &[Ctor::new(k::MIGRATION_OP, MIGRATION_OP, |r| {
            Ok(MigrationOpRow {
                migration: r.int(MIGRATION_OP_MIGRATION),
                ordinal: r.int32(MIGRATION_OP_ORDINAL),
                op: r.nested(MIGRATION_OP_OP),
            })
        })
        .with_sums(&[MIGRATION_OP_OP])];
}

/// A row of `catalog::migration_clauses`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpClauseRow {
    migration: i64,
    ordinal: i64,
    clause: i64,
    body: Nested,
}

const OP_CLAUSE: Fields = &["migration", "ordinal", "clause", "body"];
pub const OP_CLAUSE_MIGRATION: Field = field(OP_CLAUSE, "migration");
pub const OP_CLAUSE_ORDINAL: Field = field(OP_CLAUSE, "ordinal");
pub const OP_CLAUSE_CLAUSE: Field = field(OP_CLAUSE, "clause");
pub const OP_CLAUSE_BODY: Sum = sum(field(OP_CLAUSE, "body"), CLAUSES);

impl Relation for relation::MigrationClauses {
    type Row = OpClauseRow;
}

impl Decode for OpClauseRow {
    type Relation = relation::MigrationClauses;
    const CTORS: &'static [Ctor<OpClauseRow>] = &[Ctor::new(k::OP_CLAUSE, OP_CLAUSE, |r| {
        Ok(OpClauseRow {
            migration: r.int(OP_CLAUSE_MIGRATION),
            ordinal: r.int32(OP_CLAUSE_ORDINAL),
            clause: r.int32(OP_CLAUSE_CLAUSE),
            body: r.nested(OP_CLAUSE_BODY),
        })
    })
    .with_sums(&[OP_CLAUSE_BODY])];
}
