//! Results and loads as Arrow record batches: columns in Arrow's physical layout under a schema
//! of named fields, each sum carrying its constructor ids.

use std::sync::Arc;

use adtb_lang::schema::Schema;
use adtb_lang::types::Ty;

use crate::batch::{self, Batch};
use crate::catalog::layout::{self, batch_type, is_record, repr};
use crate::exec::types::ctor_set;
use crate::exec::{self, Val};

use super::error::{Error, RejectReason};
use super::text::DynRows;

/// The rows of one batch of a result stream.
const BATCH_ROWS: usize = 1024;

/// A field of an Arrow schema: its name and the type of its column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub ty: batch::Type,
}

/// The fields of the record batches of one result or one load, in column order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArrowSchema {
    pub fields: Vec<Field>,
}

impl ArrowSchema {
    /// Returns the types of the columns, in order.
    pub fn batch_schema(&self) -> batch::Schema {
        batch::Schema::new(self.fields.iter().map(|f| f.ty.clone()).collect())
    }
}

/// Rows as one column per field of an Arrow schema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordBatch {
    schema: Arc<ArrowSchema>,
    batch: Batch,
}

impl RecordBatch {
    /// Returns the rows of `batch` under `schema`, or `None` unless the batch's column types are
    /// the types of the schema's fields, in order.
    pub fn new(schema: Arc<ArrowSchema>, batch: Batch) -> Option<RecordBatch> {
        (*batch.schema() == schema.batch_schema()).then_some(RecordBatch { schema, batch })
    }

    pub fn schema(&self) -> &Arc<ArrowSchema> {
        &self.schema
    }

    pub fn batch(&self) -> &Batch {
        &self.batch
    }

    pub fn num_rows(&self) -> usize {
        self.batch.len()
    }
}

/// A result as record batches of one schema.
#[derive(Debug)]
pub struct ArrowStream {
    schema: Arc<ArrowSchema>,
    batches: std::vec::IntoIter<RecordBatch>,
}

impl ArrowStream {
    pub fn schema(&self) -> Arc<ArrowSchema> {
        self.schema.clone()
    }
}

impl Iterator for ArrowStream {
    type Item = Result<RecordBatch, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        self.batches.next().map(Ok)
    }
}

/// Returns `rows` as record batches over `schema`: one column per field of a record element,
/// one column `row` of a sum element of two or more constructors, and one column `value` of
/// any other element.
pub(super) fn stream(schema: &Schema, rows: DynRows) -> Result<ArrowStream, Error> {
    let elem = rows.ty().clone();
    let (fields, by) = columns(schema, &elem)?;
    let arrow = Arc::new(ArrowSchema { fields });
    let vals: Vec<Val> = rows.collect::<Result<_, _>>()?;
    let batches = exec::batches(
        &Val::List(Arc::from(vals)),
        &arrow.batch_schema(),
        by,
        BATCH_ROWS,
    )
    .map_err(|e| internal(&format!("{e:?}")))?;
    let batches: Vec<RecordBatch> = batches
        .into_iter()
        .map(|batch| RecordBatch {
            schema: arrow.clone(),
            batch,
        })
        .collect();
    Ok(ArrowStream {
        schema: arrow,
        batches: batches.into_iter(),
    })
}

/// Returns the fields of the columns of a result whose elements are of type `elem`, and how
/// an element fills them.
fn columns(schema: &Schema, elem: &Ty) -> Result<(Vec<Field>, exec::Rows), Error> {
    let t = repr(schema, elem);
    let column = |name: &str, ty: &Ty| -> Result<Field, Error> {
        Ok(Field {
            name: name.to_owned(),
            ty: batch_type(schema, ty)
                .ok_or_else(|| internal(&format!("a result of type {ty:?} has no column")))?,
        })
    };
    let fields: Option<Vec<(String, Ty)>> = match (&t, ctor_set(schema, &t).as_slice()) {
        (Ty::Record(fs), _) => Some(fs.clone()),
        (t, _) if is_record(schema, t) => Some(layout::ctors(schema, t).swap_remove(0).1),
        (Ty::Data { .. }, &[c]) => Some(layout::ctors(schema, &t).swap_remove(c as usize).1),
        _ => None,
    };
    if let Some(fs) = fields {
        let fields = fs
            .iter()
            .map(|(n, f)| column(n, f))
            .collect::<Result<_, _>>()?;
        return Ok((fields, exec::Rows::Fields));
    }
    let named = if is_sum(schema, &t) { "row" } else { "value" };
    Ok((vec![column(named, elem)?], exec::Rows::Whole))
}

/// Returns whether `t`, a type no newtype wraps, is a sum of two or more constructors other
/// than `Bool` and `Option`.
fn is_sum(schema: &Schema, t: &Ty) -> bool {
    match t {
        Ty::Data { def, .. } => {
            let prelude = [schema.prelude_type("Bool"), schema.prelude_type("Option")];
            !prelude.contains(def) && schema.ty(*def).ctors().len() >= 2
        }
        Ty::Sum { ctors, .. } => ctors.len() >= 2,
        _ => false,
    }
}

fn internal(what: &str) -> Error {
    Error::rejected(RejectReason::Internal {
        what: what.to_owned(),
    })
}
