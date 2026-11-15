//! Catalog rows as typed values: one codec per catalog relation, reading a row's constructor by
//! its fixed id and each field by its declared name.

use std::marker::PhantomData;

use adtb_lang::ids::{engine_ctor as k, engine_rel};
use adtb_lang::schema::Schema;
use adtb_lang::schema::prelude::ctor;
use adtb_lang::types::RelId;

use crate::exec::{self, Val};
use crate::heap::FieldType;
use crate::key::{F64, Scalar, Value};

use super::Rot;
use super::ids::{CatCtorId, CatRelId, ctor_id, relation};
use super::layout;

#[cfg(test)]
#[path = "codec_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "codec_testing.rs"]
pub(crate) mod testing;

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

/// The fixed id of a constructor of a sum with the stored types of its fields.
pub type Payload = (CatCtorId, &'static [Scalar]);

/// A field of a constructor whose type is a sum, with each of its constructors.
#[derive(Clone, Copy, Debug)]
pub struct Sum {
    field: Field,
    ctors: &'static [Payload],
}

/// Returns sum field `field`, whose type's constructors are `ctors` in any order.
pub const fn sum(field: Field, ctors: &'static [Payload]) -> Sum {
    Sum { field, ctors }
}

/// One constructor a codec reads: its fixed id, its fields, the sum fields it reads, and how a
/// row of it reads.
pub struct Ctor<T> {
    id: CatCtorId,
    fields: Fields,
    sums: &'static [Sum],
    read: fn(&Row<'_>) -> Result<T, Rot>,
}

impl<T> Ctor<T> {
    pub const fn new(id: CatCtorId, fields: Fields, read: fn(&Row<'_>) -> Result<T, Rot>) -> Self {
        Ctor {
            id,
            fields,
            sums: &[],
            read,
        }
    }

    /// Returns this constructor reading the sum fields `sums` by [`Row::ctor`].
    pub const fn with_sums(self, sums: &'static [Sum]) -> Self {
        Ctor { sums, ..self }
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
    /// For each field of `ctor.fields` that `ctor.sums` lists, the fixed id of each constructor
    /// of its type by position.
    sums: Vec<Option<Vec<CatCtorId>>>,
    /// The stored type of each field of `ctor.fields`, in its order.
    types: Vec<FieldType>,
}

impl<T: Decode> Codec<T> {
    /// Returns the codec of `T` over `schema`.
    ///
    /// Preconditions: `schema` declares `T::RELATION` with exactly the constructors and fields
    /// `T::CTORS` lists, fields in any order, and each sum field a constructor lists with
    /// exactly the constructors it lists.
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
                let declared_fields = layout::row(schema, r, c);
                let stored: Vec<String> = declared_fields.iter().map(|(n, _)| n.clone()).collect();
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
                    .collect::<Vec<usize>>();
                let mut sums = vec![None; ctor.fields.len()];
                for s in ctor.sums {
                    assert!(
                        s.field.fields == ctor.fields,
                        "the codec of {name} lists a sum field of constructor {id} among its fields"
                    );
                    let field = s.field.fields[s.field.at];
                    let FieldType::Sum(cs) =
                        layout::field_type(schema, &declared_fields[positions[s.field.at]].1)
                    else {
                        panic!("field `{field}` of constructor {id} of {name} is a sum");
                    };
                    let table: Vec<CatCtorId> = cs
                        .iter()
                        .map(|c| CatCtorId::from_raw(u64::from(c.id.0)))
                        .collect();
                    for c in &cs {
                        let id = CatCtorId::from_raw(u64::from(c.id.0));
                        let listed = s.ctors.iter().find(|(k, _)| *k == id).map(|(_, p)| {
                            p.iter().map(|s| FieldType::Scalar(*s)).collect::<Vec<_>>()
                        });
                        assert!(
                            listed.is_none_or(|p| p == c.fields),
                            "the codec of {name} lists the payload of constructor {id} of field `{field}`"
                        );
                    }
                    let mut want: Vec<CatCtorId> = s.ctors.iter().map(|(k, _)| *k).collect();
                    let mut have = table.clone();
                    want.sort_unstable();
                    have.sort_unstable();
                    assert_eq!(
                        want, have,
                        "the codec of {name} lists the constructors of field `{field}` of constructor {id}"
                    );
                    sums[s.field.at] = Some(table);
                }
                let types = positions
                    .iter()
                    .map(|&at| layout::field_type(schema, &declared_fields[at].1))
                    .collect();
                Shape {
                    ctor,
                    name: declared[c as usize].name.clone(),
                    positions,
                    sums,
                    types,
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
    /// holds another count of fields than its constructor, a field holds a value not of its
    /// declared type, or the constructor's reader refuses a value.
    pub fn decode(&self, ctor: u32, values: &[Value]) -> Result<T, Rot> {
        let shape = self.shape(ctor)?;
        let row = Row {
            relation: &self.name,
            name: &shape.name,
            ctor: shape.ctor.id,
            fields: shape.ctor.fields,
            positions: &shape.positions,
            sums: &shape.sums,
            values,
        };
        if values.len() != shape.positions.len() {
            return Err(row.refused(counted(values.len(), shape.positions.len())));
        }
        for (at, t) in shape.types.iter().enumerate() {
            let f = Field {
                fields: shape.ctor.fields,
                at,
            };
            if !fits(t, row.get(f)) {
                return Err(row.bad(f));
            }
        }
        (shape.ctor.read)(&row)
    }

    /// Returns the typed row of `v`, a value of the relation's element type.
    ///
    /// Fail as [`Codec::decode`] does, and when `v` is not a constructor's value, holds another
    /// count of fields than its constructor, or a field of it has no stored form of its type.
    pub fn decode_val(&self, schema: &Schema, v: &Val) -> Result<T, Rot> {
        let Val::Ctor(c, fields) = v else {
            return Err(Rot::CatalogRow {
                row: self.name.clone(),
                detail: format!("is {v:?}"),
            });
        };
        let shape = self.shape(*c)?;
        let declared = layout::row(schema, self.relation, *c);
        if fields.len() != declared.len() {
            return Err(Rot::CatalogRow {
                row: format!("{}.{}", self.name, shape.name),
                detail: counted(fields.len(), declared.len()),
            });
        }
        let stored = declared
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

/// Returns the detail of a row holding `held` fields where its constructor declares `declared`.
fn counted(held: usize, declared: usize) -> String {
    let fields = if held == 1 { "field" } else { "fields" };
    format!("holds {held} {fields}, not {declared}")
}

/// Returns whether `v` is a value of stored type `t`.
fn fits(t: &FieldType, v: &Value) -> bool {
    match (t, v) {
        (FieldType::Scalar(s), v) => scalar_fits(*s, v),
        (FieldType::Record(ts), Value::Record(vs)) => all_fit(ts, vs),
        (FieldType::Sum(cs), Value::Sum { ctor, fields }) => cs
            .get(*ctor as usize)
            .is_some_and(|c| all_fit(&c.fields, fields)),
        _ => false,
    }
}

fn all_fit(ts: &[FieldType], vs: &[Value]) -> bool {
    ts.len() == vs.len() && ts.iter().zip(vs).all(|(t, v)| fits(t, v))
}

fn scalar_fits(s: Scalar, v: &Value) -> bool {
    matches!(
        (s, v),
        (Scalar::I8, Value::I8(_))
            | (Scalar::I16, Value::I16(_))
            | (Scalar::I32, Value::I32(_))
            | (Scalar::I64, Value::I64(_))
            | (Scalar::F32, Value::F32(_))
            | (Scalar::F64, Value::F64(_))
            | (Scalar::Decimal(_), Value::Decimal(_))
            | (Scalar::Text, Value::Text(_))
            | (Scalar::Bytes, Value::Bytes(_))
            | (Scalar::SortKey, Value::SortKey(_))
            | (Scalar::Instant, Value::Instant(_))
            | (Scalar::Date, Value::Date(_))
            | (Scalar::Time, Value::Time(_))
            | (Scalar::DateTime, Value::DateTime(_))
            | (Scalar::ZonedDateTime, Value::ZonedDateTime { .. })
            | (Scalar::Duration, Value::Duration(_))
            | (Scalar::Period, Value::Period { .. })
    )
}

/// One stored row of a catalog relation, its fields read by [`Field`].
pub struct Row<'a> {
    relation: &'a str,
    name: &'a str,
    ctor: CatCtorId,
    fields: Fields,
    positions: &'a [usize],
    sums: &'a [Option<Vec<CatCtorId>>],
    values: &'a [Value],
}

impl Row<'_> {
    /// Returns the refusal of this row as damage, `detail` saying why.
    pub fn refused(&self, detail: String) -> Rot {
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

    /// Returns the fixed id of the row's constructor.
    pub fn ctor_id(&self) -> CatCtorId {
        self.ctor
    }

    /// Returns the refusal of the value field `f` holds.
    pub fn bad(&self, f: Field) -> Rot {
        self.refused(format!(
            "holds {:?} in field `{}`",
            self.get(f),
            f.fields[f.at]
        ))
    }

    /// Abort on field `f` holding another type than its reader takes, which `decode` refuses first.
    fn unchecked(&self, f: Field) -> ! {
        panic!(
            "decode checks field `{}` of {}.{} against its stored type before a reader reads it",
            f.fields[f.at], self.relation, self.name
        )
    }

    pub fn int(&self, f: Field) -> i64 {
        match self.get(f) {
            Value::I64(n) => *n,
            _ => self.unchecked(f),
        }
    }

    /// Returns the integer in field `f`, an `I32` widened.
    pub fn int32(&self, f: Field) -> i64 {
        match self.get(f) {
            Value::I32(n) => i64::from(*n),
            _ => self.unchecked(f),
        }
    }

    pub fn bytes(&self, f: Field) -> &[u8] {
        match self.get(f) {
            Value::Bytes(b) => b,
            _ => self.unchecked(f),
        }
    }

    /// Returns the truth of the `Bool` in field `f`.
    pub fn flag(&self, f: Field) -> bool {
        match self.get(f) {
            Value::Sum { ctor, .. } => *ctor == ctor::TRUE,
            _ => self.unchecked(f),
        }
    }

    /// Returns the fixed id of the constructor sum field `s` holds, with that constructor's
    /// fields, of the types `s` lists for it.
    pub fn ctor(&self, s: Sum) -> (CatCtorId, &[Value]) {
        let table = self.sums[s.field.at]
            .as_ref()
            .expect("a codec reads a sum field its constructor lists");
        match self.get(s.field) {
            Value::Sum { ctor, fields } => (table[*ctor as usize], fields.as_slice()),
            _ => self.unchecked(s.field),
        }
    }

    pub fn text(&self, f: Field) -> &str {
        match self.get(f) {
            Value::Text(t) => t,
            _ => self.unchecked(f),
        }
    }

    pub fn instant(&self, f: Field) -> i64 {
        match self.get(f) {
            Value::Instant(t) => *t,
            _ => self.unchecked(f),
        }
    }

    /// Returns the duration in field `f`, in microseconds.
    pub fn duration(&self, f: Field) -> i64 {
        match self.get(f) {
            Value::Duration(d) => *d,
            _ => self.unchecked(f),
        }
    }

    pub fn float(&self, f: Field) -> F64 {
        match self.get(f) {
            Value::F64(x) => *x,
            _ => self.unchecked(f),
        }
    }

    /// Returns the duration an `Option Duration` field holds, in microseconds.
    pub fn optional_duration(&self, f: Field) -> Option<i64> {
        self.optional(f, |v| match v {
            Value::Duration(d) => Some(*d),
            _ => None,
        })
    }

    /// Returns the integer an `Option I64` field holds.
    pub fn optional_int(&self, f: Field) -> Option<i64> {
        self.optional(f, |v| match v {
            Value::I64(n) => Some(*n),
            _ => None,
        })
    }

    fn optional<T>(&self, f: Field, inner: impl Fn(&Value) -> Option<T>) -> Option<T> {
        match self.get(f) {
            Value::Sum {
                ctor: ctor::SOME,
                fields,
            } => Some(inner(&fields[0]).unwrap_or_else(|| self.unchecked(f))),
            Value::Sum { .. } => None,
            _ => self.unchecked(f),
        }
    }
}

/// Returns the instant of `payload`, the fields of a sum constructor whose [`Sum`] lists them as
/// one `Instant`.
pub fn payload_instant(payload: &[Value]) -> i64 {
    match payload {
        [Value::Instant(t)] => *t,
        _ => {
            panic!("Codec::new checks a sum constructor's payload against the types its Sum lists")
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

impl Decode for RoleRow {
    const RELATION: CatRelId = engine_rel::ROLES;
    const CTORS: &'static [Ctor<RoleRow>] = &[Ctor::new(k::ROLE, ROLE, |row| {
        Ok(RoleRow {
            id: row.int(ROLE_ID),
            name: row.text(ROLE_NAME).to_owned(),
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

const BUDGET_ID: Field = field(BUDGET, "id");
const DELAY_ID: Field = field(DELAY, "id");
const EVERY_ID: Field = field(EVERY, "id");
const GRACE_ID: Field = field(GRACE, "id");
const KEEP_ID: Field = field(KEEP, "id");
const LEASE_ID: Field = field(LEASE, "id");
const LENGTH_ID: Field = field(LENGTH, "id");
const LIMIT_ID: Field = field(LIMIT, "id");
const PERIOD_ID: Field = field(PERIOD, "id");
const RETRIES_ID: Field = field(RETRIES, "id");
const ROWS_ID: Field = field(ROWS, "id");
const SAMPLING_ID: Field = field(SAMPLING, "id");
const WAIT_ID: Field = field(WAIT, "id");

/// Returns `setting`, once the header field `id` of `r` holds its constructor's catalog id.
fn keyed(r: &Row<'_>, id: Field, setting: SettingRow) -> Result<SettingRow, Rot> {
    if u64::try_from(r.int(id)) == Ok(r.ctor_id().raw()) {
        Ok(setting)
    } else {
        Err(r.bad(id))
    }
}

impl Decode for SettingRow {
    const RELATION: CatRelId = engine_rel::SETTINGS;
    const CTORS: &'static [Ctor<SettingRow>] = {
        use SettingRow as S;
        &[
            Ctor::new(k::SETTING_STATEMENT_LIMIT, LIMIT, |r| {
                keyed(r, LIMIT_ID, S::StatementLimit(r.optional_duration(LIMIT_F)))
            }),
            Ctor::new(k::SETTING_TRANSACTION_LIMIT, LIMIT, |r| {
                keyed(
                    r,
                    LIMIT_ID,
                    S::TransactionLimit(r.optional_duration(LIMIT_F)),
                )
            }),
            Ctor::new(k::SETTING_IDLE_LIMIT, LIMIT, |r| {
                keyed(r, LIMIT_ID, S::IdleLimit(r.optional_duration(LIMIT_F)))
            }),
            Ctor::new(k::SETTING_STREAM_IDLE_LIMIT, LIMIT, |r| {
                keyed(r, LIMIT_ID, S::StreamIdleLimit(r.duration(LIMIT_F)))
            }),
            Ctor::new(k::SETTING_REQUEST_WINDOW, KEEP, |r| {
                keyed(r, KEEP_ID, S::RequestWindow(r.duration(KEEP_F)))
            }),
            Ctor::new(k::SETTING_SAFE_POINT_DELAY, DELAY, |r| {
                keyed(r, DELAY_ID, S::SafePointDelay(r.duration(DELAY_F)))
            }),
            Ctor::new(k::SETTING_REPLICA_WAIT, WAIT, |r| {
                keyed(r, WAIT_ID, S::ReplicaWait(r.duration(WAIT_F)))
            }),
            Ctor::new(k::SETTING_DEFAULT_LEASE, LEASE, |r| {
                keyed(r, LEASE_ID, S::DefaultLease(r.duration(LEASE_F)))
            }),
            Ctor::new(k::SETTING_FEED_RETENTION, KEEP, |r| {
                keyed(r, KEEP_ID, S::FeedRetention(r.duration(KEEP_F)))
            }),
            Ctor::new(k::SETTING_STATS_WINDOW, LENGTH, |r| {
                keyed(r, LENGTH_ID, S::StatsWindow(r.duration(LENGTH_F)))
            }),
            Ctor::new(k::SETTING_STATS_RETENTION, KEEP, |r| {
                keyed(r, KEEP_ID, S::StatsRetention(r.duration(KEEP_F)))
            }),
            Ctor::new(k::SETTING_WAIT_SAMPLING, SAMPLING, |r| {
                keyed(
                    r,
                    SAMPLING_ID,
                    S::WaitSampling {
                        every: r.duration(SAMPLING_EVERY),
                        keep: r.duration(SAMPLING_KEEP),
                    },
                )
            }),
            Ctor::new(k::SETTING_CONFLICT_LOG, ROWS, |r| {
                keyed(r, ROWS_ID, S::ConflictLog(r.int(ROWS_F)))
            }),
            Ctor::new(k::SETTING_VACUUM_BUDGET, BUDGET, |r| {
                keyed(
                    r,
                    BUDGET_ID,
                    S::VacuumBudget {
                        io: r.float(IO),
                        cpu: r.float(CPU),
                    },
                )
            }),
            Ctor::new(k::SETTING_SCRUB_INTERVAL, EVERY, |r| {
                keyed(r, EVERY_ID, S::ScrubInterval(r.optional_duration(EVERY_F)))
            }),
            Ctor::new(k::SETTING_LEASE_GRACE, GRACE, |r| {
                keyed(r, GRACE_ID, S::LeaseGrace(r.duration(GRACE_F)))
            }),
            Ctor::new(k::SETTING_BLOCKING_NOTICE_PERIOD, PERIOD, |r| {
                keyed(r, PERIOD_ID, S::BlockingNoticePeriod(r.duration(PERIOD_F)))
            }),
            Ctor::new(k::SETTING_REPLICA_VACUUM_WAIT, WAIT, |r| {
                keyed(r, WAIT_ID, S::ReplicaVacuumWait(r.duration(WAIT_F)))
            }),
            Ctor::new(k::SETTING_RETRY_LIMIT, RETRIES, |r| {
                keyed(r, RETRIES_ID, S::RetryLimit(r.int(RETRIES_F)))
            }),
            Ctor::new(k::SETTING_RETENTION_WINDOW, LENGTH, |r| {
                keyed(r, LENGTH_ID, S::RetentionWindow(r.duration(LENGTH_F)))
            }),
        ]
    };
}
