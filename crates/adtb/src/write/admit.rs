//! The admission of a change set's new rows to the row, value and key limits.

use adtb_lang::types::RelId;

use crate::heap;
use crate::runtime::Runtime;

use super::eval::Stmt;
use super::{Error, Rejection, Rows};

/// A normalised change set whose new rows each fit the row, value and key limits. Only
/// [`Stmt::check_sizes`] builds one.
#[derive(Debug)]
pub struct Admitted {
    plus: Rows,
    minus: Rows,
}

impl Admitted {
    pub fn plus(&self) -> &Rows {
        &self.plus
    }

    pub fn minus(&self) -> &Rows {
        &self.minus
    }

    /// Returns the inserted rows, then the deleted.
    pub fn into_rows(self) -> (Rows, Rows) {
        (self.plus, self.minus)
    }
}

/// Returns the refusal of heap error `e`, met admitting a row of constructor `ctor` of `rel`.
fn too_large(rel: RelId, ctor: u32) -> impl Fn(heap::Error) -> Error {
    move |e| match e {
        heap::Error::TooLarge { len, limit } => Error::Rejected(Rejection::RowTooLarge {
            relation: rel,
            ctor,
            size: len,
            limit,
        }),
        e => e.into(),
    }
}

impl<R: Runtime> Stmt<'_, R> {
    /// Admit the change set of `plus` and `minus`, refusing a new row that holds a value past the
    /// value limit or whose stored form passes the row limit, then one whose index key passes its
    /// limit.
    pub(super) fn check_sizes(&self, plus: Rows, minus: Rows) -> Result<Admitted, Error> {
        for ((r, c), rows) in &plus {
            let heap = self.rel(*r).ctors[*c as usize].heap;
            for row in rows {
                self.heap(heap)
                    .check_row(&self.stored(heap, row)?)
                    .map_err(too_large(*r, *c))?;
            }
        }
        for ((r, c), rows) in &plus {
            let rel = self.rel(*r);
            let heap = rel.ctors[*c as usize].heap;
            for row in rows {
                let row = self.stored(heap, row)?;
                for (id, ix) in &rel.indexes {
                    let types = self.store.key_types(*id)?;
                    let Some(key) = ix.keys.key(*c, &row, &types)? else {
                        continue;
                    };
                    let stored = &self.db.bindings.indexes()[id];
                    let ctor = *stored
                        .ctors
                        .iter()
                        .find(|(_, n)| **n == *c)
                        .expect("an index over a constructor holds its id")
                        .0;
                    if let Err(e) = stored
                        .index
                        .check_key(self.db.pager.page_size(), &key, ctor)
                    {
                        return Err(match e {
                            crate::btree::Error::TooLong { len, max } => {
                                let size = stored.index.schema().encode(&key).len();
                                Error::Rejected(Rejection::KeyTooLarge {
                                    relation: *r,
                                    ctor: *c,
                                    index: ix.clause.clone(),
                                    size,
                                    limit: max - (len - size),
                                })
                            }
                            e => e.into(),
                        });
                    }
                }
            }
        }
        Ok(Admitted { plus, minus })
    }
}
