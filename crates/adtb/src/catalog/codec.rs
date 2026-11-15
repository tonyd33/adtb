//! Catalog rows as typed values: one codec per catalog relation, reading a row's constructor by
//! its fixed id and each field by its declared name.

use std::marker::PhantomData;

use adtb_lang::ids::{engine_ctor as k, engine_rel};
use adtb_lang::schema::Schema;
use adtb_lang::schema::prelude::ctor;
use adtb_lang::types::RelId;

use crate::exec::{self, Val};
use crate::key::{F64, Value};

use super::Rot;
use super::ids::{CatCtorId, CatRelId, ctor_id, relation};
use super::layout;

#[cfg(test)]
#[path = "codec_tests.rs"]
mod tests;

/// The names of all the fields of one constructor, the header's included.
pub type Fields = &'static [&'static str];

/// One field of a constructor's [`Fields`].
#[derive(Clone, Copy, Debug)]
pub struct Field {
    fields: Fields,
    at: usize,
}

const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Returns field `name` of `fields`. Fails to compile in a constant when `fields` does not list
/// `name`.
pub const fn field(fields: Fields, name: &str) -> Field {
    let mut at = 0;
    while at < fields.len() {
        if same(fields[at], name) {
            return Field { fields, at };
        }
        at += 1;
    }
    panic!("a codec reads a field its constructor lists")
}

/// One constructor a codec reads: its fixed id, its fields, and how a row of it reads.
pub struct Ctor<T> {
    id: CatCtorId,
    fields: Fields,
    read: fn(&Row<'_>) -> Result<T, Rot>,
}

impl<T> Ctor<T> {
    pub const fn new(id: CatCtorId, fields: Fields, read: fn(&Row<'_>) -> Result<T, Rot>) -> Self {
        Ctor { id, fields, read }
    }
}

/// A typed row of one catalog relation.
pub trait Decode: Sized + 'static {
    const RELATION: CatRelId;
    /// Each constructor of the relation.
    const CTORS: &'static [Ctor<Self>];
}

/// The constructors and fields of the rows of relation `T::RELATION` in one schema.
pub struct Codec<T: 'static> {
    relation: RelId,
    name: String,
    ctors: Vec<Shape<T>>,
    row: PhantomData<fn() -> T>,
}

struct Shape<T: 'static> {
    ctor: &'static Ctor<T>,
    name: String,
    /// The stored position of each field of `ctor.fields`, in its order.
    positions: Vec<usize>,
}

impl<T: Decode> Codec<T> {
    /// Returns the codec of `T` over `schema`.
    ///
    /// Preconditions: `schema` declares `T::RELATION` with exactly the constructors and fields
    /// `T::CTORS` lists, fields in any order.
    pub fn new(schema: &Schema) -> Codec<T> {
        let r = relation(schema, T::RELATION)
            .unwrap_or_else(|| panic!("the schema declares engine relation {}", T::RELATION));
        let rel = schema.relation(r);
        let name = format!("{}::{}", schema.module(rel.module).name, rel.name);
        let declared = schema.ty(rel.elem_def).ctors();
        assert_eq!(
            T::CTORS.len(),
            declared.len(),
            "the codec of {name} lists each constructor once"
        );
        let ctors = (0..declared.len())
            .map(|c| {
                let c = u32::try_from(c).expect("a constructor position fits 32 bits");
                let id = ctor_id(schema, r, c);
                let stored: Vec<String> = layout::row(schema, r, c)
                    .into_iter()
                    .map(|(n, _)| n)
                    .collect();
                let ctor = T::CTORS
                    .iter()
                    .find(|k| k.id == id)
                    .unwrap_or_else(|| panic!("the codec of {name} lists constructor {id}"));
                let mut want: Vec<&str> = ctor.fields.to_vec();
                let mut have: Vec<&str> = stored.iter().map(String::as_str).collect();
                want.sort_unstable();
                have.sort_unstable();
                assert_eq!(
                    want, have,
                    "the codec of {name} lists the fields of constructor {id}"
                );
                let positions = ctor
                    .fields
                    .iter()
                    .map(|f| {
                        stored
                            .iter()
                            .position(|s| s == f)
                            .expect("each listed field is declared")
                    })
                    .collect();
                Shape {
                    ctor,
                    name: declared[c as usize].name.clone(),
                    positions,
                }
            })
            .collect();
        Codec {
            relation: r,
            name,
            ctors,
            row: PhantomData,
        }
    }

    pub fn relation(&self) -> RelId {
        self.relation
    }

    fn shape(&self, ctor: u32) -> Result<&Shape<T>, Rot> {
        self.ctors
            .get(ctor as usize)
            .ok_or_else(|| Rot::CatalogRow {
                row: self.name.clone(),
                detail: format!("has no constructor at position {ctor}"),
            })
    }

    /// Returns the typed row of `values`, a stored row of the constructor at position `ctor`.
    ///
    /// Fail with [`Rot::CatalogRow`] when the relation has no constructor at `ctor`, `values`
    /// holds another count of fields than its constructor, or a field holds a value not of its
    /// type.
    pub fn decode(&self, ctor: u32, values: &[Value]) -> Result<T, Rot> {
        let shape = self.shape(ctor)?;
        let row = Row {
            relation: &self.name,
            name: &shape.name,
            fields: shape.ctor.fields,
            positions: &shape.positions,
            values,
        };
        if values.len() != shape.positions.len() {
            return Err(row.refused(format!(
                "holds {} fields, not {}",
                values.len(),
                shape.positions.len()
            )));
        }
        (shape.ctor.read)(&row)
    }

    /// Returns the typed row of `v`, a value of the relation's element type.
    ///
    /// Fail as [`Codec::decode`] does, and when `v` is not a constructor's value or a field of it
    /// has no stored form of its type.
    pub fn decode_val(&self, schema: &Schema, v: &Val) -> Result<T, Rot> {
        let Val::Ctor(c, fields) = v else {
            return Err(Rot::CatalogRow {
                row: self.name.clone(),
                detail: format!("is {v:?}"),
            });
        };
        self.shape(*c)?;
        let stored = layout::row(schema, self.relation, *c)
            .iter()
            .zip(fields.iter())
            .map(|((_, t), v)| exec::to_stored(&layout::field_type(schema, t), v).ok())
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| Rot::CatalogRow {
                row: self.name.clone(),
                detail: format!("has no stored form: {v:?}"),
            })?;
        self.decode(*c, &stored)
    }
}

/// One stored row of a catalog relation, its fields read by [`Field`].
pub struct Row<'a> {
    relation: &'a str,
    name: &'a str,
    fields: Fields,
    positions: &'a [usize],
    values: &'a [Value],
}

impl Row<'_> {
    fn refused(&self, detail: String) -> Rot {
        Rot::CatalogRow {
            row: format!("{}.{}", self.relation, self.name),
            detail,
        }
    }

    fn get(&self, f: Field) -> &Value {
        assert!(
            f.fields == self.fields,
            "a codec reads a field of the constructor it reads"
        );
        &self.values[self.positions[f.at]]
    }

    fn bad(&self, f: Field) -> Rot {
        self.refused(format!(
            "holds {:?} in field `{}`",
            self.get(f),
            f.fields[f.at]
        ))
    }

    pub fn int(&self, f: Field) -> Result<i64, Rot> {
        match self.get(f) {
            Value::I64(n) => Ok(*n),
            _ => Err(self.bad(f)),
        }
    }

    pub fn text(&self, f: Field) -> Result<&str, Rot> {
        match self.get(f) {
            Value::Text(t) => Ok(t),
            _ => Err(self.bad(f)),
        }
    }

    pub fn instant(&self, f: Field) -> Result<i64, Rot> {
        match self.get(f) {
            Value::Instant(t) => Ok(*t),
            _ => Err(self.bad(f)),
        }
    }

    /// Returns the duration in field `f`, in microseconds.
    pub fn duration(&self, f: Field) -> Result<i64, Rot> {
        match self.get(f) {
            Value::Duration(d) => Ok(*d),
            _ => Err(self.bad(f)),
        }
    }

    pub fn float(&self, f: Field) -> Result<F64, Rot> {
        match self.get(f) {
            Value::F64(x) => Ok(*x),
            _ => Err(self.bad(f)),
        }
    }

    /// Returns the duration an `Option Duration` field holds, in microseconds.
    pub fn optional_duration(&self, f: Field) -> Result<Option<i64>, Rot> {
        match self.get(f) {
            Value::Sum {
                ctor: ctor::NONE,
                fields,
            } if fields.is_empty() => Ok(None),
            Value::Sum {
                ctor: ctor::SOME,
                fields,
            } => match fields.as_slice() {
                [Value::Duration(d)] => Ok(Some(*d)),
                _ => Err(self.bad(f)),
            },
            _ => Err(self.bad(f)),
        }
    }
}

/// A row of `catalog::roles`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleRow {
    id: i64,
    name: String,
}

impl RoleRow {
    /// Returns the role's id and name.
    pub fn into_parts(self) -> (i64, String) {
        (self.id, self.name)
    }
}

const ROLE: Fields = &["id", "name", "created"];
const ROLE_ID: Field = field(ROLE, "id");
const ROLE_NAME: Field = field(ROLE, "name");
const ROLE_CREATED: Field = field(ROLE, "created");

impl Decode for RoleRow {
    const RELATION: CatRelId = engine_rel::ROLES;
    const CTORS: &'static [Ctor<RoleRow>] = &[Ctor::new(k::ROLE, ROLE, |row| {
        row.instant(ROLE_CREATED)?;
        Ok(RoleRow {
            id: row.int(ROLE_ID)?,
            name: row.text(ROLE_NAME)?.to_owned(),
        })
    })];
}

/// A row of `catalog::settings`, each duration in microseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingRow {
    StatementLimit(Option<i64>),
    TransactionLimit(Option<i64>),
    IdleLimit(Option<i64>),
    StreamIdleLimit(i64),
    RequestWindow(i64),
    SafePointDelay(i64),
    ReplicaWait(i64),
    DefaultLease(i64),
    FeedRetention(i64),
    StatsWindow(i64),
    StatsRetention(i64),
    WaitSampling { every: i64, keep: i64 },
    ConflictLog(i64),
    VacuumBudget { io: F64, cpu: F64 },
    ScrubInterval(Option<i64>),
    LeaseGrace(i64),
    BlockingNoticePeriod(i64),
    ReplicaVacuumWait(i64),
    RetryLimit(i64),
    RetentionWindow(i64),
}

const LIMIT: Fields = &["id", "limit"];
const KEEP: Fields = &["id", "keep"];
const DELAY: Fields = &["id", "delay"];
const WAIT: Fields = &["id", "wait"];
const LEASE: Fields = &["id", "lease"];
const LENGTH: Fields = &["id", "length"];
const SAMPLING: Fields = &["id", "every", "keep"];
const ROWS: Fields = &["id", "rows"];
const BUDGET: Fields = &["id", "io", "cpu"];
const EVERY: Fields = &["id", "every"];
const GRACE: Fields = &["id", "grace"];
const PERIOD: Fields = &["id", "period"];
const RETRIES: Fields = &["id", "retries"];

const LIMIT_F: Field = field(LIMIT, "limit");
const KEEP_F: Field = field(KEEP, "keep");
const DELAY_F: Field = field(DELAY, "delay");
const WAIT_F: Field = field(WAIT, "wait");
const LEASE_F: Field = field(LEASE, "lease");
const LENGTH_F: Field = field(LENGTH, "length");
const SAMPLING_EVERY: Field = field(SAMPLING, "every");
const SAMPLING_KEEP: Field = field(SAMPLING, "keep");
const ROWS_F: Field = field(ROWS, "rows");
const IO: Field = field(BUDGET, "io");
const CPU: Field = field(BUDGET, "cpu");
const EVERY_F: Field = field(EVERY, "every");
const GRACE_F: Field = field(GRACE, "grace");
const PERIOD_F: Field = field(PERIOD, "period");
const RETRIES_F: Field = field(RETRIES, "retries");

impl Decode for SettingRow {
    const RELATION: CatRelId = engine_rel::SETTINGS;
    const CTORS: &'static [Ctor<SettingRow>] = {
        use SettingRow as S;
        &[
            Ctor::new(k::SETTING_STATEMENT_LIMIT, LIMIT, |r| {
                Ok(S::StatementLimit(r.optional_duration(LIMIT_F)?))
            }),
            Ctor::new(k::SETTING_TRANSACTION_LIMIT, LIMIT, |r| {
                Ok(S::TransactionLimit(r.optional_duration(LIMIT_F)?))
            }),
            Ctor::new(k::SETTING_IDLE_LIMIT, LIMIT, |r| {
                Ok(S::IdleLimit(r.optional_duration(LIMIT_F)?))
            }),
            Ctor::new(k::SETTING_STREAM_IDLE_LIMIT, LIMIT, |r| {
                Ok(S::StreamIdleLimit(r.duration(LIMIT_F)?))
            }),
            Ctor::new(k::SETTING_REQUEST_WINDOW, KEEP, |r| {
                Ok(S::RequestWindow(r.duration(KEEP_F)?))
            }),
            Ctor::new(k::SETTING_SAFE_POINT_DELAY, DELAY, |r| {
                Ok(S::SafePointDelay(r.duration(DELAY_F)?))
            }),
            Ctor::new(k::SETTING_REPLICA_WAIT, WAIT, |r| {
                Ok(S::ReplicaWait(r.duration(WAIT_F)?))
            }),
            Ctor::new(k::SETTING_DEFAULT_LEASE, LEASE, |r| {
                Ok(S::DefaultLease(r.duration(LEASE_F)?))
            }),
            Ctor::new(k::SETTING_FEED_RETENTION, KEEP, |r| {
                Ok(S::FeedRetention(r.duration(KEEP_F)?))
            }),
            Ctor::new(k::SETTING_STATS_WINDOW, LENGTH, |r| {
                Ok(S::StatsWindow(r.duration(LENGTH_F)?))
            }),
            Ctor::new(k::SETTING_STATS_RETENTION, KEEP, |r| {
                Ok(S::StatsRetention(r.duration(KEEP_F)?))
            }),
            Ctor::new(k::SETTING_WAIT_SAMPLING, SAMPLING, |r| {
                Ok(S::WaitSampling {
                    every: r.duration(SAMPLING_EVERY)?,
                    keep: r.duration(SAMPLING_KEEP)?,
                })
            }),
            Ctor::new(k::SETTING_CONFLICT_LOG, ROWS, |r| {
                Ok(S::ConflictLog(r.int(ROWS_F)?))
            }),
            Ctor::new(k::SETTING_VACUUM_BUDGET, BUDGET, |r| {
                Ok(S::VacuumBudget {
                    io: r.float(IO)?,
                    cpu: r.float(CPU)?,
                })
            }),
            Ctor::new(k::SETTING_SCRUB_INTERVAL, EVERY, |r| {
                Ok(S::ScrubInterval(r.optional_duration(EVERY_F)?))
            }),
            Ctor::new(k::SETTING_LEASE_GRACE, GRACE, |r| {
                Ok(S::LeaseGrace(r.duration(GRACE_F)?))
            }),
            Ctor::new(k::SETTING_BLOCKING_NOTICE_PERIOD, PERIOD, |r| {
                Ok(S::BlockingNoticePeriod(r.duration(PERIOD_F)?))
            }),
            Ctor::new(k::SETTING_REPLICA_VACUUM_WAIT, WAIT, |r| {
                Ok(S::ReplicaVacuumWait(r.duration(WAIT_F)?))
            }),
            Ctor::new(k::SETTING_RETRY_LIMIT, RETRIES, |r| {
                Ok(S::RetryLimit(r.int(RETRIES_F)?))
            }),
            Ctor::new(k::SETTING_RETENTION_WINDOW, LENGTH, |r| {
                Ok(S::RetentionWindow(r.duration(LENGTH_F)?))
            }),
        ]
    };
}
