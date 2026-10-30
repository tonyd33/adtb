//! The rows of volatile relations kept in the file: rows of the roots heap, each naming its
//! relation by catalog id and the group a writer replaces together.

use std::collections::BTreeSet;

use crate::heap::{self, FieldType, WriteTarget};
use crate::key::{CtorId, Scalar, Value};
use crate::mvcc::{Scope, Transactions, Xid};
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::{Catalog, Error, ROOTS, int, uint, visible};

/// The constructor of a roots row holding a row of a volatile relation.
const KEPT: u32 = 14;

/// The only encoding of a kept row's fields this engine reads.
const ROW_FORMAT: u64 = 1;

/// One field of a kept row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeptField {
    Int(i64),
    Instant(i64),
    Text(String),
}

/// A row of a volatile relation kept in the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeptRow {
    /// The catalog id of the relation.
    pub relation: u64,
    /// What a writer replaces together.
    pub group: String,
    /// The constructor of the row, by position.
    pub ctor: u32,
    pub fields: Vec<KeptField>,
}

pub(super) fn ctors() -> [heap::CtorType; 1] {
    [heap::CtorType {
        id: CtorId(KEPT),
        fields: [
            Scalar::I64,
            Scalar::Text,
            Scalar::I64,
            Scalar::I64,
            Scalar::Bytes,
        ]
        .into_iter()
        .map(FieldType::Scalar)
        .collect(),
    }]
}

pub(super) fn holds(ctor: u32) -> bool {
    ctor == KEPT
}

fn encode(fields: &[KeptField]) -> Vec<u8> {
    let mut out = Vec::new();
    for f in fields {
        match f {
            KeptField::Int(n) => {
                out.push(0);
                out.extend_from_slice(&n.to_le_bytes());
            }
            KeptField::Instant(n) => {
                out.push(2);
                out.extend_from_slice(&n.to_le_bytes());
            }
            KeptField::Text(t) => {
                out.push(1);
                out.extend_from_slice(&(t.len() as u64).to_le_bytes());
                out.extend_from_slice(t.as_bytes());
            }
        }
    }
    out
}

fn decode(mut bytes: &[u8]) -> Option<Vec<KeptField>> {
    let mut out = Vec::new();
    while let Some((tag, rest)) = bytes.split_first() {
        let (head, rest) = rest.split_first_chunk::<8>()?;
        match tag {
            0 => {
                out.push(KeptField::Int(i64::from_le_bytes(*head)));
                bytes = rest;
            }
            1 => {
                let len = usize::try_from(u64::from_le_bytes(*head)).ok()?;
                let (text, rest) = rest.split_at_checked(len)?;
                out.push(KeptField::Text(String::from_utf8(text.to_vec()).ok()?));
                bytes = rest;
            }
            2 => {
                out.push(KeptField::Instant(i64::from_le_bytes(*head)));
                bytes = rest;
            }
            _ => return None,
        }
    }
    Some(out)
}

fn row_of(fields: &[Value]) -> Result<KeptRow, Error> {
    let bad = || Error::Corrupt(format!("a kept row holds {fields:?}"));
    let [
        relation,
        Value::Text(group),
        ctor,
        format,
        Value::Bytes(row),
    ] = fields
    else {
        return Err(bad());
    };
    let format = uint(format)?;
    if format != ROW_FORMAT {
        return Err(Error::Corrupt(format!(
            "a kept row has format {format}; this engine reads format {ROW_FORMAT}"
        )));
    }
    Ok(KeptRow {
        relation: uint(relation)?,
        group: group.clone(),
        ctor: u32::try_from(uint(ctor)?).map_err(|_| bad())?,
        fields: decode(row).ok_or_else(bad)?,
    })
}

impl Catalog {
    /// Returns the page ids of the heap kept rows lie in.
    pub fn kept_pages<R: Runtime>(
        &self,
        pager: &Pager<R>,
    ) -> Result<Vec<crate::storage::Pid>, Error> {
        Ok(self.roots.pages(pager)?)
    }

    /// Returns the kept rows of the latest commit.
    pub fn kept<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
    ) -> Result<Vec<KeptRow>, Error> {
        let (id, snapshot) = txns.take(Scope::Heaps(BTreeSet::from([ROOTS])))?;
        let rows = visible(pager, txns, &snapshot, &self.roots);
        txns.release(id);
        let mut out = Vec::new();
        for row in rows? {
            if let [Value::Sum { ctor, fields }] = row.as_slice()
                && holds(*ctor)
            {
                out.push(row_of(fields)?);
            }
        }
        Ok(out)
    }

    /// Replace in transaction `x` the kept rows of `relation` and `group` by `rows`.
    ///
    /// Preconditions: `x` is a transaction of the write path, and no other transaction changes
    /// the rows of `relation` and `group` while it runs.
    pub fn replace_kept<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        x: Xid,
        (relation, group): (u64, &str),
        rows: &[KeptRow],
    ) -> Result<(), Error> {
        let (id, snapshot) = txns.take(Scope::Heaps(BTreeSet::from([ROOTS])))?;
        let mut versions = Vec::new();
        let scanned = self.roots.scan(pager, |v| versions.push(v));
        let mut gone = Vec::new();
        let judged = scanned.and_then(|()| {
            for v in versions {
                if !txns.sees(pager, &snapshot, None, v.header.xmin, v.header.xmax)? {
                    continue;
                }
                if let [Value::Sum { ctor, fields }] = v.row.as_slice()
                    && holds(*ctor)
                    && let [r, Value::Text(g), ..] = fields.as_slice()
                    && uint(r).is_ok_and(|r| r == relation)
                    && g == group
                {
                    gone.push((v.tid, v.header));
                }
            }
            Ok(())
        });
        txns.release(id);
        judged?;
        for (tid, header) in gone {
            self.roots
                .delete(pager, WriteTarget::checked(tid, header), x.0)?;
        }
        for r in rows {
            let row = Value::Sum {
                ctor: KEPT,
                fields: vec![
                    int(r.relation),
                    Value::Text(r.group.clone()),
                    int(u64::from(r.ctor)),
                    int(ROW_FORMAT),
                    Value::Bytes(encode(&r.fields)),
                ],
            };
            self.roots.insert(pager, x.0, &[row])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(format: u64) -> Vec<Value> {
        vec![
            int(339),
            Value::Text("database".into()),
            int(2),
            int(format),
            Value::Bytes(encode(&[
                KeptField::Int(7),
                KeptField::Instant(-3),
                KeptField::Text("page 9".into()),
            ])),
        ]
    }

    #[test]
    fn a_kept_row_of_this_format_reads_back() {
        let row = row_of(&stored(ROW_FORMAT)).unwrap();
        assert_eq!(2, row.ctor);
        assert_eq!(
            vec![
                KeptField::Int(7),
                KeptField::Instant(-3),
                KeptField::Text("page 9".into()),
            ],
            row.fields
        );
    }

    #[test]
    fn a_kept_row_of_another_format_is_refused() {
        assert!(matches!(
            row_of(&stored(ROW_FORMAT + 1)),
            Err(Error::Corrupt(why)) if why.contains("format 2")
        ));
    }
}
