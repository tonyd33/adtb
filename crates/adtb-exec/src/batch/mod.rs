//! Hold Arrow batches: aligned buffers, columns and their types.

mod buffer;
mod column;
pub mod ipc;
mod types;

#[cfg(test)]
mod tests;

pub use buffer::{ALIGN, Buffer};
pub use column::Column;
pub use types::{Ctor, Datum, Shape, Type, UNION_LIMIT, shape, tag_width};

use crate::heap::FieldType;
use crate::key::Value;

use types::Cell;

/// A push that would put an offset of a batch column at 2^31 or past, beyond what Arrow's `i32`
/// offsets address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooLarge;

/// The types of a batch's columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schema {
    fields: Vec<Type>,
}

impl Schema {
    pub fn new(fields: Vec<Type>) -> Schema {
        Schema { fields }
    }

    /// Returns the schema of the batches of a heap whose rows have fields `row`.
    pub fn of_heap(row: &[FieldType]) -> Schema {
        Schema::new(row.iter().map(Type::from).collect())
    }

    pub fn fields(&self) -> &[Type] {
        &self.fields
    }
}

/// Rows held as one [`Column`] per field of a [`Schema`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Batch {
    schema: Schema,
    len: usize,
    columns: Vec<Column>,
}

impl Batch {
    /// Returns a batch of `rows`, each a value per field of `schema`.
    ///
    /// Panics:
    /// - A row is not of the schema's types or holds a list.
    /// - A column passes 2^31 elements or value bytes.
    pub fn from_rows<'a>(schema: Schema, rows: impl IntoIterator<Item = &'a [Value]>) -> Batch {
        let mut b = BatchBuilder::new(schema);
        for r in rows {
            b.push_row(r)
                .expect("a batch from rows holds fewer than 2^31 elements or bytes per column");
        }
        b.finish()
    }

    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Returns the bytes the batch's columns hold.
    pub fn bytes(&self) -> usize {
        self.columns.iter().map(Column::bytes).sum()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    pub fn into_columns(self) -> Vec<Column> {
        self.columns
    }

    /// Returns field `field` of row `row`.
    ///
    /// Panics:
    /// - The field's type has a list.
    pub fn value(&self, field: usize, row: usize) -> Value {
        self.columns[field].get(&self.schema.fields[field], row)
    }

    /// Returns field `field` of row `row`.
    pub fn datum(&self, field: usize, row: usize) -> Datum {
        self.columns[field].get(&self.schema.fields[field], row)
    }

    /// Returns row `row`.
    ///
    /// Panics:
    /// - A field's type has a list.
    pub fn row(&self, row: usize) -> Vec<Value> {
        (0..self.columns.len())
            .map(|f| self.value(f, row))
            .collect()
    }

    /// Returns row `row`.
    pub fn datums(&self, row: usize) -> Vec<Datum> {
        (0..self.columns.len())
            .map(|f| self.datum(f, row))
            .collect()
    }
}

/// A batch under construction, appended to row by row.
#[derive(Debug)]
pub struct BatchBuilder {
    batch: Batch,
}

impl BatchBuilder {
    pub fn new(schema: Schema) -> BatchBuilder {
        let columns = schema.fields.iter().map(Column::new).collect();
        BatchBuilder {
            batch: Batch {
                schema,
                len: 0,
                columns,
            },
        }
    }

    pub fn len(&self) -> usize {
        self.batch.len
    }

    pub fn is_empty(&self) -> bool {
        self.batch.len == 0
    }

    /// Returns the bytes the columns of the batch built so far hold.
    pub fn bytes(&self) -> usize {
        self.batch.bytes()
    }

    /// Append `row`, a value per field of the schema, as a heap returns it.
    ///
    /// Errors:
    /// - [`TooLarge`] when a column would pass 2^31 elements or value bytes; the builder then
    ///   holds what it held before.
    ///
    /// Panics:
    /// - `row` is not of the schema's types or holds a list.
    pub fn push_row(&mut self, row: &[Value]) -> Result<(), TooLarge> {
        self.push(row)
    }

    /// Append `row`, a datum per field of the schema.
    ///
    /// Errors:
    /// - [`TooLarge`] as [`push_row`](BatchBuilder::push_row).
    ///
    /// Panics:
    /// - `row` is not of the schema's types.
    pub fn push_datums(&mut self, row: &[Datum]) -> Result<(), TooLarge> {
        self.push(row)
    }

    /// Append `row` as [`push_row`](BatchBuilder::push_row) does. Returns the rows held before
    /// it as a finished batch, `row` then starting the next, when `row` would take a column of
    /// theirs past 2^31 elements or value bytes.
    ///
    /// Errors:
    /// - [`TooLarge`] when a column of `row` alone passes 2^31 elements or value bytes; the
    ///   builder then holds what it held before.
    pub fn push_row_or_cut(&mut self, row: &[Value]) -> Result<Option<Batch>, TooLarge> {
        self.push_or_cut(row)
    }

    /// Append `row` as [`push_datums`](BatchBuilder::push_datums) does, cutting the batch as
    /// [`push_row_or_cut`](BatchBuilder::push_row_or_cut) does.
    ///
    /// Errors:
    /// - [`TooLarge`] as [`push_row_or_cut`](BatchBuilder::push_row_or_cut).
    pub fn push_datums_or_cut(&mut self, row: &[Datum]) -> Result<Option<Batch>, TooLarge> {
        self.push_or_cut(row)
    }

    fn push_or_cut<C: Cell>(&mut self, row: &[C]) -> Result<Option<Batch>, TooLarge> {
        match self.push(row) {
            Err(TooLarge) if !self.is_empty() => {
                let mut next = BatchBuilder::new(self.batch.schema.clone());
                next.push(row)?;
                Ok(Some(std::mem::replace(self, next).finish()))
            }
            r => r.map(|()| None),
        }
    }

    fn push<C: Cell>(&mut self, row: &[C]) -> Result<(), TooLarge> {
        let b = &mut self.batch;
        assert_eq!(b.schema.fields.len(), row.len(), "a row of the wrong arity");
        for ((col, t), v) in b.columns.iter_mut().zip(&b.schema.fields).zip(row) {
            if let Err(e) = col.push(t, v) {
                for c in &mut b.columns {
                    c.truncate(b.len);
                }
                return Err(e);
            }
        }
        b.len += 1;
        Ok(())
    }

    pub fn finish(self) -> Batch {
        self.batch
    }
}
