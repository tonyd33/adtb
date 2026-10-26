use adtb_lang::evolve::{
    ArmTarget, Clause, IndexKind, Place, Resolved, ResolvedOp, Source, Target,
};
use adtb_lang::schema::{Schema, ValueDef};
use adtb_lang::types::Ty;

use crate::heap::{FieldType, put_uvarint};
use crate::key::Value;

use super::layout;

/// A BLAKE3-256 hash.
pub type Hash = [u8; 32];

/// A resolved operator as a value of `catalog::Operator`, with its clauses as values of
/// `catalog::ClauseBody` in clause order.
pub type OperatorRow = (Value, Vec<Value>);

/// Returns the hash of the version `source` is at: `h(0)` is 32 zero bytes, and `h(n)` the hash
/// of `h(n - 1)` followed by the encoding of migration `n`'s resolved operators.
///
/// Panics if a migration of the log does not resolve.
pub fn version_hash(schema: &Schema, source: &Source) -> Hash {
    let mut h = [0; 32];
    for l in &source.log {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&h);
        hasher.update(&encoded(schema, &operators(schema, source, l.number)));
        h = *hasher.finalize().as_bytes();
    }
    h
}

/// Returns the resolved operators of migration `number` of `source` in ordinal order.
///
/// Panics if the migration does not resolve.
pub fn operators(schema: &Schema, source: &Source, number: u64) -> Vec<OperatorRow> {
    let ops = source
        .resolved(number)
        .unwrap_or_else(|ds| panic!("logged migration {number} resolves: {ds:?}"));
    let op_ty = field_ty(schema, "migration_ops", "op");
    let clause_ty = field_ty(schema, "migration_clauses", "body");
    ops.iter()
        .map(|op| {
            let clauses = op.clauses.iter().map(|c| clause(schema, &clause_ty, c));
            (operator(schema, &op_ty, op), clauses.collect())
        })
        .collect()
}

/// Returns the encoding of a migration's operators: their count, then each operator, the count
/// of its clauses and each clause, in the wire value encoding.
pub fn encoded(schema: &Schema, ops: &[OperatorRow]) -> Vec<u8> {
    let op_ty = layout::field_type(schema, &field_ty(schema, "migration_ops", "op"));
    let clause_ty = layout::field_type(schema, &field_ty(schema, "migration_clauses", "body"));
    let mut out = Vec::new();
    put_uvarint(ops.len() as u64, &mut out);
    for (op, clauses) in ops {
        encode(&op_ty, op, &mut out);
        put_uvarint(clauses.len() as u64, &mut out);
        for c in clauses {
            encode(&clause_ty, c, &mut out);
        }
    }
    out
}

/// Returns the type of field `field` of the rows of `catalog::{relation}`.
fn field_ty(schema: &Schema, relation: &str, field: &str) -> Ty {
    let m = schema.module_named("catalog").expect("the catalog module");
    let Some(&ValueDef::Relation(r)) = schema.module(m).values.get(relation) else {
        panic!("the catalog declares relation `{relation}`")
    };
    layout::row(schema, r, 0)
        .into_iter()
        .find(|(n, _)| n == field)
        .unwrap_or_else(|| panic!("a row of `{relation}` has field `{field}`"))
        .1
}

fn id(n: u64) -> Value {
    Value::I64(i64::try_from(n).expect("a catalog id fits 63 bits"))
}

fn text(s: &str) -> Value {
    Value::Text(s.to_string())
}

/// Returns the value of constructor `ctor` of the declared sum `ty`, its fields given by name.
fn sum(schema: &Schema, ty: &Ty, ctor: &str, fields: &[(&str, Value)]) -> Value {
    let Ty::Data { def, .. } = ty else {
        panic!("{ty:?} is a declared sum")
    };
    let k = schema
        .ty(*def)
        .ctor_index(ctor)
        .unwrap_or_else(|| panic!("`{}` has constructor `{ctor}`", schema.ty(*def).name));
    let names = layout::ctors(schema, ty).swap_remove(k as usize).1;
    assert_eq!(
        names.len(),
        fields.len(),
        "a value of `{ctor}` gives every field"
    );
    let fields = names
        .iter()
        .map(|(n, _)| {
            fields
                .iter()
                .find(|(f, _)| f == n)
                .unwrap_or_else(|| panic!("a value of `{ctor}` gives `{n}`"))
                .1
                .clone()
        })
        .collect();
    Value::Sum { ctor: k, fields }
}

fn boolean(schema: &Schema, b: bool) -> Value {
    let ty = Ty::Data {
        def: schema.prelude_type("Bool"),
        args: Vec::new(),
        ctors: None,
    };
    sum(schema, &ty, if b { "True" } else { "False" }, &[])
}

fn operator(schema: &Schema, ty: &Ty, op: &ResolvedOp) -> Value {
    let v = |ctor: &str, fields: &[(&str, Value)]| sum(schema, ty, ctor, fields);
    match &op.op {
        Resolved::AddModule { module } => v("AddModule", &[("module", text(module))]),
        Resolved::AddType { sum, decl } => v("AddType", &[("sum", id(*sum)), ("decl", text(decl))]),
        Resolved::AddRelation { relation, decl } => v(
            "AddRelation",
            &[("relation", id(*relation)), ("decl", text(decl))],
        ),
        Resolved::AddSet {
            set,
            relation,
            name,
        } => v(
            "AddSet",
            &[
                ("set", id(*set)),
                ("relation", id(*relation)),
                ("name", text(name)),
            ],
        ),
        Resolved::AddUnique {
            index,
            relation,
            fields,
        } => v(
            "AddUnique",
            &[
                ("index", id(*index)),
                ("relation", id(*relation)),
                ("fields", text(fields)),
            ],
        ),
        Resolved::AddIndex {
            index,
            relation,
            interval,
            fields,
        } => v(
            "AddIndex",
            &[
                ("index", id(*index)),
                ("relation", id(*relation)),
                ("interval", boolean(schema, *interval)),
                ("fields", text(fields)),
            ],
        ),
        Resolved::DefineFn { function, decl } => v(
            "DefineFn",
            &[("function", id(*function)), ("decl", text(decl))],
        ),
        Resolved::DefineAggregate { function, decl } => v(
            "DefineAggregate",
            &[("function", id(*function)), ("decl", text(decl))],
        ),
        Resolved::AddConstraint { constraint, body } => v(
            "AddConstraint",
            &[("constraint", id(*constraint)), ("body", text(body))],
        ),
        Resolved::AddCheck { constraint, body } => v(
            "AddCheck",
            &[("constraint", id(*constraint)), ("body", text(body))],
        ),
        other => changing(schema, ty, other),
    }
}

/// Returns the value of an operator that drops, renames or reshapes objects.
fn changing(schema: &Schema, ty: &Ty, op: &Resolved) -> Value {
    let v = |ctor: &str, fields: &[(&str, Value)]| sum(schema, ty, ctor, fields);
    let field = |ctor: &str, name: &str| field_of(schema, ty, ctor, name);
    let target = |ctor: &str, t: &Target| {
        let tt = field(ctor, "target");
        let value = match t {
            Target::InRelation { relation, ctor } => sum(
                schema,
                &tt,
                "InRelation",
                &[("relation", id(*relation)), ("ctor", id(*ctor))],
            ),
            Target::OfType { ctor } => sum(schema, &tt, "OfType", &[("ctor", id(*ctor))]),
        };
        v(ctor, &[("target", value)])
    };
    match op {
        Resolved::DropModule { module } => v("DropModule", &[("module", text(module))]),
        Resolved::DropType { sum } => v("DropType", &[("sum", id(*sum))]),
        Resolved::DropRelation { relation } => v("DropRelation", &[("relation", id(*relation))]),
        Resolved::DropSet { set } => v("DropSet", &[("set", id(*set))]),
        Resolved::AddConstructor {
            ctor,
            sum: of,
            decl,
            place,
        } => {
            let pt = field("AddConstructor", "place");
            let place = match place {
                Place::Last => sum(schema, &pt, "Last", &[]),
                Place::Before { ctor } => sum(schema, &pt, "Before", &[("ctor", id(*ctor))]),
                Place::After { ctor } => sum(schema, &pt, "After", &[("ctor", id(*ctor))]),
            };
            v(
                "AddConstructor",
                &[
                    ("ctor", id(*ctor)),
                    ("sum", id(*of)),
                    ("decl", text(decl)),
                    ("place", place),
                ],
            )
        }
        Resolved::RemoveConstructor { ctor } => v("RemoveConstructor", &[("ctor", id(*ctor))]),
        Resolved::Enable { target: t } => target("Enable", t),
        Resolved::Disable { target: t } => target("Disable", t),
        Resolved::Deprecate { target: t } => target("Deprecate", t),
        Resolved::RenameConstructor { ctor, name } => v(
            "RenameConstructor",
            &[("ctor", id(*ctor)), ("name", text(name))],
        ),
        Resolved::RenameType { sum, name } => {
            v("RenameType", &[("sum", id(*sum)), ("name", text(name))])
        }
        Resolved::RenameRelation { relation, name } => v(
            "RenameRelation",
            &[("relation", id(*relation)), ("name", text(name))],
        ),
        Resolved::RenameSet { set, name } => {
            v("RenameSet", &[("set", id(*set)), ("name", text(name))])
        }
        Resolved::RenameField { field, name } => v(
            "RenameField",
            &[("field", id(*field)), ("name", text(name))],
        ),
        Resolved::DropUnique { index } => v("DropUnique", &[("index", id(*index))]),
        Resolved::DropIndex { index } => v("DropIndex", &[("index", id(*index))]),
        Resolved::Rekey {
            relation,
            key,
            fields,
        } => v(
            "Rekey",
            &[
                ("relation", id(*relation)),
                ("key", id(*key)),
                ("fields", text(fields)),
            ],
        ),
        Resolved::RedefineFn { function, decl } => v(
            "RedefineFn",
            &[("function", id(*function)), ("decl", text(decl))],
        ),
        Resolved::DropFn { function } => v("DropFn", &[("function", id(*function))]),
        Resolved::DropAggregate { function } => v("DropAggregate", &[("function", id(*function))]),
        Resolved::DropConstraint { constraint } => {
            v("DropConstraint", &[("constraint", id(*constraint))])
        }
        Resolved::DropCheck { constraint } => v("DropCheck", &[("constraint", id(*constraint))]),
        Resolved::Revert { migration } => v("Revert", &[("migration", id(*migration))]),
        other => reshaping(schema, ty, other),
    }
}

/// Returns the value of an operator that reshapes constructors or fields.
fn reshaping(schema: &Schema, ty: &Ty, op: &Resolved) -> Value {
    let v = |ctor: &str, fields: &[(&str, Value)]| sum(schema, ty, ctor, fields);
    let option = |ctor: &str, name: &str, t: Option<&String>| {
        let o = field_of(schema, ty, ctor, name);
        match t {
            Some(t) => sum(schema, &o, "Some", &[("value", text(t))]),
            None => sum(schema, &o, "None", &[]),
        }
    };
    match op {
        Resolved::Split {
            ctor,
            decl,
            routing,
        } => v(
            "Split",
            &[
                ("ctor", id(*ctor)),
                ("decl", text(decl)),
                ("routing", text(routing)),
            ],
        ),
        Resolved::Merge {
            ctor,
            decl,
            routing,
        } => v(
            "Merge",
            &[
                ("ctor", id(*ctor)),
                ("decl", text(decl)),
                ("routing", option("Merge", "routing", routing.as_ref())),
            ],
        ),
        Resolved::Nest { ctor, decl } => v("Nest", &[("ctor", id(*ctor)), ("decl", text(decl))]),
        Resolved::Unnest { ctor } => v("Unnest", &[("ctor", id(*ctor))]),
        Resolved::AddField { field, decl, fill } => v(
            "AddField",
            &[
                ("field", id(*field)),
                ("decl", text(decl)),
                ("fill", text(fill)),
            ],
        ),
        Resolved::DropField { field } => v("DropField", &[("field", id(*field))]),
        Resolved::MoveField {
            field,
            header,
            fill,
        } => v(
            "MoveField",
            &[
                ("field", id(*field)),
                ("header", boolean(schema, *header)),
                ("fill", option("MoveField", "fill", fill.as_ref())),
            ],
        ),
        declaring => panic!("{declaring:?} is a declaring operator"),
    }
}

/// Returns the type of field `name` of constructor `ctor` of the declared sum `ty`.
fn field_of(schema: &Schema, ty: &Ty, ctor: &str, name: &str) -> Ty {
    let Ty::Data { def, .. } = ty else {
        panic!("{ty:?} is a declared sum")
    };
    let k = schema
        .ty(*def)
        .ctor_index(ctor)
        .expect("an operator constructor");
    layout::ctors(schema, ty)
        .swap_remove(k as usize)
        .1
        .into_iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("`{ctor}` has field `{name}`"))
        .1
}

fn clause(schema: &Schema, ty: &Ty, c: &Clause) -> Value {
    match c {
        Clause::Member { ctor } => sum(schema, ty, "Member", &[("ctor", id(*ctor))]),
        Clause::Joins { set } => sum(schema, ty, "Joins", &[("set", id(*set))]),
        Clause::Arm { target, arm } => {
            let at = field_of(schema, ty, "Arm", "target");
            let target = match target {
                ArmTarget::OfFn { function } => {
                    sum(schema, &at, "OfFn", &[("function", id(*function))])
                }
                ArmTarget::OfConstraint { constraint } => sum(
                    schema,
                    &at,
                    "OfConstraint",
                    &[("constraint", id(*constraint))],
                ),
                ArmTarget::OfComputed { computed } => {
                    sum(schema, &at, "OfComputed", &[("computed", id(*computed))])
                }
            };
            sum(schema, ty, "Arm", &[("target", target), ("arm", text(arm))])
        }
        Clause::Replacement {
            index,
            kind,
            fields,
        } => {
            let kt = field_of(schema, ty, "Replacement", "kind");
            let kind = match kind {
                IndexKind::Unique => "Unique",
                IndexKind::Plain => "Plain",
                IndexKind::Interval => "Interval",
            };
            sum(
                schema,
                ty,
                "Replacement",
                &[
                    ("index", id(*index)),
                    ("kind", sum(schema, &kt, kind, &[])),
                    ("fields", text(fields)),
                ],
            )
        }
        Clause::Carry { field } => sum(schema, ty, "Carry", &[("field", id(*field))]),
        Clause::Fill { field, fill } => sum(
            schema,
            ty,
            "Fill",
            &[("field", id(*field)), ("fill", text(fill))],
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
