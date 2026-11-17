use adtb_lang::ids::{CatCtorId, engine_ctor as k};
use adtb_lang::schema::Schema;

use crate::key::Value;

use super::super::codec::testing::{Sample, Sampled, by_name, field_ty, position};
use super::super::codec::{Codec, Decode, Nested, nullary};
use super::*;

fn id(n: u64) -> Nested {
    Nested::int(n)
}

/// Returns the stored value of `v` in field `field` of the one constructor `of` of `T`'s
/// relation in `schema`.
fn stored<T: Decode>(schema: &Schema, of: CatCtorId, field: &str, v: &Nested) -> Value {
    let codec = Codec::<T>::new(schema);
    let ty = field_ty(
        schema,
        codec.relation(),
        position(schema, &codec, of),
        field,
    );
    by_name(schema, &ty, v)
}

impl Sampled for MigrationOpRow {
    fn samples(schema: &Schema) -> Vec<Sample<MigrationOpRow>> {
        let ops = [
            Nested::of(
                ADD_CONSTRUCTOR,
                [
                    (ADD_CONSTRUCTOR_CTOR, id(21)),
                    (ADD_CONSTRUCTOR_SUM, id(11)),
                    (ADD_CONSTRUCTOR_DECL, Nested::text("Circle { r: I64 }")),
                    (
                        ADD_CONSTRUCTOR_PLACE,
                        Nested::of(BEFORE, [(BEFORE_CTOR, id(22))]),
                    ),
                ],
            ),
            Nested::of(
                ENABLE,
                [(
                    ENABLE_TARGET,
                    Nested::of(
                        IN_RELATION,
                        [(IN_RELATION_RELATION, id(31)), (IN_RELATION_CTOR, id(21))],
                    ),
                )],
            ),
            Nested::of(
                MERGE,
                [
                    (MERGE_CTOR, id(23)),
                    (MERGE_DECL, Nested::text("M { x: Text }")),
                    (MERGE_ROUTING, optional(None)),
                ],
            ),
            Nested::of(
                GRANT,
                [
                    (GRANT_PRIVILEGE, Nested::text("read")),
                    (GRANT_TARGET, optional(Some("shop::shapes"))),
                    (GRANT_GRANTEE, Nested::text("web")),
                    (GRANT_GRANT_OPTION, flag(true)),
                ],
            ),
            Nested::of(
                INSTALL,
                [
                    (INSTALL_MODULE, Nested::text("geo")),
                    (INSTALL_VERSION, Nested::text("1.2")),
                    (INSTALL_PACKAGE, Nested::Scalar(Value::Bytes(vec![5; 32]))),
                ],
            ),
        ];
        ops.into_iter()
            .zip(1..)
            .map(|(op, ordinal)| {
                let fields = vec![
                    ("migration", Value::I64(3)),
                    ("ordinal", Value::I32(ordinal)),
                    (
                        "op",
                        stored::<MigrationOpRow>(schema, k::MIGRATION_OP, "op", &op),
                    ),
                ];
                let want = MigrationOpRow {
                    migration: 3,
                    ordinal: i64::from(ordinal),
                    op,
                };
                (k::MIGRATION_OP, fields, want)
            })
            .collect()
    }
}

impl Sampled for OpClauseRow {
    fn samples(schema: &Schema) -> Vec<Sample<OpClauseRow>> {
        let bodies = [
            Nested::of(
                ARM,
                [
                    (
                        ARM_TARGET,
                        Nested::of(OF_COMPUTED, [(OF_COMPUTED_COMPUTED, id(41))]),
                    ),
                    (ARM_ARM, Nested::text("E e -> e.w")),
                ],
            ),
            Nested::of(
                REPLACEMENT,
                [
                    (REPLACEMENT_INDEX, id(61)),
                    (REPLACEMENT_KIND, Nested::of(nullary(k::INDEX_INTERVAL), [])),
                    (REPLACEMENT_FIELDS, Nested::text("(x)")),
                ],
            ),
            Nested::of(
                PRIVILEGE,
                [
                    (PRIVILEGE_PRIVILEGE, Nested::text("call")),
                    (PRIVILEGE_TARGET, optional(Some("shop::f"))),
                ],
            ),
        ];
        bodies
            .into_iter()
            .zip(1..)
            .map(|(body, clause)| {
                let fields = vec![
                    ("migration", Value::I64(3)),
                    ("ordinal", Value::I32(2)),
                    ("clause", Value::I32(clause)),
                    (
                        "body",
                        stored::<OpClauseRow>(schema, k::OP_CLAUSE, "body", &body),
                    ),
                ];
                let want = OpClauseRow {
                    migration: 3,
                    ordinal: 2,
                    clause: i64::from(clause),
                    body,
                };
                (k::OP_CLAUSE, fields, want)
            })
            .collect()
    }
}
