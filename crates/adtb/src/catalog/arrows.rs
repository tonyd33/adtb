//! The rows of a constructor relation as an Arrow IPC stream in the external type mapping.

use adtb_lang::schema::Schema;
use adtb_lang::types::{RelId, Ty};

use crate::batch::Batch;
use crate::batch::ipc::{
    self, Budget, FileCtor, FileField, FileSchema, FileType, IpcError, SumKind,
};
use crate::exec::{Val, conv, from_stored, to_stored};
use crate::key::{CtorId, Value};

use super::layout::{ctors, field_type, is_record, key_value_ty, repr, row, scalar};

/// Returns the columns of a row of constructor `ctor` of relation `rel`.
pub fn file_schema(schema: &Schema, rel: RelId, ctor: u32) -> FileSchema {
    FileSchema::new(fields(schema, &row(schema, rel, ctor)))
}

fn fields(schema: &Schema, fs: &[(String, Ty)]) -> Vec<FileField> {
    fs.iter()
        .map(|(name, ty)| FileField {
            name: name.clone(),
            ty: file_type(schema, ty),
        })
        .collect()
}

fn file_type(schema: &Schema, ty: &Ty) -> FileType {
    match repr(schema, ty) {
        Ty::Base(b) => FileType::Scalar(scalar(b)),
        Ty::Key { rel, .. } => file_type(schema, &key_value_ty(schema, rel)),
        Ty::Record(fs) => FileType::Record(fields(schema, &fs)),
        Ty::Unit => FileType::Record(Vec::new()),
        Ty::Set(e) | Ty::NonEmpty(e) => FileType::List(Box::new(file_type(schema, &e))),
        t if is_record(schema, &t) => FileType::Record(fields(schema, &ctors(schema, &t)[0].1)),
        t => FileType::Sum(
            kind(schema, &t),
            ctor_names(schema, &t)
                .into_iter()
                .zip(ctors(schema, &t))
                .map(|(name, (id, fs))| FileCtor {
                    id: CtorId(id),
                    name,
                    fields: fields(schema, &fs),
                })
                .collect(),
        ),
    }
}

/// Returns which sum `t` is, by the identity of its declaration.
fn kind(schema: &Schema, t: &Ty) -> SumKind {
    match t {
        Ty::Data { def, .. } if *def == schema.prelude_type("Bool") => SumKind::Bool,
        Ty::Data { def, .. } if *def == schema.prelude_type("Option") => SumKind::Option,
        _ => SumKind::Other,
    }
}

fn ctor_names(schema: &Schema, t: &Ty) -> Vec<String> {
    match repr(schema, t) {
        Ty::Tag(t) => ctor_names(schema, &t),
        Ty::Data { def, .. } => schema
            .ty(def)
            .ctors()
            .iter()
            .map(|c| c.name.clone())
            .collect(),
        Ty::Sum { ctors, .. } => ctors.into_iter().map(|(name, _)| name).collect(),
        t => panic!("{t:?} is not a sum"),
    }
}

/// The most [`weight`] one record batch of [`write_rows`] holds: any column's offsets stay
/// within `i32`.
pub const BATCH_LIMIT: usize = (1 << 31) - 1;

/// Returns `rows`, rows of constructor `ctor` of `rel`, as one Arrow IPC stream, a record batch
/// ending before the row that would take its [`weight`] past `limit`.
pub fn write_rows<'a>(
    schema: &Schema,
    rel: RelId,
    ctor: u32,
    rows: impl IntoIterator<Item = &'a Val>,
    limit: usize,
) -> Vec<u8> {
    let file = file_schema(schema, rel, ctor);
    let types: Vec<_> = row(schema, rel, ctor)
        .iter()
        .map(|(_, t)| field_type(schema, t))
        .collect();
    let stored: Vec<Vec<_>> = rows
        .into_iter()
        .map(|r| {
            types
                .iter()
                .zip(r.fields().expect("a row is a constructor's fields"))
                .map(|(t, v)| to_stored(t, v).expect("a stored row has its heap form"))
                .collect()
        })
        .collect();
    let batch = |rows: &[Vec<Value>]| {
        Batch::from_rows(file.schema().clone(), rows.iter().map(Vec::as_slice))
    };
    let (mut batches, mut start, mut used) = (Vec::new(), 0, 0);
    for (i, row) in stored.iter().enumerate() {
        let w = 1 + row.iter().map(weight).sum::<usize>();
        if i > start && used + w > limit {
            batches.push(batch(&stored[start..i]));
            (start, used) = (i, 0);
        }
        used += w;
    }
    batches.push(batch(&stored[start..]));
    let mut out = Vec::new();
    ipc::write(&file, &batches, &mut out).expect("a Vec takes any bytes");
    out
}

/// Returns at least what `v` adds to the elements or value bytes of any one column.
fn weight(v: &Value) -> usize {
    match v {
        Value::Text(t) => 1 + t.len(),
        Value::Bytes(b) | Value::SortKey(b) => 1 + b.len(),
        Value::ZonedDateTime { zone, .. } => 64 + zone.len(),
        Value::Period { .. } => 32,
        Value::Record(fs) | Value::Sum { fields: fs, .. } => {
            1 + fs.iter().map(weight).sum::<usize>()
        }
        _ => 1,
    }
}

/// Returns the most rows a stream of `len` bytes of `file`'s columns may hold: the rows hold
/// distinct keys, so a row of no bits admits one row, and any other row takes its bits.
pub fn row_budget(file: &FileSchema, len: usize) -> usize {
    match file.row_bits() {
        0 => 1,
        w => 8 * len / w,
    }
}

/// Returns the rows of constructor `ctor` of `rel` that the Arrow IPC stream `bytes` holds, at
/// most [`row_budget`].
pub fn read_rows(
    schema: &Schema,
    rel: RelId,
    ctor: u32,
    bytes: &[u8],
) -> Result<Vec<Val>, IpcError> {
    let file = file_schema(schema, rel, ctor);
    let convs: Vec<_> = row(schema, rel, ctor)
        .iter()
        .map(|(_, t)| conv(schema, t))
        .collect();
    let mut budget = Budget::new(row_budget(&file, bytes.len()));
    let batches = ipc::read(bytes, &file, &mut budget)?;
    Ok(batches
        .iter()
        .flat_map(|b| (0..b.len()).map(|i| b.row(i)))
        .map(|r| {
            Val::ctor(
                ctor,
                convs
                    .iter()
                    .zip(&r)
                    .map(|(c, v)| from_stored(c, v))
                    .collect(),
            )
        })
        .collect())
}

#[cfg(test)]
#[path = "arrows_tests.rs"]
mod tests;
