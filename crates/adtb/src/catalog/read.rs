//! The one way a catalog transaction reads the rows of a relation's heaps: at the snapshot of its
//! hold, each heap judged against the pager's marks before it is read.

use std::collections::BTreeMap;
use std::sync::Arc;

use adtb_lang::types::RelId;

use crate::exec::Store;
use crate::heap::{self, Heap};
use crate::key::Value;
use crate::mvcc::{Hold, Transactions, Xid};
use crate::plan::{self, HeapId};
use crate::runtime::Runtime;
use crate::storage::{PageError, Pager, Quarantine};

use super::{Abandoning, Catalog, Error, Migrating};

/// What a catalog transaction reads: the rows the snapshot of a hold sees, with the writes of a
/// transaction when it names one.
pub(super) struct Snapshot<'s, R: Runtime> {
    pager: &'s Pager<R>,
    txns: &'s Transactions<R>,
    hold: &'s Hold,
    own: Option<Xid>,
}

impl<'s, R: Runtime> Snapshot<'s, R> {
    /// Returns the snapshot of `hold`, with the writes of `own` when it is a transaction.
    pub(super) fn new(
        pager: &'s Pager<R>,
        txns: &'s Transactions<R>,
        (hold, own): (&'s Hold, Option<Xid>),
    ) -> Snapshot<'s, R> {
        Snapshot {
            pager,
            txns,
            hold,
            own,
        }
    }

    /// Returns the snapshot of `m.hold`, without the writes of `m.x`.
    pub(super) fn of(m: &'s Migrating<'_, R>) -> Snapshot<'s, R> {
        Snapshot::new(m.pager, m.txns, (&m.hold, None))
    }

    /// Returns the snapshot of `m.hold`, with the writes of `m.x`.
    pub(super) fn writing(m: &'s Migrating<'_, R>) -> Snapshot<'s, R> {
        Snapshot::new(m.pager, m.txns, (&m.hold, Some(m.x)))
    }

    fn marked(&self, heap: u32) -> bool {
        self.pager.any_quarantined() && self.pager.quarantined().contains(&Quarantine::Heap(heap))
    }

    /// Returns the rows of `heap` this snapshot sees, in heap order.
    fn visible(&self, heap: &Heap) -> Result<Vec<Vec<Value>>, PageError> {
        let pager = self.pager;
        let reader = self.txns.reader(pager, self.hold, self.own);
        'scan: loop {
            let mut versions = Vec::new();
            heap.scan(pager, |v| versions.push(v))?;
            let mut out = Vec::new();
            for v in versions {
                let Some(seen) = v.looked_up(pager, reader.sees(&v.header.writers))? else {
                    continue 'scan;
                };
                if seen.visible() {
                    out.push(v.row);
                }
            }
            return Ok(out);
        }
    }
}

impl Catalog {
    /// Returns the rows of relation `r` that `at` sees, each with its constructor index, by
    /// constructor and then in heap order.
    ///
    /// Fail with [`Error::Quarantined`] naming the first constructor relation of `r` the pager
    /// marks.
    pub(super) fn rows<R: Runtime>(
        &self,
        at: &Snapshot<'_, R>,
        r: RelId,
    ) -> Result<Vec<(u32, Vec<Value>)>, Error> {
        let mut out = Vec::new();
        for (&(_, c), heap) in self.heaps.range((r, 0)..=(r, u32::MAX)) {
            self.admit(at, r, c)?;
            out.extend(at.visible(heap)?.into_iter().map(|row| (c, row)));
        }
        Ok(out)
    }

    /// Returns whether the heap of constructor `c` of relation `r` holds a row `at` sees.
    ///
    /// Fail with [`Error::Quarantined`] naming the constructor relation when the pager marks it.
    pub(super) fn holds_any<R: Runtime>(
        &self,
        at: &Snapshot<'_, R>,
        r: RelId,
        c: u32,
    ) -> Result<bool, Error> {
        self.admit(at, r, c)?;
        Ok(!at.visible(self.heap(r, c))?.is_empty())
    }

    /// Returns the store of queries over this catalog's heaps and indexes at `at`.
    pub(super) fn store<R: Runtime>(&self, at: &Snapshot<'_, R>) -> Judged<R> {
        let names = self
            .heaps
            .iter()
            .map(|(&(r, c), heap)| (heap.id().0, self.ctor_named(r, c)))
            .collect();
        let reader = at.txns.reader(at.pager, at.hold, at.own);
        Judged {
            store: Arc::new(Store::migrating(reader, self.bindings())),
            names: Some(names),
        }
    }

    /// Fail with [`Error::Quarantined`] naming constructor `c` of relation `r` when the pager
    /// marks its heap.
    fn admit<R: Runtime>(&self, at: &Snapshot<'_, R>, r: RelId, c: u32) -> Result<(), Error> {
        if at.marked(self.heap(r, c).id().0) {
            return Err(Error::Quarantined(self.ctor_named(r, c)));
        }
        Ok(())
    }

    /// Returns the name a client sees of constructor `c` of relation `r`.
    fn ctor_named(&self, r: RelId, c: u32) -> String {
        let schema = &self.schema;
        let relation = schema.relation(r);
        let ctor = &schema.ty(relation.elem_def).ctors()[c as usize].name;
        format!(
            "{}::{}.{ctor}",
            schema.module(relation.module).name,
            relation.name
        )
    }
}

/// The store a catalog query runs over, and the name of each heap whose mark refuses a query
/// reading it.
pub(super) struct Judged<R: Runtime> {
    store: Arc<Store<R>>,
    names: Option<BTreeMap<u32, String>>,
}

impl<R: Runtime> Judged<R> {
    /// Returns `store` with no mark judged, for `verify`.
    pub(super) fn verifying(store: Arc<Store<R>>) -> Judged<R> {
        Judged { store, names: None }
    }

    pub(super) fn store(&self) -> &Arc<Store<R>> {
        &self.store
    }

    /// Fail with [`Error::Quarantined`] naming the first heap of `reads` the pager marks.
    pub(super) fn admit(&self, reads: &plan::Reads) -> Result<(), Error> {
        let Some(names) = &self.names else {
            return Ok(());
        };
        let pager = self.store.pager();
        if !pager.any_quarantined() {
            return Ok(());
        }
        let marks = pager.quarantined();
        for &HeapId(id) in &reads.heaps {
            let id = u32::try_from(id).expect("a heap id is a structure id");
            if marks.contains(&Quarantine::Heap(id))
                && let Some(name) = names.get(&id)
            {
                return Err(Error::Quarantined(name.clone()));
            }
        }
        Ok(())
    }
}

/// Delete, as `m.x`, one version the snapshot of `m.hold` sees of each row of `rows` from `heap`.
///
/// Precondition: `heap` holds the catalog's own rows or an engine relation's, whose damage marks
/// the database.
pub(super) fn delete_rows<R: Runtime>(
    m: &Migrating<'_, R>,
    heap: &Heap,
    rows: &[Vec<Value>],
) -> Result<(), Error> {
    delete_keyed(m, heap, rows.to_vec(), <[Value]>::to_vec)
}

/// Delete, as `m.x`, one version the snapshot of `m.hold` sees from `heap` of each row whose
/// image under `key` is one of `keys`.
///
/// Precondition: `heap` holds the catalog's own rows or an engine relation's, whose damage marks
/// the database.
pub(super) fn delete_keyed<R: Runtime>(
    m: &Migrating<'_, R>,
    heap: &Heap,
    mut keys: Vec<Vec<Value>>,
    key: impl Fn(&[Value]) -> Vec<Value>,
) -> Result<(), Error> {
    let pager = m.pager;
    if keys.is_empty() {
        return Ok(());
    }
    let reader = m.txns.reader(pager, &m.hold, None);
    'scan: loop {
        let mut versions = Vec::new();
        heap.scan(pager, |v| versions.push(v))?;
        for v in versions {
            let Some(at) = keys.iter().position(|k| *k == key(&v.row)) else {
                continue;
            };
            let Some(seen) = v.looked_up(pager, reader.sees(&v.header.writers))? else {
                continue 'scan;
            };
            if seen.visible() {
                heap.delete(pager, heap::WriteTarget::checked(v.tid, v.header), m.x)
                    .kept(&Abandoning(()))?
                    .unchecked(&Abandoning(()));
                keys.swap_remove(at);
            }
        }
        assert!(keys.is_empty(), "every row deleted is visible: {keys:?}");
        return Ok(());
    }
}
