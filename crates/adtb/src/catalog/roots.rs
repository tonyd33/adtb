//! The rows of the roots heap: each a value of one sum whose constructor ids are the file
//! format's, written and read only through [`RootsRow`].

use adtb_lang::ids::CatCtorId;

use crate::heap::{CtorType, FieldType};
use crate::key::{CtorId, OrderKey, Scalar, Value};
use crate::storage::Pid;

use super::Rot;
use super::deployment::{Fact, Mark, Origin};
use super::ids::{CatConstraintId, CatFieldId, CatIndexId, CatRelId};
use super::int;
use super::kept::{self, KeptRow, ROW_FORMAT};
use super::readers::Kind;

/// The constructor naming a heap.
const HEAP_ROOT: u32 = 0;
/// The constructor naming an index.
const INDEX_ROOT: u32 = 1;
/// The constructor holding a constructor's order key.
const ORDER_KEY: u32 = 2;
/// The constructor holding a migration's text.
const MIGRATION: u32 = 3;
/// The constructor naming a history heap.
const HISTORY_HEAP: u32 = 4;
/// The constructor naming a history key index.
const HISTORY_INDEX: u32 = 5;
/// The constructor naming a pending-dropped constraint, by its catalog id, and a bundle holding
/// it.
const HELD_CONSTRAINT: u32 = 6;
/// The constructor naming a pending-dropped check, by its catalog id, and a bundle holding it.
const HELD_CHECK: u32 = 7;
/// The constructor naming a pending-dropped unique, by its index, and a bundle holding it.
const HELD_UNIQUE: u32 = 8;
/// The constructor naming a pending-dropped old key, by its index, its relation and its fields,
/// and a bundle holding it.
const HELD_KEY: u32 = 9;
/// The constructor naming a role whose log grants became dormant at a version.
const DORMANT: u32 = 10;
/// The constructor marking a file whose families have durable owners.
const DEPLOYED: u32 = 11;
/// The constructor naming a family's owner.
const FAMILY: u32 = 12;
/// The constructor recording a bundle's successful registration.
const REGISTERED: u32 = 13;
/// The constructor holding a row of a volatile relation.
const KEPT: u32 = 14;
/// The constructor latching a lease as expired.
const EXPIRED: u32 = 15;

/// The fields of each constructor, at the position of its id.
const SHAPES: [(u32, &[Scalar]); 16] = {
    use Scalar::{Bytes, I64, Text};
    [
        (HEAP_ROOT, &[I64, I64, I64, I64]),
        (INDEX_ROOT, &[I64, I64]),
        (ORDER_KEY, &[I64, Bytes]),
        (MIGRATION, &[I64, Text]),
        (HISTORY_HEAP, &[I64, I64, I64, I64]),
        (HISTORY_INDEX, &[I64, I64, I64]),
        (HELD_CONSTRAINT, &[I64, Text, Text, Text, I64]),
        (HELD_CHECK, &[I64, Text, Text, Text, I64]),
        (HELD_UNIQUE, &[I64, Text, Text, Text, I64]),
        (HELD_KEY, &[I64, Text, Text, Text, I64, I64, Bytes]),
        (DORMANT, &[I64, Text]),
        (DEPLOYED, &[]),
        (FAMILY, &[Text, I64]),
        (REGISTERED, &[I64, I64, I64, Text]),
        (KEPT, &[I64, Text, I64, I64, Bytes]),
        (EXPIRED, &[I64, I64]),
    ]
};

const _: () = {
    let mut at = 0;
    while at < SHAPES.len() {
        assert!(
            SHAPES[at].0 as usize == at,
            "a roots constructor's id is its position"
        );
        at += 1;
    }
};

pub(super) fn row_type() -> Vec<FieldType> {
    let ctors = SHAPES
        .iter()
        .map(|&(id, fields)| CtorType {
            id: CtorId(id),
            fields: fields.iter().map(|&s| FieldType::Scalar(s)).collect(),
        })
        .collect();
    vec![FieldType::Sum(ctors)]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RootsRow {
    HeapRoot {
        rel: CatRelId,
        ctor: CatCtorId,
        heap: u32,
        at: Pid,
    },
    IndexRoot {
        index: u64,
        at: Pid,
    },
    OrderKey {
        ctor: u64,
        key: OrderKey,
    },
    Migration {
        version: u64,
        text: String,
    },
    HistoryHeap {
        rel: CatRelId,
        ctor: CatCtorId,
        heap: u32,
        at: Pid,
    },
    HistoryIndex {
        rel: CatRelId,
        owner: u32,
        at: Pid,
    },
    /// A bundle `holder` holding the drop of `kind`, `module::name`, at version `dropped`.
    Held {
        dropped: u64,
        module: String,
        name: String,
        holder: String,
        kind: Kind,
    },
    Dormant {
        version: u64,
        role: String,
    },
    Deployment(Fact),
    /// A kept row as stored, its fields undecoded.
    Kept {
        relation: i64,
        group: String,
        ctor: i64,
        format: i64,
        row: Vec<u8>,
    },
}

fn id(v: &Value) -> Result<u64, Rot> {
    match v {
        Value::I64(n) => u64::try_from(*n).map_err(|_| Rot::NegativeId(*n)),
        other => Err(Rot::NotAnId(other.clone())),
    }
}

fn heap_id(v: &Value) -> Result<u32, Rot> {
    let n = id(v)?;
    u32::try_from(n).map_err(|_| Rot::HeapIdWide(n))
}

fn pid(v: &Value) -> Result<Pid, Rot> {
    Ok(Pid(id(v)?))
}

impl RootsRow {
    /// Returns the row of the roots heap holding this one.
    pub(super) fn encode(&self) -> Vec<Value> {
        let (ctor, fields) = self.parts();
        vec![Value::Sum { ctor, fields }]
    }

    /// Returns what this row records, as a refusal of it names it.
    pub(super) fn what(&self) -> String {
        match self {
            RootsRow::Migration { version, .. } => format!("the text of migration {version}"),
            RootsRow::Held {
                module,
                name,
                holder,
                ..
            } => format!("the hold of bundle {holder} on `{module}::{name}`"),
            RootsRow::Dormant { role, .. } => format!("the dormant grants of role {role}"),
            RootsRow::Deployment(_) => "a deployment fact".to_string(),
            RootsRow::Kept { relation, .. } => format!("a kept row of relation {relation}"),
            RootsRow::OrderKey { ctor, .. } => format!("the order key of constructor {ctor}"),
            RootsRow::HeapRoot { .. }
            | RootsRow::IndexRoot { .. }
            | RootsRow::HistoryHeap { .. }
            | RootsRow::HistoryIndex { .. } => "a root".to_string(),
        }
    }

    /// Returns the constructor id and the fields of this row.
    pub(super) fn parts(&self) -> (u32, Vec<Value>) {
        match self {
            RootsRow::HeapRoot {
                rel,
                ctor,
                heap,
                at,
            } => (
                HEAP_ROOT,
                vec![
                    int(rel.raw()),
                    int(ctor.raw()),
                    int(u64::from(*heap)),
                    int(at.0),
                ],
            ),
            RootsRow::IndexRoot { index, at } => (INDEX_ROOT, vec![int(*index), int(at.0)]),
            RootsRow::OrderKey { ctor, key } => (
                ORDER_KEY,
                vec![int(*ctor), Value::Bytes(key.as_bytes().to_vec())],
            ),
            RootsRow::Migration { version, text } => {
                (MIGRATION, vec![int(*version), Value::Text(text.clone())])
            }
            RootsRow::HistoryHeap {
                rel,
                ctor,
                heap,
                at,
            } => (
                HISTORY_HEAP,
                vec![
                    int(rel.raw()),
                    int(ctor.raw()),
                    int(u64::from(*heap)),
                    int(at.0),
                ],
            ),
            RootsRow::HistoryIndex { rel, owner, at } => (
                HISTORY_INDEX,
                vec![int(rel.raw()), int(u64::from(*owner)), int(at.0)],
            ),
            RootsRow::Held {
                dropped,
                module,
                name,
                holder,
                kind,
            } => {
                let mut fields = vec![
                    int(*dropped),
                    Value::Text(module.clone()),
                    Value::Text(name.clone()),
                    Value::Text(holder.clone()),
                ];
                let ctor = match kind {
                    Kind::Constraint(id) => {
                        fields.push(int(id.raw()));
                        HELD_CONSTRAINT
                    }
                    Kind::Check(id) => {
                        fields.push(int(id.raw()));
                        HELD_CHECK
                    }
                    Kind::Unique(id) => {
                        fields.push(int(id.raw()));
                        HELD_UNIQUE
                    }
                    Kind::Key {
                        index,
                        relation,
                        fields: key,
                    } => {
                        fields.extend([
                            int(index.raw()),
                            int(relation.raw()),
                            Value::Bytes(key.iter().flat_map(|f| f.raw().to_le_bytes()).collect()),
                        ]);
                        HELD_KEY
                    }
                };
                (ctor, fields)
            }
            RootsRow::Dormant { version, role } => {
                (DORMANT, vec![int(*version), Value::Text(role.clone())])
            }
            RootsRow::Deployment(Fact::Deployed) => (DEPLOYED, Vec::new()),
            RootsRow::Deployment(Fact::Family(family, role)) => {
                (FAMILY, vec![Value::Text(family.clone()), Value::I64(*role)])
            }
            RootsRow::Deployment(Fact::Registered(bundle, m)) => (
                REGISTERED,
                vec![
                    Value::I64(*bundle),
                    Value::I64(m.role),
                    int(match m.origin {
                        Origin::Open => 0,
                        Origin::Command => 1,
                    }),
                    Value::Text(m.text.clone()),
                ],
            ),
            RootsRow::Deployment(Fact::Expired(bundle, renewed)) => {
                (EXPIRED, vec![Value::I64(*bundle), Value::I64(*renewed)])
            }
            RootsRow::Kept {
                relation,
                group,
                ctor,
                format,
                row,
            } => (
                KEPT,
                vec![
                    Value::I64(*relation),
                    Value::Text(group.clone()),
                    Value::I64(*ctor),
                    Value::I64(*format),
                    Value::Bytes(row.clone()),
                ],
            ),
        }
    }

    /// Returns the row the roots heap row `row` holds.
    ///
    /// Fail when `row` is not one value of a constructor of the roots heap, or a field holds a
    /// value its constructor refuses.
    pub(super) fn decode(row: &[Value]) -> Result<RootsRow, Rot> {
        let [Value::Sum { ctor, fields }] = row else {
            return Err(Rot::RootsRow(row.to_vec()));
        };
        let ctor = *ctor;
        Ok(match (ctor, fields.as_slice()) {
            (HEAP_ROOT, [rel, c, heap, at]) => RootsRow::HeapRoot {
                rel: CatRelId::from_raw(id(rel)?),
                ctor: CatCtorId::from_raw(id(c)?),
                heap: heap_id(heap)?,
                at: pid(at)?,
            },
            (INDEX_ROOT, [index, at]) => RootsRow::IndexRoot {
                index: id(index)?,
                at: pid(at)?,
            },
            (ORDER_KEY, [c, Value::Bytes(key)]) => RootsRow::OrderKey {
                ctor: id(c)?,
                key: OrderKey::new(key.clone()).ok_or_else(|| Rot::OrderKey(key.clone()))?,
            },
            (MIGRATION, [version, Value::Text(text)]) => RootsRow::Migration {
                version: id(version)?,
                text: text.clone(),
            },
            (HISTORY_HEAP, [rel, c, heap, at]) => RootsRow::HistoryHeap {
                rel: CatRelId::from_raw(id(rel)?),
                ctor: CatCtorId::from_raw(id(c)?),
                heap: heap_id(heap)?,
                at: pid(at)?,
            },
            (HISTORY_INDEX, [rel, owner, at]) => RootsRow::HistoryIndex {
                rel: CatRelId::from_raw(id(rel)?),
                owner: heap_id(owner)?,
                at: pid(at)?,
            },
            (HELD_CONSTRAINT | HELD_CHECK | HELD_UNIQUE | HELD_KEY, _) => held(ctor, fields)?,
            (DORMANT, [version, Value::Text(role)]) => RootsRow::Dormant {
                version: id(version)?,
                role: role.clone(),
            },
            (DEPLOYED | FAMILY | REGISTERED | EXPIRED, _) => {
                RootsRow::Deployment(fact(ctor, fields)?)
            }
            (
                KEPT,
                [
                    Value::I64(relation),
                    Value::Text(group),
                    Value::I64(c),
                    Value::I64(format),
                    Value::Bytes(row),
                ],
            ) => RootsRow::Kept {
                relation: *relation,
                group: group.clone(),
                ctor: *c,
                format: *format,
                row: row.clone(),
            },
            (KEPT, _) => return Err(Rot::KeptRow(fields.clone())),
            _ => {
                return Err(Rot::RootsCtor {
                    ctor,
                    fields: fields.clone(),
                });
            }
        })
    }
}

fn held(ctor: u32, fields: &[Value]) -> Result<RootsRow, Rot> {
    let corrupt = || Rot::PendingRow {
        ctor,
        fields: fields.to_vec(),
    };
    let Some((
        [
            dropped,
            Value::Text(module),
            Value::Text(name),
            Value::Text(holder),
        ],
        index,
    )) = fields.split_first_chunk::<4>()
    else {
        return Err(corrupt());
    };
    let kind = match (ctor, index) {
        (HELD_CONSTRAINT, [n]) => Kind::Constraint(CatConstraintId::from_raw(id(n)?)),
        (HELD_CHECK, [n]) => Kind::Check(CatConstraintId::from_raw(id(n)?)),
        (HELD_UNIQUE, [n]) => Kind::Unique(CatIndexId::from_raw(id(n)?)),
        (HELD_KEY, [index, rel, Value::Bytes(key)]) => {
            let (chunks, rest) = key.as_chunks::<8>();
            if !rest.is_empty() {
                return Err(corrupt());
            }
            Kind::Key {
                index: CatIndexId::from_raw(id(index)?),
                relation: CatRelId::from_raw(id(rel)?),
                fields: chunks
                    .iter()
                    .map(|c| CatFieldId::from_raw(u64::from_le_bytes(*c)))
                    .collect(),
            }
        }
        _ => return Err(corrupt()),
    };
    Ok(RootsRow::Held {
        dropped: id(dropped)?,
        module: module.clone(),
        name: name.clone(),
        holder: holder.clone(),
        kind,
    })
}

fn fact(ctor: u32, fields: &[Value]) -> Result<Fact, Rot> {
    Ok(match (ctor, fields) {
        (DEPLOYED, []) => Fact::Deployed,
        (FAMILY, [Value::Text(family), Value::I64(role)]) => Fact::Family(family.clone(), *role),
        (
            REGISTERED,
            [
                Value::I64(bundle),
                Value::I64(role),
                origin,
                Value::Text(text),
            ],
        ) => {
            let origin = match id(origin)? {
                0 => Origin::Open,
                1 => Origin::Command,
                n => return Err(Rot::Origin(n)),
            };
            Fact::Registered(
                *bundle,
                Mark {
                    role: *role,
                    origin,
                    text: text.clone(),
                },
            )
        }
        (EXPIRED, [Value::I64(bundle), Value::I64(renewed)]) => Fact::Expired(*bundle, *renewed),
        _ => {
            return Err(Rot::DeploymentRow {
                ctor,
                fields: fields.to_vec(),
            });
        }
    })
}

/// Returns the roots row holding kept row `r`.
pub(super) fn kept(r: &KeptRow) -> RootsRow {
    let signed = |n: u64| i64::try_from(n).expect("an id fits 63 bits");
    RootsRow::Kept {
        relation: signed(r.relation),
        group: r.group.clone(),
        ctor: signed(r.ctor.map_or(0, CatCtorId::raw)),
        format: signed(ROW_FORMAT),
        row: kept::encode(&r.fields),
    }
}

/// Returns the kept row a roots row of fields `relation`, `group`, `ctor`, `format` and `row`
/// holds.
pub(super) fn kept_row(
    relation: i64,
    group: &str,
    ctor: i64,
    format: i64,
    row: &[u8],
) -> Result<KeptRow, Rot> {
    let bad = || {
        Rot::KeptRow(vec![
            Value::I64(relation),
            Value::Text(group.into()),
            Value::I64(ctor),
            Value::I64(format),
            Value::Bytes(row.to_vec()),
        ])
    };
    let format = id(&Value::I64(format))?;
    if format != ROW_FORMAT {
        return Err(Rot::KeptFormat(format));
    }
    Ok(KeptRow {
        relation: id(&Value::I64(relation))?,
        group: group.into(),
        ctor: match id(&Value::I64(ctor))? {
            0 => None,
            id => Some(CatCtorId::from_raw(id)),
        },
        fields: kept::decode(row).ok_or_else(bad)?,
    })
}

#[cfg(test)]
#[path = "roots_tests.rs"]
mod tests;
