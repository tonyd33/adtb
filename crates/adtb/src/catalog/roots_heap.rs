//! The roots heap: rows written and read only as [`RootsRow`].

use std::sync::Arc;

use crate::heap::{self, Heap, LargeStore, Version};
use crate::key::{Tid, Value};

use crate::mvcc::{Hold, Transactions, Xid};
use crate::runtime::Runtime;
use crate::storage::{PageError, Pager, Pid};

use super::admit::{RootsAdmitted, admit_roots};
use super::roots::{self, RootsRow};
use super::{Abandoning, Error, Migrating, ROOTS, delete_rows};

/// The roots heap of a catalog.
#[derive(Clone)]
pub(super) struct Roots(Arc<Heap>);

impl Roots {
    /// Create an empty roots heap in `pager`.
    pub(super) fn create<R: Runtime>(pager: &Pager<R>, large: LargeStore) -> Result<Roots, Error> {
        Ok(Roots(Arc::new(Heap::create(
            pager,
            ROOTS,
            roots::row_type(),
            large,
        )?)))
    }

    /// Returns the roots heap of id `id` rooted at `at`.
    pub(super) fn open<R: Runtime>(
        pager: &Pager<R>,
        (id, at): (u32, Pid),
        large: LargeStore,
    ) -> Result<Roots, Error> {
        Ok(Roots(Arc::new(Heap::open(
            pager,
            id,
            at,
            roots::row_type(),
            large,
        )?)))
    }

    /// Returns the id and root page the catalog root names it by.
    pub(super) fn named(&self) -> (u32, Pid) {
        (self.0.id().0, self.0.root())
    }

    pub(super) fn id(&self) -> u32 {
        self.0.id().0
    }

    /// Returns the heap, for a verifier that reads its pages.
    pub(super) fn verified(&self) -> Arc<Heap> {
        self.0.clone()
    }

    #[cfg(any(test, feature = "testing"))]
    pub(super) fn pages<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pid>, PageError> {
        self.0.pages(pager)
    }

    pub(super) fn owned_pages<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pid>, PageError> {
        self.0.owned_pages(pager)
    }

    /// Returns each row the snapshot of `hold` sees, with the writes of `own` when it is a
    /// transaction, with the transaction that wrote it and where it lies.
    pub(super) fn rows<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        (hold, own): (&Hold, Option<Xid>),
    ) -> Result<Vec<(Xid, Tid, RootsRow)>, Error> {
        let reader = txns.reader(pager, hold, own);
        'scan: loop {
            let mut versions = Vec::new();
            self.0.scan(pager, |v| versions.push(v))?;
            let mut out = Vec::new();
            for v in versions {
                let Some(seen) = v.looked_up(pager, reader.sees(&v.header.writers))? else {
                    continue 'scan;
                };
                if seen.visible() {
                    out.push((
                        v.header.writers.inserter(),
                        v.tid,
                        RootsRow::decode(&v.row)?,
                    ));
                }
            }
            return Ok(out);
        }
    }

    /// Returns the rows of the latest state of a database whose commit log has `damage`.
    ///
    /// Fail with `damage` when a version's writer or deleter has a damaged status page and no
    /// commit record in the retained log, so no uncommitted row reads as committed.
    pub(super) fn latest<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        damage: PageError,
    ) -> Result<Vec<RootsRow>, Error> {
        let logged = pager.logged_commits();
        'scan: loop {
            let mut versions = Vec::new();
            self.0.scan(pager, |v| versions.push(v))?;
            let mut rows = Vec::new();
            for v in versions {
                let live = txns.latest_live(pager, &v.header.writers, &logged);
                let Some(live) = v.looked_up(pager, live)? else {
                    continue 'scan;
                };
                if live.ok_or(damage)? {
                    rows.push(RootsRow::decode(&v.row)?);
                }
            }
            return Ok(rows);
        }
    }

    /// Returns every version of the heap with its row, whoever wrote it.
    pub(super) fn versions<R: Runtime>(
        &self,
        pager: &Pager<R>,
    ) -> Result<Vec<(Version, RootsRow)>, Error> {
        let mut versions = Vec::new();
        self.0.scan(pager, |v| versions.push(v))?;
        versions
            .into_iter()
            .map(|v| {
                let row = RootsRow::decode(&v.row)?;
                Ok((v, row))
            })
            .collect()
    }

    /// Returns `rows` as the heap stores them, refusing a row past its limits.
    pub(super) fn admit(
        &self,
        rows: impl IntoIterator<Item = RootsRow>,
    ) -> Result<RootsAdmitted, Error> {
        admit_roots(&self.0, rows)
    }

    /// Insert `rows` as transaction `x`.
    pub(super) fn insert<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        x: Xid,
        rows: &RootsAdmitted,
    ) -> Result<(), Error> {
        for row in rows.rows() {
            let mut judge = txns.judge_now(pager, self.id());
            let put = self.0.insert_pruning(pager, x, row, &mut judge);
            put.kept(&Abandoning(()))?;
        }
        Ok(())
    }

    /// Insert `rows` as `m.x`, in order, as [`Heap::insert_all_pruning`] does.
    pub(super) fn insert_all<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        rows: &RootsAdmitted,
    ) -> Result<(), Error> {
        m.insert_all(&self.0, rows.rows().iter().map(Ok), |_, _| Ok(()))
    }

    /// Delete, as `m.x`, one version the snapshot of `m.hold` sees of each of `rows`.
    pub(super) fn delete<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        rows: &[RootsRow],
    ) -> Result<(), Error> {
        let rows: Vec<Vec<Value>> = rows.iter().map(RootsRow::encode).collect();
        delete_rows(m, &self.0, &rows)
    }

    /// Delete version `v` as transaction `x`, unless its header has changed.
    pub(super) fn delete_version<R: Runtime>(
        &self,
        pager: &Pager<R>,
        v: &Version,
        x: Xid,
    ) -> Result<(), Error> {
        self.0
            .delete(pager, heap::WriteTarget::checked(v.tid, v.header), x)
            .kept(&Abandoning(()))?
            .unchecked(&Abandoning(()));
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn heap(&self) -> &Heap {
        &self.0
    }
}
