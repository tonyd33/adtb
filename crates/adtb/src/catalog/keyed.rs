//! The engine's keyed storage: the deployment facts and kept rows, held as rows of the roots
//! heap, each under a key that two writers of conflict over. They are engine-internal: no
//! system relation holds them, and no format or engine hash names them.

use std::fmt;

use crate::heap::{Flags, Version};
use crate::mvcc::{Conflict, Hold, Logged, Probe, Reader, Transactions, Write, Xid};
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::Error;
use super::deployment::Fact;
use super::roots::RootsRow;
use super::roots_heap::Roots;

/// A running transaction of the write path, as keyed storage writes in it: its id and the hold
/// of its snapshot.
#[derive(Clone, Copy)]
pub struct Writer<'a> {
    hold: &'a Hold,
    x: Xid,
}

impl fmt::Debug for Writer<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Writer")
            .field("x", &self.x)
            .finish_non_exhaustive()
    }
}

impl<'a> Writer<'a> {
    /// Returns the writer of transaction `x`, whose snapshot `hold` holds.
    pub(crate) fn new(hold: &'a Hold, x: Xid) -> Writer<'a> {
        Writer { hold, x }
    }
}

/// What two writers of keyed storage conflict over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Key {
    /// The mark that the file records family owners: one key for every deploy, so any two
    /// deploys that write it conflict.
    Marked,
    Family(String),
    Registered(i64),
    Expired(i64),
    /// The kept rows of the relation of this catalog id.
    Kept(i64),
}

impl RootsRow {
    /// Returns the key this row is stored under, `None` for a row of no keyed storage.
    pub(super) fn key(&self) -> Option<Key> {
        match self {
            RootsRow::Deployment(Fact::Deployed) => Some(Key::Marked),
            RootsRow::Deployment(Fact::Family(family, _)) => Some(Key::Family(family.clone())),
            RootsRow::Deployment(Fact::Registered(bundle, _)) => Some(Key::Registered(*bundle)),
            RootsRow::Deployment(Fact::Expired(bundle, _)) => Some(Key::Expired(*bundle)),
            RootsRow::Kept { relation, .. } => Some(Key::Kept(*relation)),
            RootsRow::HeapRoot { .. }
            | RootsRow::IndexRoot { .. }
            | RootsRow::OrderKey { .. }
            | RootsRow::Migration { .. }
            | RootsRow::HistoryHeap { .. }
            | RootsRow::HistoryIndex { .. }
            | RootsRow::Held { .. }
            | RootsRow::Dormant { .. } => None,
        }
    }
}

/// Make `key` hold the rows `next` returns, given the rows it holds at the snapshot of `w` with
/// the writes of `w`.
///
/// Fail with a conflict when another transaction that is running, or that committed after the
/// snapshot of `w`, wrote a row of `key`.
pub(super) fn put<R: Runtime>(
    pager: &Pager<R>,
    txns: &Transactions<R>,
    roots: &Roots,
    w: Writer<'_>,
    key: &Key,
    next: impl FnOnce(&[RootsRow]) -> Vec<RootsRow>,
) -> Result<(), Error> {
    let reader = txns.reader(pager, w.hold, Some(w.x));
    let held = seen(pager, &reader, roots, (w.x, key))?;
    let now: Vec<RootsRow> = held.iter().map(|(_, row)| row.clone()).collect();
    let after = next(&now);
    for (v, row) in &held {
        if after.contains(row) {
            continue;
        }
        roots.delete_version(pager, v, w.x)?;
    }
    let fresh = after.into_iter().filter(|r| !now.contains(r));
    roots.insert(pager, txns, w.x, &roots.admit(fresh)?)?;
    unrivalled(pager, &reader, roots, (w.x, key))
}

/// Returns each version of `key` in `roots` that `reader` sees, with its row. Fail with a
/// conflict when a transaction other than `x` deletes one and is running or committed after
/// the snapshot of `reader`.
fn seen<R: Runtime>(
    pager: &Pager<R>,
    reader: &Reader<Logged<R>>,
    roots: &Roots,
    (x, key): (Xid, &Key),
) -> Result<Vec<(Version, RootsRow)>, Error> {
    let heap = roots.id();
    'scan: loop {
        let mut out = Vec::new();
        for (v, row) in roots.versions(pager)? {
            if row.key().as_ref() != Some(key) {
                continue;
            }
            let Some(seen) = v.looked_up(pager, reader.sees(&v.header.writers))? else {
                continue 'scan;
            };
            if !seen.visible() {
                continue;
            }
            let checked = reader.check_write(x, &v.header.writers, heap);
            match v.looked_up(pager, checked)? {
                None => continue 'scan,
                Some(Write::Proceed) => out.push((v, row)),
                Some(Write::Wait(other)) => {
                    return Err(Error::Conflict(Conflict::running(heap, other)));
                }
                Some(Write::Fail(f)) => return Err(Error::Conflict(f)),
            }
        }
        return Ok(out);
    }
}

/// Fail with a conflict when a version of `key` in `roots` was written by a transaction other
/// than `x` that is running or committed after the snapshot of `reader`.
fn unrivalled<R: Runtime>(
    pager: &Pager<R>,
    reader: &Reader<Logged<R>>,
    roots: &Roots,
    (x, key): (Xid, &Key),
) -> Result<(), Error> {
    let heap = roots.id();
    'scan: loop {
        for (v, row) in roots.versions(pager)? {
            if row.key().as_ref() != Some(key) {
                continue;
            }
            let taken_back = v.flags.contains(Flags::TAKEN_BACK);
            let probed = reader.probe_unique(x, &v.header.writers, taken_back, heap);
            match v.looked_up(pager, probed)? {
                None => continue 'scan,
                Some(Probe::Skip | Probe::Conflict) => {}
                Some(Probe::Wait(other)) => {
                    return Err(Error::Conflict(Conflict::running(heap, other)));
                }
                Some(Probe::Fail(f)) => return Err(Error::Conflict(f)),
            }
        }
        return Ok(());
    }
}
