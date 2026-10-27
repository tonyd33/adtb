use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::btree::{self, Index};
use crate::heap::{self, Fate, Header, Heap, LargeStore, Pruned, Updated};
use crate::key::{CtorId, Tid, Value};
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

/// The heaps of some constructors of one sum, and the indexes over them.
///
/// Invariants: the heaps share one large-value store. An index entry of a constructor outside
/// `heaps` is kept.
#[derive(Debug)]
pub struct Sum<'a> {
    /// Each constructor's heap.
    pub heaps: Vec<(CtorId, &'a Heap)>,
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
    heap: &Heap,
    pid: Pid,
) -> Result<Pruned, PageError> {
    let w = txns.watermark(heap.id());
    heap.prune(pager, pid, |p, h| judge(txns, p, w, h), &[], false)
}

/// Insert `row` into `heap` as `xid` writes it, pruning a page found without room first, as
/// [`prune`] does.
pub fn insert<R: Runtime>(
    pager: &Pager<R>,
    txns: &Transactions<R>,
    heap: &Heap,
    xid: Xid,
    row: &[Value],
) -> Result<Tid, heap::Error> {
    let w = txns.watermark(heap.id());
    let mut j = |p: &Pager<R>, h: &Header| judge(txns, p, w, h);
    heap.insert_pruning(pager, xid.0, row, Some(&mut j))
}

/// Update the version at `at` of `heap` as `xid` writes `row`, pruning a page found without room
/// first, as [`prune`] does.
pub fn update<R: Runtime>(
    pager: &Pager<R>,
    txns: &Transactions<R>,
    heap: &Heap,
    at: Tid,
    xid: Xid,
    row: &[Value],
    indexed: &[usize],
) -> Result<Updated, heap::Error> {
    let w = txns.watermark(heap.id());
    let mut j = |p: &Pager<R>, h: &Header| judge(txns, p, w, h);
    heap.update_pruning(pager, at, xid.0, row, indexed, Some(&mut j))
}

/// What a running vacuum is doing to one heap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Collecting dead versions from the heap's pages.
    Scan,
    /// Deleting the index entries of the dead versions collected.
    Indexes,
    /// Freeing the dead versions collected.
    Prune,
    /// Freeing the out-of-line values no version points at any more.
    Sweep,
}

/// The progress of a vacuum on one heap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    pub heap: u32,
    pub phase: Phase,
    /// The fraction of the heap's pages scanned, from 0 to 1.
    pub done: f64,
    /// Dead versions collected and not yet removed.
    pub dead_versions: usize,
}

/// The progress of the vacuums that report to it, one row per heap, readable while they run.
#[derive(Debug, Default)]
pub struct Progress {
    rows: Mutex<BTreeMap<u32, Step>>,
}

impl Progress {
    pub fn new() -> Progress {
        Progress::default()
    }

    /// Returns the progress of every heap of a running vacuum, by heap id.
    pub fn rows(&self) -> Vec<Step> {
        self.lock().values().copied().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<u32, Step>> {
        self.rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn set(&self, heap: u32, f: impl FnOnce(&mut Step)) {
        let mut rows = self.lock();
        let row = rows.entry(heap).or_insert(Step {
            heap,
            phase: Phase::Scan,
            done: 0.0,
            dead_versions: 0,
        });
        f(row);
    }

    fn finish(&self, heap: u32) {
        self.lock().remove(&heap);
    }
}

/// Vacuum `sum`: delete the index entries of versions dead to every snapshot their heap's
/// watermark admits or whose inserter aborted, free those versions, clear aborted deleters, set
/// all-visible bits, remove empty pages from their heaps, and free the out-of-line values no
/// version points at any more. Writers and readers of the sum may run meanwhile.
///
/// Heap pages are scanned in directory order, skipping all-visible ones. Each time the dead set
/// passes `budget` bytes, and once at the end, every index of the sum is cleaned of the set's
/// TIDs in one pass and then the set's pages are pruned, each under its exclusive latch. The
/// large-value store is then swept of the values left unreferenced. `progress` shows each heap's
/// phase while the vacuum runs.
///
/// Precondition: no other vacuum of the sum, or of a sum sharing its large-value store, runs.
///
/// Panics if the heaps do not share one large-value store.
pub fn vacuum<R: Runtime>(
    pager: &Pager<R>,
    txns: &Transactions<R>,
    sum: &Sum,
    budget: usize,
    progress: &Progress,
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
    let mut marks = Watermarks::default();
    for (_, heap) in &sum.heaps {
        progress.set(heap.id(), |_| {});
    }
    for (_, heap) in &sum.heaps {
        let id = heap.id();
        let pages = heap.pages(pager)?;
        for (n, pid) in pages.iter().enumerate() {
            report.scanned += 1;
            progress.set(id, |s| {
                s.phase = Phase::Scan;
                s.done = n as f64 / pages.len() as f64;
            });
            if heap.all_visible(pager, *pid)? {
                report.skipped += 1;
                continue;
            }
            let w = marks.of(txns, id);
            let slots = heap.dead_slots(pager, *pid, |p, hd| judge(txns, p, w, hd))?;
            report.dead += slots.len();
            progress.set(id, |s| s.dead_versions += slots.len());
            dead.add(id, *pid, &slots);
            if dead.bytes() >= budget {
                clean(
                    pager,
                    txns,
                    sum,
                    &mut dead,
                    &mut marks,
                    &mut report,
                    progress,
                )?;
            }
        }
        progress.set(id, |s| s.done = 1.0);
    }
    clean(
        pager,
        txns,
        sum,
        &mut dead,
        &mut marks,
        &mut report,
        progress,
    )?;
    for (_, heap) in &sum.heaps {
        progress.set(heap.id(), |s| s.phase = Phase::Sweep);
    }
    report.chunks = large.sweep(pager, |x| {
        Ok(txns.status(pager, Xid(x))? != Status::InProgress)
    })?;
    for (_, heap) in &sum.heaps {
        progress.finish(heap.id());
    }
    Ok(report)
}

/// The watermark each heap is judged by for the whole of one vacuum: the least of its watermark
/// and the oldest transaction running, taken when the vacuum first judges the heap.
///
/// Every transaction below it had resolved when it was taken. A version it finds dead stays dead
/// from the scan that collects it to the prune that frees it, however the heap's watermark moves.
#[derive(Debug, Default)]
struct Watermarks {
    fixed: BTreeMap<u32, Xid>,
}

impl Watermarks {
    fn of<R: Runtime>(&mut self, txns: &Transactions<R>, heap: u32) -> Xid {
        *self.fixed.entry(heap).or_insert_with(|| {
            let w = txns.watermark(heap);
            txns.running().first().map_or(w, |&r| w.min(r))
        })
    }
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
    sum: &Sum,
    dead: &mut DeadSet,
    marks: &mut Watermarks,
    report: &mut Report,
    progress: &Progress,
) -> Result<(), Error> {
    if dead.is_empty() {
        return Ok(());
    }
    report.rounds += 1;
    for (_, heap) in &sum.heaps {
        progress.set(heap.id(), |s| s.phase = Phase::Indexes);
    }
    let heap_of: BTreeMap<CtorId, u32> = sum.heaps.iter().map(|(c, h)| (*c, h.id())).collect();
    for index in &sum.indexes {
        report.entries += index.retain(pager, |e| {
            !heap_of
                .get(&e.ctor)
                .is_some_and(|&heap| dead.contains(heap, e.tid))
        })?;
    }
    for (_, heap) in &sum.heaps {
        let id = heap.id();
        progress.set(id, |s| s.phase = Phase::Prune);
        let w = marks.of(txns, id);
        for (pid, slots) in dead.pages(id) {
            let done = heap.prune(pager, pid, |p, h| judge(txns, p, w, h), &slots, true)?;
            report.freed += done.freed;
            report.cleared += done.cleared;
            report.all_visible += usize::from(done.all_visible);
            report.removed += usize::from(done.removed);
            progress.set(id, |s| {
                s.dead_versions = s.dead_versions.saturating_sub(slots.len());
            });
        }
        progress.set(id, |s| s.phase = Phase::Scan);
    }
    dead.clear();
    Ok(())
}
