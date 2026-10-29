//! The results of a task: their layout, the batches the transport takes, and the completion.

use std::sync::Arc;

use adtb_lang::schema::Schema;
use adtb_lang::textual::Form;
use adtb_lang::types::Ty;

use crate::exec::{self, Arena, Val};

use super::super::arrow::{self, ArrowSchema, RecordBatch};
use super::super::{Error, Position};

/// The form a task's result batches take.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    #[default]
    Rows,
    Arrow,
}

/// The type and form of one result's elements, which every batch of the result shares.
#[derive(Debug)]
pub struct Layout {
    ty: Ty,
    form: Form,
    arrow: Option<(Arc<ArrowSchema>, exec::Rows)>,
}

impl Layout {
    pub(super) fn new(schema: &Schema, ty: Ty, format: Format) -> Result<Layout, Error> {
        let arrow = match format {
            Format::Rows => None,
            Format::Arrow => Some(arrow::layout(schema, &ty)?),
        };
        Ok(Layout {
            form: Form::of(schema, &ty),
            ty,
            arrow,
        })
    }

    /// Returns the type of each element.
    pub fn ty(&self) -> &Ty {
        &self.ty
    }

    /// Returns the form the textual syntax prints each element at.
    pub fn form(&self) -> &Form {
        &self.form
    }

    /// Returns the schema of the record batches of a task submitted for Arrow.
    pub fn arrow(&self) -> Option<&Arc<ArrowSchema>> {
        self.arrow.as_ref().map(|(s, _)| s)
    }

    /// Returns `rows` as one batch of this layout, charged to `held`.
    pub(super) fn batch(&self, rows: Vec<Val>, held: Arena) -> Result<ResultBatch, Error> {
        let len = rows.len();
        let body = match &self.arrow {
            None => Body::Rows(rows),
            Some((schema, by)) => Body::Arrow(arrow::record_batch(schema, *by, rows)?),
        };
        Ok(ResultBatch { body, len, held })
    }
}

#[derive(Debug)]
enum Body {
    Rows(Vec<Val>),
    Arrow(RecordBatch),
}

/// Elements of a result handed to the transport. It holds a reservation of its call's memory
/// grant until it is released or dropped.
#[derive(Debug)]
pub struct ResultBatch {
    body: Body,
    len: usize,
    held: Arena,
}

impl ResultBatch {
    /// Returns the number of elements.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the bytes of the grant the batch holds.
    pub fn bytes(&self) -> usize {
        self.held.bytes()
    }

    /// Returns the elements of a batch of a task submitted for rows.
    pub fn rows(&self) -> Option<&[Val]> {
        match &self.body {
            Body::Rows(rows) => Some(rows),
            Body::Arrow(_) => None,
        }
    }

    /// Returns the record batch of a task submitted for Arrow.
    pub fn record_batch(&self) -> Option<&RecordBatch> {
        match &self.body {
            Body::Arrow(b) => Some(b),
            Body::Rows(_) => None,
        }
    }

    /// Return the batch's reservation to its grant.
    pub fn release(self) {}
}

/// How a task ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Completion {
    /// A call that only read, at the position of its snapshot.
    Read {
        snapshot: Position,
    },
    /// A call that committed, after `attempts` attempts. `incomplete` when its output was
    /// abandoned before every batch was delivered.
    Committed {
        position: Position,
        attempts: u32,
        incomplete: bool,
    },
    Failed(Error),
}
