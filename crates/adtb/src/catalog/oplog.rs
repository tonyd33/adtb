use std::sync::{Arc, OnceLock};

use adtb_lang::evolve::{
    ArmTarget, Clause, IndexKind, Place, Replaying, Resolved, ResolvedOp, Source, Target,
};
use adtb_lang::ids::engine_ctor as k;
use adtb_lang::schema::Schema;

use crate::heap::{FieldType, put_uvarint};
use crate::key::Value;

use super::Error;
use super::codec::{Codec, Field, Nested, Payload, nullary};
use super::operators::{
    self as o, MIGRATION_OP_OP, MigrationOpRow, OP_CLAUSE_BODY, OpClauseRow, flag, optional,
};

/// A BLAKE3-256 hash.
pub type Hash = [u8; 32];

/// The hash of one version, made at most once and shared by every clone.
#[derive(Clone, Debug, Default)]
pub struct VersionHash(Arc<OnceLock<Hash>>);

impl VersionHash {
    /// Returns the hash of the version `source` is at, as [`version_hash`] gives it, made on the
    /// first call that succeeds.
    ///
    /// Preconditions:
    /// - `schema` and `source` are those of the version this hash is of.
    pub fn get(&self, schema: &Schema, source: &Source) -> Result<Hash, Error> {
        if let Some(h) = self.0.get() {
            return Ok(*h);
        }
        let h = version_hash(schema, source).map_err(super::unresolved)?;
        Ok(*self.0.get_or_init(|| h))
    }

    /// Hold `hash` as the version's when no hash is held yet.
    pub(super) fn made(&self, hash: Hash) {
        self.0.get_or_init(|| hash);
    }
}

/// A resolved operator as a value of `catalog::Operator`, with its clauses as values of
/// `catalog::ClauseBody` in clause order.
pub type OperatorRow = (Value, Vec<Value>);

/// Returns the hash of the version `source` is at: `h(0)` is 32 zero bytes, and `h(n)` the hash
/// of `h(n - 1)` followed by the encoding of migration `n`'s resolved operators.
///
/// Fail with the diagnostics of the first migration of the log that does not resolve, or the
/// migration model's failure on it.
pub fn version_hash(schema: &Schema, source: &Source) -> Replaying<Hash> {
    chain(schema, source, |_, _| {})
}

/// Returns the hash [`version_hash`] returns, giving `each` the operators of each migration
/// of the log in order with the hash of the version it leaves.
pub fn chain(
    schema: &Schema,
    source: &Source,
    mut each: impl FnMut(&[OperatorRow], Hash),
) -> Replaying<Hash> {
    let log = Log::new(schema);
    let mut h = [0; 32];
    for l in &source.log {
        let ops = log.rows(&source.resolved(l.number)?);
        h = log.chained(h, &ops);
        each(&ops, h);
    }
    Ok(h)
}

/// Returns the hash of the version after one whose hash is `previous`, migrated by `ops`.
#[cfg(any(test, feature = "testing"))]
pub fn chained(schema: &Schema, previous: Hash, ops: &[ResolvedOp]) -> Hash {
    let log = Log::new(schema);
    log.chained(previous, &log.rows(ops))
}

/// Returns the resolved operators of migration `number` of `source` in ordinal order.
///
/// Panics:
/// - The migration does not resolve.
#[cfg(test)]
pub fn operators(log: &Log, source: &Source, number: u64) -> Vec<OperatorRow> {
    let ops = source
        .resolved(number)
        .unwrap_or_else(|ds| panic!("logged migration {number} resolves: {ds:?}"));
    log.rows(&ops)
}

/// The codecs of the rows of `migration_ops` and `migration_clauses`.
pub struct Log {
    ops: Codec<MigrationOpRow>,
    clauses: Codec<OpClauseRow>,
}

impl Log {
    pub fn new(schema: &Schema) -> Log {
        Log {
            ops: Codec::new(schema),
            clauses: Codec::new(schema),
        }
    }

    pub fn ops(&self) -> &Codec<MigrationOpRow> {
        &self.ops
    }

    pub fn clauses(&self) -> &Codec<OpClauseRow> {
        &self.clauses
    }

    /// Returns `ops` as stored values of `catalog::Operator` with their clauses.
    pub fn rows(&self, ops: &[ResolvedOp]) -> Vec<OperatorRow> {
        ops.iter()
            .map(|op| {
                let clauses = op
                    .clauses
                    .iter()
                    .map(|c| self.clauses.stored(OP_CLAUSE_BODY, &clause(c)));
                (
                    self.ops.stored(MIGRATION_OP_OP, &operator(&op.op)),
                    clauses.collect(),
                )
            })
            .collect()
    }

    /// Returns the hash of `previous` followed by the encoding of `ops`.
    pub fn chained(&self, previous: Hash, ops: &[OperatorRow]) -> Hash {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&previous);
        hasher.update(&self.encoded(ops));
        *hasher.finalize().as_bytes()
    }

    /// Returns the encoding of a migration's operators: their count, then each operator, the
    /// count of its clauses and each clause, in the wire value encoding.
    pub fn encoded(&self, ops: &[OperatorRow]) -> Vec<u8> {
        let op_ty = self.ops.field_type(MIGRATION_OP_OP.field());
        let clause_ty = self.clauses.field_type(OP_CLAUSE_BODY.field());
        let mut out = Vec::new();
        put_uvarint(ops.len() as u64, &mut out);
        for (op, clauses) in ops {
            encode(op_ty, op, &mut out);
            put_uvarint(clauses.len() as u64, &mut out);
            for c in clauses {
                encode(clause_ty, c, &mut out);
            }
        }
        out
    }
}

fn id(n: u64) -> Nested {
    Nested::int(n)
}

fn text(s: &str) -> Nested {
    Nested::text(s)
}

fn one(p: Payload, f: Field, object: u64) -> Nested {
    Nested::of(p, [(f, id(object))])
}

fn two(p: Payload, (f, object): (Field, u64), (g, t): (Field, &str)) -> Nested {
    Nested::of(p, [(f, id(object)), (g, text(t))])
}

fn operator(op: &Resolved) -> Nested {
    match op {
        Resolved::AddModule { module } => {
            Nested::of(o::ADD_MODULE, [(o::ADD_MODULE_MODULE, text(module))])
        }
        Resolved::DropModule { module } => {
            Nested::of(o::DROP_MODULE, [(o::DROP_MODULE_MODULE, text(module))])
        }
        Resolved::AddType { sum, decl } => two(
            o::ADD_TYPE,
            (o::ADD_TYPE_SUM, *sum),
            (o::ADD_TYPE_DECL, decl),
        ),
        Resolved::DropType { sum } => one(o::DROP_TYPE, o::DROP_TYPE_SUM, *sum),
        Resolved::RenameType { sum, name } => two(
            o::RENAME_TYPE,
            (o::RENAME_TYPE_SUM, *sum),
            (o::RENAME_TYPE_NAME, name),
        ),
        Resolved::AddRelation { relation: r, decl } => two(
            o::ADD_RELATION,
            (o::ADD_RELATION_RELATION, *r),
            (o::ADD_RELATION_DECL, decl),
        ),
        Resolved::DropRelation { relation: r } => {
            one(o::DROP_RELATION, o::DROP_RELATION_RELATION, *r)
        }
        Resolved::RenameRelation { relation: r, name } => two(
            o::RENAME_RELATION,
            (o::RENAME_RELATION_RELATION, *r),
            (o::RENAME_RELATION_NAME, name),
        ),
        Resolved::AddSet {
            set,
            relation,
            name,
        } => Nested::of(
            o::ADD_SET,
            [
                (o::ADD_SET_SET, id(*set)),
                (o::ADD_SET_RELATION, id(*relation)),
                (o::ADD_SET_NAME, text(name)),
            ],
        ),
        Resolved::DropSet { set } => one(o::DROP_SET, o::DROP_SET_SET, *set),
        Resolved::RenameSet { set, name } => two(
            o::RENAME_SET,
            (o::RENAME_SET_SET, *set),
            (o::RENAME_SET_NAME, name),
        ),
        Resolved::AddUnique {
            index,
            relation,
            fields,
        } => Nested::of(
            o::ADD_UNIQUE,
            [
                (o::ADD_UNIQUE_INDEX, id(*index)),
                (o::ADD_UNIQUE_RELATION, id(*relation)),
                (o::ADD_UNIQUE_FIELDS, text(fields)),
            ],
        ),
        Resolved::AddIndex {
            index,
            relation,
            interval,
            fields,
        } => Nested::of(
            o::ADD_INDEX,
            [
                (o::ADD_INDEX_INDEX, id(*index)),
                (o::ADD_INDEX_RELATION, id(*relation)),
                (o::ADD_INDEX_INTERVAL, flag(*interval)),
                (o::ADD_INDEX_FIELDS, text(fields)),
            ],
        ),
        Resolved::DropUnique { index } => one(o::DROP_UNIQUE, o::DROP_UNIQUE_INDEX, *index),
        Resolved::DropIndex { index } => one(o::DROP_INDEX, o::DROP_INDEX_INDEX, *index),
        Resolved::Rekey {
            relation,
            key,
            fields,
        } => Nested::of(
            o::REKEY,
            [
                (o::REKEY_RELATION, id(*relation)),
                (o::REKEY_KEY, id(*key)),
                (o::REKEY_FIELDS, text(fields)),
            ],
        ),
        Resolved::DefineFn { function: f, decl } => two(
            o::DEFINE_FN,
            (o::DEFINE_FN_FUNCTION, *f),
            (o::DEFINE_FN_DECL, decl),
        ),
        Resolved::RedefineFn { function: f, decl } => two(
            o::REDEFINE_FN,
            (o::REDEFINE_FN_FUNCTION, *f),
            (o::REDEFINE_FN_DECL, decl),
        ),
        Resolved::DropFn { function: f } => one(o::DROP_FN, o::DROP_FN_FUNCTION, *f),
        Resolved::DefineAggregate { function: f, decl } => two(
            o::DEFINE_AGGREGATE,
            (o::DEFINE_AGGREGATE_FUNCTION, *f),
            (o::DEFINE_AGGREGATE_DECL, decl),
        ),
        Resolved::DropAggregate { function: f } => {
            one(o::DROP_AGGREGATE, o::DROP_AGGREGATE_FUNCTION, *f)
        }
        Resolved::AddConstraint {
            constraint: c,
            body,
        } => two(
            o::ADD_CONSTRAINT,
            (o::ADD_CONSTRAINT_CONSTRAINT, *c),
            (o::ADD_CONSTRAINT_BODY, body),
        ),
        Resolved::DropConstraint { constraint: c } => {
            one(o::DROP_CONSTRAINT, o::DROP_CONSTRAINT_CONSTRAINT, *c)
        }
        Resolved::AddCheck {
            constraint: c,
            body,
        } => two(
            o::ADD_CHECK,
            (o::ADD_CHECK_CONSTRAINT, *c),
            (o::ADD_CHECK_BODY, body),
        ),
        Resolved::DropCheck { constraint: c } => one(o::DROP_CHECK, o::DROP_CHECK_CONSTRAINT, *c),
        other => reshaped(other),
    }
}

/// Returns the operator on constructors or fields `op` is, or what [`derived`] returns.
fn reshaped(op: &Resolved) -> Nested {
    match op {
        Resolved::AddConstructor {
            ctor,
            sum: of,
            decl,
            place,
        } => Nested::of(
            o::ADD_CONSTRUCTOR,
            [
                (o::ADD_CONSTRUCTOR_CTOR, id(*ctor)),
                (o::ADD_CONSTRUCTOR_SUM, id(*of)),
                (o::ADD_CONSTRUCTOR_DECL, text(decl)),
                (o::ADD_CONSTRUCTOR_PLACE, placed(place)),
            ],
        ),
        Resolved::RemoveConstructor { ctor } => {
            one(o::REMOVE_CONSTRUCTOR, o::REMOVE_CONSTRUCTOR_CTOR, *ctor)
        }
        Resolved::RenameConstructor { ctor, name } => two(
            o::RENAME_CONSTRUCTOR,
            (o::RENAME_CONSTRUCTOR_CTOR, *ctor),
            (o::RENAME_CONSTRUCTOR_NAME, name),
        ),
        Resolved::Enable { target: t } => Nested::of(o::ENABLE, [(o::ENABLE_TARGET, target(t))]),
        Resolved::Disable { target: t } => Nested::of(o::DISABLE, [(o::DISABLE_TARGET, target(t))]),
        Resolved::Deprecate { target: t } => {
            Nested::of(o::DEPRECATE, [(o::DEPRECATE_TARGET, target(t))])
        }
        Resolved::Split {
            ctor,
            decl,
            routing,
        } => Nested::of(
            o::SPLIT,
            [
                (o::SPLIT_CTOR, id(*ctor)),
                (o::SPLIT_DECL, text(decl)),
                (o::SPLIT_ROUTING, text(routing)),
            ],
        ),
        Resolved::Merge {
            ctor,
            decl,
            routing,
        } => Nested::of(
            o::MERGE,
            [
                (o::MERGE_CTOR, id(*ctor)),
                (o::MERGE_DECL, text(decl)),
                (o::MERGE_ROUTING, optional(routing.as_deref())),
            ],
        ),
        Resolved::Nest { ctor, decl } => two(o::NEST, (o::NEST_CTOR, *ctor), (o::NEST_DECL, decl)),
        Resolved::Unnest { ctor } => one(o::UNNEST, o::UNNEST_CTOR, *ctor),
        Resolved::AddField { field, decl, fill } => Nested::of(
            o::ADD_FIELD,
            [
                (o::ADD_FIELD_FIELD, id(*field)),
                (o::ADD_FIELD_DECL, text(decl)),
                (o::ADD_FIELD_FILL, text(fill)),
            ],
        ),
        Resolved::DropField { field } => one(o::DROP_FIELD, o::DROP_FIELD_FIELD, *field),
        Resolved::AlterField { field, action } => two(
            o::ALTER_FIELD,
            (o::ALTER_FIELD_FIELD, *field),
            (o::ALTER_FIELD_ACTION, action),
        ),
        Resolved::RenameField { field, name } => two(
            o::RENAME_FIELD,
            (o::RENAME_FIELD_FIELD, *field),
            (o::RENAME_FIELD_NAME, name),
        ),
        Resolved::MoveField {
            field,
            header,
            fill,
        } => Nested::of(
            o::MOVE_FIELD,
            [
                (o::MOVE_FIELD_FIELD, id(*field)),
                (o::MOVE_FIELD_HEADER, flag(*header)),
                (o::MOVE_FIELD_FILL, optional(fill.as_deref())),
            ],
        ),
        other => derived(other),
    }
}

/// Returns the operator of a revert, or of an operator on defaults, computed fields, stored
/// views, grants, extensions, zone rules or data.
fn derived(op: &Resolved) -> Nested {
    match op {
        Resolved::AddDefault {
            default,
            relation,
            field,
            value,
        } => Nested::of(
            o::ADD_DEFAULT,
            [
                (o::ADD_DEFAULT_DEFAULT, id(*default)),
                (o::ADD_DEFAULT_RELATION, id(*relation)),
                (o::ADD_DEFAULT_FIELD, id(*field)),
                (o::ADD_DEFAULT_VALUE, text(value)),
            ],
        ),
        Resolved::Revert { migration } => one(o::REVERT, o::REVERT_MIGRATION, *migration),
        Resolved::DropDefault { default } => {
            one(o::DROP_DEFAULT, o::DROP_DEFAULT_DEFAULT, *default)
        }
        Resolved::Store {
            view,
            function,
            decl,
        } => Nested::of(
            o::STORE,
            [
                (o::STORE_VIEW, id(*view)),
                (o::STORE_FUNCTION, id(*function)),
                (o::STORE_DECL, text(decl)),
            ],
        ),
        Resolved::Unstore { view } => one(o::UNSTORE, o::UNSTORE_VIEW, *view),
        Resolved::AddComputed {
            computed,
            relation,
            name,
            decl,
        } => Nested::of(
            o::ADD_COMPUTED,
            [
                (o::ADD_COMPUTED_COMPUTED, id(*computed)),
                (o::ADD_COMPUTED_RELATION, id(*relation)),
                (o::ADD_COMPUTED_NAME, text(name)),
                (o::ADD_COMPUTED_DECL, text(decl)),
            ],
        ),
        Resolved::RedefineComputed { computed, decl } => two(
            o::REDEFINE_COMPUTED,
            (o::REDEFINE_COMPUTED_COMPUTED, *computed),
            (o::REDEFINE_COMPUTED_DECL, decl),
        ),
        Resolved::DropComputed { computed } => {
            one(o::DROP_COMPUTED, o::DROP_COMPUTED_COMPUTED, *computed)
        }
        Resolved::Grant {
            privilege,
            target,
            grantee,
            grant_option,
        } => Nested::of(
            o::GRANT,
            [
                (o::GRANT_PRIVILEGE, text(privilege)),
                (o::GRANT_TARGET, optional(Some(target))),
                (o::GRANT_GRANTEE, text(grantee)),
                (o::GRANT_GRANT_OPTION, flag(*grant_option)),
            ],
        ),
        Resolved::Revoke {
            privilege,
            target,
            grantee,
        } => Nested::of(
            o::REVOKE,
            [
                (o::REVOKE_PRIVILEGE, text(privilege)),
                (o::REVOKE_TARGET, optional(Some(target))),
                (o::REVOKE_GRANTEE, text(grantee)),
            ],
        ),
        Resolved::Install {
            module,
            version,
            package,
        } => Nested::of(
            o::INSTALL,
            [
                (o::INSTALL_MODULE, text(module)),
                (o::INSTALL_VERSION, text(version)),
                (o::INSTALL_PACKAGE, hashed(package)),
            ],
        ),
        Resolved::Uninstall { module } => {
            Nested::of(o::UNINSTALL, [(o::UNINSTALL_MODULE, text(module))])
        }
        Resolved::AdoptTzdb { release, data } => Nested::of(
            o::ADOPT_TZDB,
            [
                (o::ADOPT_TZDB_RELEASE, text(release)),
                (o::ADOPT_TZDB_DATA, hashed(data)),
            ],
        ),
        Resolved::Script { write } => Nested::of(o::SCRIPT, [(o::SCRIPT_WRITE, text(write))]),
        _ => {
            panic!("{op:?} is a revert or an operator on derived data, grants, extensions or data")
        }
    }
}

/// Returns the BLAKE3-256 hash of `data` as a `Bytes` value.
fn hashed(data: &str) -> Nested {
    Nested::Scalar(Value::Bytes(
        blake3::hash(data.as_bytes()).as_bytes().to_vec(),
    ))
}

fn target(t: &Target) -> Nested {
    match t {
        Target::InRelation { relation, ctor } => Nested::of(
            o::IN_RELATION,
            [
                (o::IN_RELATION_RELATION, id(*relation)),
                (o::IN_RELATION_CTOR, id(*ctor)),
            ],
        ),
        Target::OfType { ctor } => one(o::OF_TYPE, o::OF_TYPE_CTOR, *ctor),
    }
}

fn placed(place: &Place) -> Nested {
    match place {
        Place::Last => Nested::of(o::LAST, []),
        Place::Before { ctor } => one(o::BEFORE, o::BEFORE_CTOR, *ctor),
        Place::After { ctor } => one(o::AFTER, o::AFTER_CTOR, *ctor),
    }
}

fn clause(c: &Clause) -> Nested {
    match c {
        Clause::Member { ctor } => one(o::MEMBER, o::MEMBER_CTOR, *ctor),
        Clause::Joins { set } => one(o::JOINS, o::JOINS_SET, *set),
        Clause::Scope { set } => one(o::SCOPE, o::SCOPE_SET, *set),
        Clause::Arm { target, arm } => {
            let target = match target {
                ArmTarget::OfFn { function } => one(o::OF_FN, o::OF_FN_FUNCTION, *function),
                ArmTarget::OfConstraint { constraint } => {
                    one(o::OF_CONSTRAINT, o::OF_CONSTRAINT_CONSTRAINT, *constraint)
                }
                ArmTarget::OfComputed { computed } => {
                    one(o::OF_COMPUTED, o::OF_COMPUTED_COMPUTED, *computed)
                }
            };
            Nested::of(o::ARM, [(o::ARM_TARGET, target), (o::ARM_ARM, text(arm))])
        }
        Clause::Replacement {
            index,
            kind,
            fields,
        } => {
            let kind = match kind {
                IndexKind::Unique => k::INDEX_UNIQUE,
                IndexKind::Plain => k::INDEX_PLAIN,
                IndexKind::Interval => k::INDEX_INTERVAL,
            };
            Nested::of(
                o::REPLACEMENT,
                [
                    (o::REPLACEMENT_INDEX, id(*index)),
                    (o::REPLACEMENT_KIND, Nested::of(nullary(kind), [])),
                    (o::REPLACEMENT_FIELDS, text(fields)),
                ],
            )
        }
        Clause::Carry { field } => one(o::CARRY, o::CARRY_FIELD, *field),
        Clause::Fill { field, fill } => two(o::FILL, (o::FILL_FIELD, *field), (o::FILL_FILL, fill)),
        Clause::Privilege { privilege, target } => Nested::of(
            o::PRIVILEGE,
            [
                (o::PRIVILEGE_PRIVILEGE, text(privilege)),
                (o::PRIVILEGE_TARGET, optional(Some(target))),
            ],
        ),
    }
}

/// Append `v`, a value stored as `t`, in the wire value encoding: a constructor of a sum of two
/// or more constructors tagged by its catalog id.
pub fn encode(t: &FieldType, v: &Value, out: &mut Vec<u8>) {
    match (t, v) {
        (FieldType::Scalar(_), v) => scalar(v, out),
        (FieldType::Record(ts), Value::Record(vs)) if ts.len() == vs.len() => {
            for (t, v) in ts.iter().zip(vs) {
                encode(t, v, out);
            }
        }
        (FieldType::Sum(cs), Value::Sum { ctor, fields }) => {
            let c = cs
                .get(*ctor as usize)
                .unwrap_or_else(|| panic!("constructor {ctor} of a sum of {}", cs.len()));
            if cs.len() > 1 {
                put_uvarint(u64::from(c.id.0), out);
            }
            assert_eq!(
                c.fields.len(),
                fields.len(),
                "a sum value gives every field of its constructor"
            );
            for (t, v) in c.fields.iter().zip(fields) {
                encode(t, v, out);
            }
        }
        (t, v) => panic!("{v:?} is a value of {t:?}"),
    }
}

fn bytes(b: &[u8], out: &mut Vec<u8>) {
    put_uvarint(b.len() as u64, out);
    out.extend_from_slice(b);
}

fn scalar(v: &Value, out: &mut Vec<u8>) {
    match v {
        Value::I8(n) => out.extend_from_slice(&n.to_le_bytes()),
        Value::I16(n) => out.extend_from_slice(&n.to_le_bytes()),
        Value::I32(n) | Value::Date(n) => out.extend_from_slice(&n.to_le_bytes()),
        Value::I64(n)
        | Value::Instant(n)
        | Value::Time(n)
        | Value::DateTime(n)
        | Value::Duration(n) => out.extend_from_slice(&n.to_le_bytes()),
        Value::F32(x) => out.extend_from_slice(&x.get().to_bits().to_le_bytes()),
        Value::F64(x) => out.extend_from_slice(&x.get().to_bits().to_le_bytes()),
        Value::Decimal(n) => out.extend_from_slice(&n.to_le_bytes()),
        Value::Text(s) => bytes(s.as_bytes(), out),
        Value::Bytes(b) | Value::SortKey(b) => bytes(b, out),
        Value::Period { months, days } => {
            out.extend_from_slice(&months.to_le_bytes());
            out.extend_from_slice(&days.to_le_bytes());
        }
        Value::ZonedDateTime { .. } => panic!("a catalog operator holds no zoned date-time"),
        Value::Record(_) | Value::Sum { .. } => panic!("{v:?} is a value of a scalar field type"),
    }
}
