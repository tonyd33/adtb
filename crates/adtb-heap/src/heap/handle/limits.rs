use crate::key::Value;
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::super::page::{Arrangement, align8};
use super::super::row::{FieldType, Layout, POINTER_LEN, value_limit};
use super::super::version::HEADER_LEN;
use super::super::{Error, fixed, slotted};

/// Returns the arrangement of the pages of a heap of `layout`, or the row limit its smallest
/// version passes when that does not fit a page of `page_size` bytes.
pub(super) fn arrangement_of(layout: &Layout, page_size: usize) -> Result<Arrangement, Oversize> {
    if !layout.is_fixed() {
        let len = HEADER_LEN + layout.least_body_len();
        let limit = slotted::max_version(page_size);
        if len > limit {
            return Err(Oversize::Row { len, limit });
        }
        return Ok(Arrangement::Slotted);
    }
    let width = HEADER_LEN + align8(layout.fixed_width());
    if fixed::capacity(page_size, width) == 0 {
        return Err(Oversize::Row {
            len: width,
            limit: fixed::max_width(page_size),
        });
    }
    Ok(Arrangement::Fixed { width })
}

/// The limit a row passes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Oversize {
    /// A field's whole value is stored in `len` bytes, past the value limit `limit`.
    Value { len: usize, limit: usize },
    /// The row's smallest version, every value it can move out of line moved, is `len` bytes,
    /// past the `limit` a page holds.
    Row { len: usize, limit: usize },
}

impl From<Oversize> for Error {
    fn from(o: Oversize) -> Error {
        match o {
            Oversize::Value { len, limit } | Oversize::Row { len, limit } => {
                Error::TooLarge { len, limit }
            }
        }
    }
}

/// The limits each row of a heap of one row type is stored within.
#[derive(Debug)]
pub struct RowLimits {
    layout: Layout,
    arrangement: Arrangement,
    page_size: usize,
    value_limit: u64,
}

impl RowLimits {
    /// Returns the limits of the rows of a heap of fields `row` on `pager`, or the row limit its
    /// smallest version passes when that does not fit a page.
    pub fn new<R: Runtime>(pager: &Pager<R>, row: Vec<FieldType>) -> Result<RowLimits, Oversize> {
        let layout = Layout::new(row);
        let arrangement = arrangement_of(&layout, pager.page_size())?;
        Ok(RowLimits {
            layout,
            arrangement,
            page_size: pager.page_size(),
            value_limit: value_limit(pager),
        })
    }

    /// Refuse a row with a field whose whole value's stored length passes the value limit, or
    /// else a row whose smallest version passes the largest version a page holds.
    pub fn check(&self, row: &[Value]) -> Result<(), Oversize> {
        oversize(
            &self.layout,
            self.arrangement,
            self.page_size,
            self.value_limit,
            row,
        )
    }
}

/// Refuse a row of `layout` with a field whose whole value's stored length passes `value_limit`,
/// or else a row whose smallest version passes the largest version a page of `arrangement` holds.
pub(super) fn oversize(
    layout: &Layout,
    arrangement: Arrangement,
    page_size: usize,
    value_limit: u64,
    row: &[Value],
) -> Result<(), Oversize> {
    let (_, vars) = layout.encode(row);
    let mut lengths = layout.field_lengths(&vars);
    if let Some(len) = lengths.find(|&len| len as u64 > value_limit) {
        return Err(Oversize::Value {
            len,
            limit: value_limit as usize,
        });
    }
    let lengths = vars.iter().map(Vec::len);
    smallest(layout, arrangement, page_size, lengths)
        .map_err(|(len, limit)| Oversize::Row { len, limit })
}

/// Refuse a version of `layout` whose variable-width values are `lengths` long when its smallest
/// form, each value longer than a pointer moved out of line, passes the largest version a page
/// of `arrangement` holds. Returns that form's length and the limit.
pub(super) fn smallest(
    layout: &Layout,
    arrangement: Arrangement,
    page_size: usize,
    lengths: impl Iterator<Item = usize>,
) -> Result<(), (usize, usize)> {
    let limit = match arrangement {
        Arrangement::Fixed { width } => width,
        Arrangement::Slotted => slotted::max_version(page_size),
    };
    let base = HEADER_LEN + layout.least_body_len();
    let len = base + lengths.map(|len| len.min(POINTER_LEN)).sum::<usize>();
    if len > limit {
        return Err((len, limit));
    }
    Ok(())
}
