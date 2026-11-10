use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::btree::{self, Index};
use crate::heap::{self, Heap, LargeStore};
use crate::history::{self, History};
use crate::key::CtorId;
use crate::mvcc::{Horizon, Rule, Transactions, Xid};
use crate::runtime::Runtime;
use crate::storage::{PageError, Pager, Walk};

use super::dead::DeadSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Page(PageError),
    Index(btree::Error),
    /// A version could not be written to a history heap.
    Heap(heap::Error),
    /// The vacuum's pace stopped it.
    Stopped,
}

impl From<heap::Error> for Error {
    fn from(e: heap::Error) -> Error {
        Error::Heap(e)
    }
}

impl From<heap::Refused> for Error {
    fn from(e: heap::Refused) -> Error {
        Error::Heap(e.into())
    }
}

impl From<history::Error> for Error {
    fn from(e: history::Error) -> Error {
        match e {
            history::Error::Page(e) => Error::Page(e),
            history::Error::Index(e) => Error::Index(e),
            history::Error::Heap(e) => Error::Heap(e),
        }
    }
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

/// What one run of [`vacuum`] did to one heap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Freed {
    /// Heap slots emptied.
    pub freed: usize,
    /// Heap pages that left the heap.
    pub removed: usize,
}

/// What one run of [`vacuum`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// Versions moved to history heaps.
    pub moved: usize,
    /// Rounds of index cleanup and pruning.
    pub rounds: usize,
    /// No version of the sum names an aborted transaction below this id, nor has one as its
    /// deleter.
    pub clean_below: Xid,
    /// What it did to each heap that freed something, by heap id.
    pub heaps: BTreeMap<u32, Freed>,
}

/// Prune page `pid` of `heap` of the versions dead to every snapshot its watermark admits,
/// leaving its all-visible bit as it is unless the page changes.
#[cfg(test)]
pub fn prune<R: Runtime>(
    pager: &Pager<R>,
    txns: &Transactions<R>,
    heap: &Heap,
    pid: crate::storage::Pid,
) -> Result<heap::Pruned, heap::Refused> {
    let judge = txns.judge(pager, txns.horizon_now(heap.id().0));
    heap.prune(pager, pid, judge, &[], false)
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
    /// Finished: every dead version it found is freed.
    Done,
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
    #[cfg(test)]
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

    /// Show `heap` as a vacuum that has just begun.
    pub fn start(&self, heap: u32) {
        self.lock().insert(
            heap,
            Step {
                heap,
                phase: Phase::Scan,
                done: 0.0,
                dead_versions: 0,
            },
        );
    }

    fn set(&self, heap: u32, f: impl FnOnce(&mut Step)) {
        if let Some(row) = self.lock().get_mut(&heap) {
            f(row);
        }
    }

    fn finish(&self, heap: u32) {
        self.set(heap, |s| {
            s.phase = Phase::Done;
            s.done = 1.0;
            s.dead_versions = 0;
        });
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
/// phase, and `Done` once the closing prune of its pages ends. `pace` is called before each heap
/// page scanned, moved to history or pruned, and after each index leaf, store chunk and store
/// page, holding no latch. Once it returns false the vacuum fails with `Stopped` and frees
/// nothing more.
///
/// Precondition: no other vacuum of the sum, or of a sum sharing its large-value store, runs.
///
/// Versions the retention window keeps stay where they are.
///
/// Panics if the heaps do not share one large-value store.
pub fn vacuum<R: Runtime>(
    pager: &Pager<R>,
    txns: &Transactions<R>,
    sum: &Sum,
    budget: usize,
    progress: &Progress,
    pace: &mut dyn FnMut() -> bool,
) -> Result<Report, Error> {
    let mut pass = Pass {
        progress,
        pace,
        last: false,
    };
    let marks = Watermarks::new(Mode::Current(None), txns.cutoff());
    run(pager, txns, sum, marks, budget, &mut pass)
}

/// Vacuum `sum` as [`vacuum`] does, moving each version the retention window keeps to the
/// history heap of its constructor in `into` before freeing it.
///
/// Preconditions: `into` has a history heap for every constructor of `sum`, and no
/// [`expire`] of those heaps runs.
pub fn vacuum_keeping<R: Runtime>(
    pager: &Pager<R>,
    txns: &Transactions<R>,
    sum: &Sum,
    into: &History,
    budget: usize,
    progress: &Progress,
    pace: &mut dyn FnMut() -> bool,
) -> Result<Report, Error> {
    let mut pass = Pass {
        progress,
        pace,
        last: false,
    };
    let marks = Watermarks::new(Mode::Current(Some(into)), txns.cutoff());
    run(pager, txns, sum, marks, budget, &mut pass)
}

/// Vacuum `past`, the history heaps of a sum and the history key index over them, of every
/// version the retention window no longer keeps, as [`vacuum`] does, and then forget the commit
/// times and markers before the window.
pub fn expire<R: Runtime>(
    pager: &Pager<R>,
    txns: &Transactions<R>,
    past: &Sum,
    budget: usize,
    progress: &Progress,
    pace: &mut dyn FnMut() -> bool,
) -> Result<Report, Error> {
    let cutoff = txns.cutoff();
    let mut pass = Pass {
        progress,
        pace,
        last: false,
    };
    let marks = Watermarks::new(Mode::Past, cutoff);
    let report = run(pager, txns, past, marks, budget, &mut pass)?;
    if let (Some(record), Some(cutoff)) = (txns.history(), cutoff) {
        finished(record.expire(pager, cutoff, pass.pace)?)?;
    }
    Ok(report)
}

/// What one run reports its progress to, and what paces it.
struct Pass<'p> {
    progress: &'p Progress,
    pace: &'p mut dyn FnMut() -> bool,
    /// Whether the clean running is the one after the scan ends.
    last: bool,
}

impl Pass<'_> {
    /// Pace the run, or fail it once the pace stops it.
    fn step(&mut self) -> Result<(), Error> {
        if (self.pace)() {
            Ok(())
        } else {
            Err(Error::Stopped)
        }
    }
}

/// Returns the count of a walk that finished, or fails the run if it stopped.
fn finished(walk: Walk) -> Result<usize, Error> {
    match walk {
        Walk::Done(n) => Ok(n),
        Walk::Stopped(_) => Err(Error::Stopped),
    }
}

fn run<R: Runtime>(
    pager: &Pager<R>,
    txns: &Transactions<R>,
    sum: &Sum,
    mut marks: Watermarks,
    budget: usize,
    pass: &mut Pass,
) -> Result<Report, Error> {
    let progress = pass.progress;
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
        moved: 0,
        rounds: 0,
        clean_below: txns.running().first().copied().unwrap_or(txns.next()),
        heaps: BTreeMap::new(),
    };
    let mut dead = DeadSet::new();
    for (_, heap) in &sum.heaps {
        progress.start(heap.id().0);
    }
    for (_, heap) in &sum.heaps {
        let id = heap.id().0;
        let pages = heap.pages(pager)?;
        for (n, pid) in pages.iter().enumerate() {
            pass.step()?;
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
            let slots = heap.dead_slots(pager, *pid, txns.judge(pager, w))?;
            report.dead += slots.len();
            progress.set(id, |s| s.dead_versions += slots.len());
            dead.add(id, *pid, &slots);
            if dead.bytes() >= budget {
                clean(pager, txns, sum, &mut dead, &mut marks, &mut report, pass)?;
            }
        }
        progress.set(id, |s| s.done = 1.0);
    }
    pass.last = true;
    clean(pager, txns, sum, &mut dead, &mut marks, &mut report, pass)?;
    report.chunks = finished(large.sweep(pager, |x| txns.resolved(pager, x), pass.pace)?)?;
    Ok(report)
}

/// Which versions a run of vacuum frees.
#[derive(Clone, Copy)]
enum Mode<'a> {
    /// Of current heaps: those dead to every snapshot, except those the retention window keeps,
    /// which move to the history heaps given or else stay.
    Current(Option<&'a History<'a>>),
    /// Of history heaps: those the retention window no longer keeps.
    Past,
}

/// The horizon each heap is judged by for the whole of one vacuum, fixed at its first judgement
/// of the heap, with the window's cutoff taken when the vacuum starts.
struct Watermarks<'a> {
    mode: Mode<'a>,
    cutoff: Option<u64>,
    fixed: BTreeMap<u32, Horizon>,
}

impl<'a> Watermarks<'a> {
    fn new(mode: Mode<'a>, cutoff: Option<u64>) -> Self {
        Watermarks {
            mode,
            cutoff,
            fixed: BTreeMap::new(),
        }
    }

    /// Returns the history heaps the versions the window keeps move to, if they move.
    fn moves_to(&self) -> Option<&'a History<'a>> {
        match self.mode {
            Mode::Current(into) => into,
            Mode::Past => None,
        }
    }

    fn of<R: Runtime>(&mut self, txns: &Transactions<R>, heap: u32) -> Horizon {
        let rule = match self.mode {
            Mode::Current(None) => Rule::Current,
            Mode::Current(Some(_)) => Rule::Moving,
            Mode::Past => Rule::Past,
        };
        let cutoff = self.cutoff;
        *self
            .fixed
            .entry(heap)
            .or_insert_with(|| txns.horizon_fixed(heap, cutoff, rule))
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

/// Copy the versions of `dead` the retention window keeps to their history heaps, when the rule
/// moves them, then delete the index entries of every TID in `dead`, then prune each of its pages,
/// freeing those TIDs, and empty `dead`. On the pass's last clean, finish each heap's progress
/// once its pages are pruned.
fn clean<R: Runtime>(
    pager: &Pager<R>,
    txns: &Transactions<R>,
    sum: &Sum,
    dead: &mut DeadSet,
    marks: &mut Watermarks,
    report: &mut Report,
    pass: &mut Pass,
) -> Result<(), Error> {
    let (progress, last) = (pass.progress, pass.last);
    if dead.is_empty() {
        for (_, heap) in sum.heaps.iter().filter(|_| last) {
            progress.finish(heap.id().0);
        }
        return Ok(());
    }
    report.rounds += 1;
    for (_, heap) in &sum.heaps {
        progress.set(heap.id().0, |s| s.phase = Phase::Indexes);
    }
    if let Some(into) = marks.moves_to() {
        for &(ctor, heap) in &sum.heaps {
            let w = marks.of(txns, heap.id().0);
            for (pid, _) in dead.pages(heap.id().0) {
                pass.step()?;
                report.moved += history::keep(pager, txns, heap, ctor, into, &w, pid)?;
            }
        }
    }
    let heap_of: BTreeMap<CtorId, u32> = sum.heaps.iter().map(|(c, h)| (*c, h.id().0)).collect();
    for index in &sum.indexes {
        let keep = |e: &crate::key::Entry| {
            !heap_of
                .get(&e.ctor)
                .is_some_and(|&heap| dead.contains(heap, e.tid))
        };
        report.entries += finished(index.retain(pager, keep, pass.pace)?)?;
    }
    for &(_, heap) in &sum.heaps {
        let id = heap.id().0;
        progress.set(id, |s| s.phase = Phase::Prune);
        let w = marks.of(txns, id);
        for (pid, slots) in dead.pages(id) {
            pass.step()?;
            let done = heap.prune(pager, pid, txns.judge(pager, w), &slots, true)?;
            report.freed += done.freed;
            report.cleared += done.cleared;
            report.all_visible += usize::from(done.all_visible);
            report.removed += usize::from(done.removed);
            let heap = report.heaps.entry(id).or_default();
            heap.freed += done.freed;
            heap.removed += usize::from(done.removed);
            progress.set(id, |s| {
                s.dead_versions = s.dead_versions.saturating_sub(slots.len());
            });
        }
        if last {
            progress.finish(id);
        } else {
            progress.set(id, |s| s.phase = Phase::Scan);
        }
    }
    dead.clear();
    Ok(())
}
