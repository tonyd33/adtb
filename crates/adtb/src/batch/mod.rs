mod buffer;
mod column;
mod types;

#[cfg(test)]
mod tests;

pub use buffer::{ALIGN, Buffer};
pub use column::Column;
pub use types::{Ctor, Datum, Shape, Type, UNION_LIMIT, shape, tag_width};

use crate::heap::FieldType;
use crate::key::Value;

use types::Cell;

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
    /// Panics if a row is not of the schema's types or holds a list.
    pub fn from_rows<'a>(schema: Schema, rows: impl IntoIterator<Item = &'a [Value]>) -> Batch {
        let mut b = BatchBuilder::new(schema);
        rows.into_iter().for_each(|r| b.push_row(r));
        b.finish()
    }

    pub fn schema(&self) -> &Schema {
        &self.schema
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
    /// Panics if the field's type has a list.
    pub fn value(&self, field: usize, row: usize) -> Value {
        self.columns[field].get(&self.schema.fields[field], row)
    }

    /// Returns field `field` of row `row`.
    pub fn datum(&self, field: usize, row: usize) -> Datum {
        self.columns[field].get(&self.schema.fields[field], row)
    }

    /// Returns row `row`.
    ///
    /// Panics if a field's type has a list.
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
        self.batch.columns.iter().map(Column::bytes).sum()
    }

    /// Append `row`, a value per field of the schema, as a heap returns it.
    ///
    /// Panics if `row` is not of the schema's types or holds a list.
    pub fn push_row(&mut self, row: &[Value]) {
        self.push(row);
    }

    /// Append `row`, a datum per field of the schema.
    ///
    /// Panics if `row` is not of the schema's types.
    pub fn push_datums(&mut self, row: &[Datum]) {
        self.push(row);
    }

    fn push<C: Cell>(&mut self, row: &[C]) {
        let b = &mut self.batch;
        assert_eq!(b.schema.fields.len(), row.len(), "a row of the wrong arity");
        for ((col, t), v) in b.columns.iter_mut().zip(&b.schema.fields).zip(row) {
            col.push(t, v);
        }
        b.len += 1;
    }

    pub fn finish(self) -> Batch {
        self.batch
    }
}
