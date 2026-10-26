use std::collections::{BTreeMap, BTreeSet};

use crate::btree::{self, Index};
use crate::heap::{Fate, Header, Heap, LargeStore, Pruned};
use crate::key::CtorId;
use crate::mvcc::{Status, Transactions, Xid};
use crate::runtime::Runtime;
use crate::storage::{PageError, Pager, Pid};

use super::dead::DeadSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Page(PageError),
    Index(btree::Error),
}

impl From<PageError> for Error {
    fn from(e: PageError) -> Error {
        Error::Page(e)
    }
}

impl From<btree::Error> for Error {
    fn from(e: btree::Error) -> Error {
        Error::Index(e)
    }
}

/// The heaps and indexes of one sum.
///
/// Invariants: the heaps share one large-value store, and every index entry's constructor is
/// one of `heaps`.
#[derive(Debug)]
pub struct Sum<'a> {
    /// Each constructor's heap.
    pub heaps: Vec<(CtorId, &'a mut Heap)>,
    pub indexes: Vec<&'a Index>,
}

/// What one run of [`vacuum`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    /// Heap pages read.
    pub scanned: usize,
    /// Heap pages skipped as all-visible.
    pub skipped: usize,
    /// Slots found dead and named by index entries.
    pub dead: usize,
    /// Index entries deleted.
    pub entries: usize,
    /// Heap slots emptied.
    pub freed: usize,
    /// Versions whose aborted deleter was cleared.
    pub cleared: usize,
    /// Heap pages left all-visible.
    pub all_visible: usize,
    /// Heap pages that left their heap.
    pub removed: usize,
    /// Out-of-line chunks freed.
    pub chunks: usize,
    /// Rounds of index cleanup and pruning.
    pub rounds: usize,
    /// No version of the sum names an aborted transaction below this id, nor has one as its
    /// deleter.
    pub clean_below: Xid,
}

/// Returns the fate of the version whose header is `h` in a heap whose watermark is `watermark`.
pub fn judge<R: Runtime>(
    txns: &Transactions<R>,
    pager: &Pager<R>,
    watermark: Xid,
    h: &Header,
) -> Result<Fate, PageError> {
    match txns.status(pager, Xid(h.xmin))? {
        Status::Aborted => return Ok(Fate::Remove),
        Status::InProgress => return Ok(Fate::Keep),
        Status::Committed => {}
    }
    if Xid(h.xmax) == Xid::NONE {
        return Ok(if Xid(h.xmin) < watermark {
            Fate::AllVisible
        } else {
            Fate::Keep
        });
    }
    Ok(match txns.status(pager, Xid(h.xmax))? {
        Status::Aborted => Fate::Clear,
        Status::Committed if Xid(h.xmax) < watermark => Fate::Remove,
        Status::Committed | Status::InProgress => Fate::Keep,
    })
}

/// Prune page `pid` of `heap` of the versions dead to every snapshot its watermark admits,
/// leaving its all-visible bit as it is unless the page changes.
pub fn prune<R: Runtime>(
    pager: &Pager<R>,
    txns: &Transactions<R>,
    heap: &mut Heap,
    pid: Pid,
) -> Result<Pruned, PageError> {
    let w = txns.watermark(heap.id());
    heap.prune(pager, pid, |p, h| judge(txns, p, w, h), &[], false)
}

/// Vacuum `sum`: delete the index entries of versions dead to every snapshot their heap's
/// watermark admits or whose inserter aborted, free those versions, clear aborted deleters, set
/// all-visible bits, remove empty pages from their heaps, and free the out-of-line chunks no
/// version points at.
///
/// Heap pages are scanned in directory order, skipping all-visible ones. Each time the dead set
/// passes `budget` bytes, and once at the end, every index of the sum is cleaned of the set's
/// TIDs in one pass and then the set's pages are pruned.
///
/// Precondition: no write to the sum is in progress.
///
/// Panics if the heaps do not share one large-value store.
pub fn vacuum<R: Runtime>(
    pager: &Pager<R>,
    txns: &Transactions<R>,
    sum: &mut Sum,
    budget: usize,
) -> Result<Report, Error> {
    let large = shared_store(sum);
    let mut report = Report {
        scanned: 0,
        skipped: 0,
        dead: 0,
        entries: 0,
        freed: 0,
        cleared: 0,
        all_visible: 0,
        removed: 0,
        chunks: 0,
        rounds: 0,
        clean_below: txns.running().first().copied().unwrap_or(txns.next()),
    };
    let mut dead = DeadSet::new();
    for h in 0..sum.heaps.len() {
        let heap = &*sum.heaps[h].1;
        let id = heap.id();
        for pid in heap.pages(pager)? {
            let heap = &*sum.heaps[h].1;
            report.scanned += 1;
            if heap.all_visible(pager, pid)? {
                report.skipped += 1;
                continue;
            }
            let w = txns.watermark(id);
            let slots = heap.dead_slots(pager, pid, |p, hd| judge(txns, p, w, hd))?;
            report.dead += slots.len();
            dead.add(id, pid, &slots);
            if dead.bytes() >= budget {
                clean(pager, txns, sum, &mut dead, &mut report)?;
            }
        }
    }
    clean(pager, txns, sum, &mut dead, &mut report)?;
    let mut keep = BTreeSet::new();
    for (_, heap) in &sum.heaps {
        for pointer in heap.out_of_line(pager)? {
            keep.extend(large.chunks(pager, pointer)?);
        }
    }
    report.chunks = large.sweep(pager, &keep)?;
    Ok(report)
}

fn shared_store(sum: &Sum) -> LargeStore {
    let large = sum.heaps[0].1.large();
    assert!(
        sum.heaps.iter().all(|(_, h)| h.large() == large),
        "the heaps of a sum share one large-value store"
    );
    large
}

/// Delete the index entries of every TID in `dead`, then prune each of its pages, freeing those
/// TIDs, and empty `dead`.
fn clean<R: Runtime>(
    pager: &Pager<R>,
    txns: &Transactions<R>,
    sum: &mut Sum,
    dead: &mut DeadSet,
    report: &mut Report,
) -> Result<(), Error> {
    if dead.is_empty() {
        return Ok(());
    }
    report.rounds += 1;
    let heap_of: BTreeMap<CtorId, u32> = sum.heaps.iter().map(|(c, h)| (*c, h.id())).collect();
    for index in &sum.indexes {
        report.entries += index.retain(pager, |e| {
            !heap_of
                .get(&e.ctor)
                .is_some_and(|&heap| dead.contains(heap, e.tid))
        })?;
    }
    for (_, heap) in sum.heaps.iter_mut() {
        let w = txns.watermark(heap.id());
        for (pid, slots) in dead.pages(heap.id()) {
            let done = heap.prune(pager, pid, |p, h| judge(txns, p, w, h), &slots, true)?;
            report.freed += done.freed;
            report.cleared += done.cleared;
            report.all_visible += usize::from(done.all_visible);
            report.removed += usize::from(done.removed);
        }
    }
    dead.clear();
    Ok(())
}
