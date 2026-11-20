//! The rows of volatile relations kept in the file: rows of the roots heap, each naming its
//! relation by catalog id and the group a writer replaces together.

use std::collections::BTreeSet;

use adtb_lang::ids::CatCtorId;

use crate::heap::{Unchecked, WriteTarget};
use crate::mvcc::{Scope, Transactions, Xid};
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::roots::{self, RootsRow};
use super::{Catalog, Error, ROOTS, insert_row, visible};

/// The only encoding of a kept row's constructor and fields this engine reads.
pub(super) const ROW_FORMAT: u64 = 2;

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
    /// The catalog id of the row's constructor, `None` for a row that is no value of a sum. The
    /// file holds `None` as 0, an id no constructor holds.
    pub ctor: Option<CatCtorId>,
    pub fields: Vec<KeptField>,
}

pub(super) fn encode(fields: &[KeptField]) -> Vec<u8> {
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

pub(super) fn decode(mut bytes: &[u8]) -> Option<Vec<KeptField>> {
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

impl Catalog {
    /// Returns the page ids of the heap kept rows lie in.
    #[cfg(any(test, feature = "testing"))]
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
        let hold = txns.take(Scope::Heaps(BTreeSet::from([ROOTS])))?;
        let rows = visible(pager, txns, &hold, &self.roots);
        drop(hold);
        let mut out = Vec::new();
        for row in rows? {
            match RootsRow::decode(&row)? {
                RootsRow::Kept {
                    relation,
                    group,
                    ctor,
                    format,
                    row,
                } => out.push(roots::kept_row(relation, &group, ctor, format, &row)?),
                RootsRow::HeapRoot { .. }
                | RootsRow::IndexRoot { .. }
                | RootsRow::OrderKey { .. }
                | RootsRow::Migration { .. }
                | RootsRow::HistoryHeap { .. }
                | RootsRow::HistoryIndex { .. }
                | RootsRow::Held { .. }
                | RootsRow::Dormant { .. }
                | RootsRow::Deployment(_) => {}
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
        let hold = txns.take(Scope::Heaps(BTreeSet::from([ROOTS])))?;
        let judged = || -> Result<_, Error> {
            let reader = txns.reader(pager, &hold, None);
            'scan: loop {
                let mut versions = Vec::new();
                self.roots.scan(pager, |v| versions.push(v))?;
                let mut gone = Vec::new();
                for v in versions {
                    let Some(seen) = v.looked_up(pager, reader.sees(&v.header.writers))? else {
                        continue 'scan;
                    };
                    if !seen.visible() {
                        continue;
                    }
                    match RootsRow::decode(&v.row)? {
                        RootsRow::Kept {
                            relation: r,
                            group: g,
                            ..
                        } if u64::try_from(r) == Ok(relation) && g == group => {
                            gone.push((v.tid, v.header));
                        }
                        RootsRow::Kept { .. }
                        | RootsRow::HeapRoot { .. }
                        | RootsRow::IndexRoot { .. }
                        | RootsRow::OrderKey { .. }
                        | RootsRow::Migration { .. }
                        | RootsRow::HistoryHeap { .. }
                        | RootsRow::HistoryIndex { .. }
                        | RootsRow::Held { .. }
                        | RootsRow::Dormant { .. }
                        | RootsRow::Deployment(_) => {}
                    }
                }
                return Ok(gone);
            }
        };
        let gone = judged();
        drop(hold);
        let gone = gone?;
        for (tid, header) in gone {
            self.roots
                .delete(pager, WriteTarget::checked(tid, header), x)?
                .unchecked(Unchecked::Catalog);
        }
        for r in rows {
            let row = roots::kept(r);
            insert_row(pager, txns, &self.roots, x, &row.encode())?;
        }
        Ok(())
    }
}
