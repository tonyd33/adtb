//! The vacuum running on a database, as `sys::vacuums` shows it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use adtb_lang::types::RelId;

use adtb_sys::lock::Lock;

use crate::exec::{F64, Val};
use crate::key::CtorId;
use crate::mvcc::Xid;
use crate::plan::HeapId;
use crate::runtime::{Instant, Io, Op, Runtime, ThreadId};
use crate::vacuum::{Pacer, Phase, Progress, Step};

use super::{Database, Error, Stop};

pub(super) const VACUUMS: u64 = 334;

/// The longest a vacuum sleeps before it looks again at what stops it.
const NAP: Duration = Duration::from_millis(100);

/// One constructor relation of the running vacuum.
#[derive(Debug)]
struct Shown {
    id: i64,
    ctor: CtorId,
    heap: u32,
}

struct Running {
    relation: u64,
    started: i64,
    ctors: Vec<Shown>,
}

/// The vacuum a database runs, one at a time under its vacuum lock.
#[derive(Default)]
pub(super) struct Vacuums {
    pub(super) progress: Progress,
    running: Lock<Option<Running>>,
    next_id: AtomicI64,
}

/// Shows a vacuum in `sys::vacuums` until dropped, an error or a panic included.
pub(super) struct ShownVacuum<'d, R: Runtime> {
    db: &'d Database<R>,
}

impl<R: Runtime> Drop for ShownVacuum<'_, R> {
    fn drop(&mut self) {
        *self.db.vacuums.running.lock(&self.db.rt) = None;
    }
}

fn phase_text(phase: Phase) -> &'static str {
    match phase {
        Phase::Scan => "scan",
        Phase::Indexes => "indexes",
        Phase::Prune => "prune",
        Phase::Done => "done",
    }
}

impl Shown {
    fn row(&self, step: Option<&Step>, relation: u64, started: i64) -> Val {
        let key = |n: u64| Val::Int(i64::try_from(n).unwrap_or(i64::MAX));
        let (phase, done, dead) = step.map_or(("scan", 0.0, 0), |s| {
            (phase_text(s.phase), s.done, s.dead_versions)
        });
        Val::ctor(
            0,
            vec![
                Val::Int(self.id),
                key(relation),
                key(u64::from(self.ctor.0)),
                Val::text(phase),
                Val::F64(F64::new(done)),
                Val::Int(i64::try_from(dead).unwrap_or(i64::MAX)),
                Val::Instant(started),
            ],
        )
    }
}

impl<R: Runtime> Database<R> {
    /// Returns the budget every vacuum and the scrub run within.
    pub fn pacer(&self) -> &Pacer {
        &self.pacer
    }

    /// Charge the vacuum step that began at `began` to the pacer and sleep as it says on `io`,
    /// looking at `stop` and for a close at least every `NAP` and on each unpark; at a budget of
    /// 0, until one of them fails the vacuum. Sets `began` to the end of the sleep, failed or not.
    pub(super) fn pace(
        &self,
        stop: &Stop,
        began: &mut Instant,
        io: &mut Option<R::Io>,
    ) -> Result<(), Error> {
        let until = self
            .pacer
            .charge(&self.rt, *began)
            .map(|gap| self.rt.now() + gap);
        let slept = self.sleep_until(stop, until, io);
        *began = self.rt.now();
        slept
    }

    fn sleep_until(
        &self,
        stop: &Stop,
        until: Option<Instant>,
        io: &mut Option<R::Io>,
    ) -> Result<(), Error> {
        loop {
            self.rebuild_stopped()?;
            stop.check(self.rt.now())?;
            let left = until.map_or(NAP, |at| at.saturating_since(self.rt.now()));
            if left.is_zero() {
                return Ok(());
            }
            let io = io.get_or_insert_with(|| self.rt.io());
            io.submit(Op::Timer {
                at: self.rt.now() + left.min(NAP),
            });
            self.rt.park();
            while io.poll().is_some() {}
        }
    }

    /// Show a vacuum of the constructor relations `ctors`, each its id with its heap, of the
    /// relation whose catalog key is `relation`, until the guard returned drops.
    pub(super) fn show_vacuum(&self, relation: u64, ctors: &[(CtorId, u32)]) -> ShownVacuum<'_, R> {
        let started = self
            .pager
            .wall()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_micros()).unwrap_or(i64::MAX));
        let ctors = ctors
            .iter()
            .map(|&(ctor, heap)| {
                self.vacuums.progress.start(heap);
                Shown {
                    id: self.vacuums.next_id.fetch_add(1, Ordering::Relaxed),
                    ctor,
                    heap,
                }
            })
            .collect();
        *self.vacuums.running.lock(&self.rt) = Some(Running {
            relation,
            started,
            ctors,
        });
        ShownVacuum { db: self }
    }

    fn vacuum_steps(&self) -> BTreeMap<u32, Step> {
        self.vacuums
            .progress
            .rows()
            .into_iter()
            .map(|s| (s.heap, s))
            .collect()
    }

    /// Returns the rows of `sys::vacuums`.
    pub(super) fn vacuum_rows(&self) -> BTreeSet<Val> {
        let steps = self.vacuum_steps();
        let running = self.vacuums.running.lock(&self.rt);
        let Some(r) = running.as_ref() else {
            return BTreeSet::new();
        };
        (r.ctors.iter())
            .map(|s| s.row(steps.get(&s.heap), r.relation, r.started))
            .collect()
    }
}

/// The least dead versions that make a relation due a background vacuum.
const LEAST_DEAD: u64 = 50;

/// The versions a transaction's statements changed in one heap.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Change {
    deleted: u64,
    inserted: u64,
    /// Inserted by statements that failed or were taken back.
    pub(super) undone: u64,
}

pub(super) type Changed = BTreeMap<HeapId, Change>;

/// What makes each relation due a background vacuum.
#[derive(Default)]
pub(super) struct Dead {
    /// The versions made dead in each heap and not yet freed by a vacuum.
    heaps: BTreeMap<HeapId, u64>,
    /// The pages of each relation's heaps after its last finished vacuum. A relation absent has
    /// not been vacuumed since open and is due.
    pages: BTreeMap<RelId, u64>,
    /// The watermark each relation's last finished vacuum began under. Its dead versions make
    /// it due only once the watermark has passed it.
    horizon: BTreeMap<RelId, Xid>,
    /// When the last finished vacuum of each relation that left versions in history began.
    kept: BTreeMap<RelId, Instant>,
    worker: Option<ThreadId>,
    /// Whether the worker found nothing due and sleeps.
    idle: bool,
}

impl Dead {
    /// Count the versions `changed` made dead: those it deleted on a commit, those it inserted
    /// on an abort, and those taken back either way.
    pub(super) fn add(&mut self, changed: &Changed, committed: bool) {
        for (heap, c) in changed {
            let dead = if committed { c.deleted } else { c.inserted };
            *self.heaps.entry(*heap).or_default() += dead + c.undone;
        }
    }
}

/// A relation due a background vacuum, else when the next one is due by the clock.
#[derive(Debug, PartialEq, Eq)]
pub enum Due {
    Now(RelId),
    At(Option<Instant>),
}

impl<R: Runtime> Database<R> {
    /// Add to `changed` the versions the outcome `o` of a statement deleted and inserted.
    pub(super) fn record_changed(&self, changed: &mut Changed, o: &super::Outcome) {
        let heap = |r: &RelId, c: &u32| self.rels.get(r).and_then(|rel| rel.ctors.get(*c as usize));
        for ((r, c), rows) in &o.deleted {
            if let Some(ctor) = heap(r, c) {
                changed.entry(ctor.heap).or_default().deleted += rows.len() as u64;
            }
        }
        for ((r, c), rows) in &o.inserted {
            if let Some(ctor) = heap(r, c) {
                changed.entry(ctor.heap).or_default().inserted += rows.len() as u64;
            }
        }
    }

    /// Count the versions `changed` made dead: those it deleted on a commit, those it inserted
    /// on an abort, and those taken back either way. Unpark the background worker if it is idle
    /// and a relation is now due.
    pub(super) fn add_dead(&self, changed: &Changed, committed: bool) {
        if changed.is_empty() {
            return;
        }
        let mut d = self.session.dead.lock(&self.rt);
        d.add(changed, committed);
        let due = (self.rels.values())
            .filter(|r| r.ctors.iter().any(|c| changed.contains_key(&c.heap)))
            .any(|r| self.dead_due(&d, r));
        if due && d.idle {
            d.idle = false;
            if let Some(t) = d.worker {
                self.rt.unpark(t);
            }
        }
    }

    fn dead_due(&self, d: &Dead, rel: &super::schema::Rel) -> bool {
        let Some(&pages) = d.pages.get(&rel.id) else {
            return true;
        };
        let count: u64 = (rel.ctors.iter())
            .map(|c| d.heaps.get(&c.heap).copied().unwrap_or(0))
            .sum();
        count >= pages.max(LEAST_DEAD) && d.horizon.get(&rel.id) < Some(&self.horizon(rel))
    }

    /// Returns the least watermark of the heaps of `rel`.
    pub(super) fn horizon(&self, rel: &super::schema::Rel) -> Xid {
        (rel.ctors.iter())
            .map(|c| self.session.txns.watermark(self.heaps[&c.heap].heap.id().0))
            .min()
            .unwrap_or_else(|| self.session.txns.next())
    }

    /// Unpark `worker` when a commit or an abort makes a relation due while it is idle.
    pub fn watch_dead(&self, worker: ThreadId) {
        self.session.dead.lock(&self.rt).worker = Some(worker);
    }

    /// Returns the first relation outside `skipped` after `last`, in turn, due a background
    /// vacuum at `now`: by its dead versions, or, under a retention window of `window`, by a
    /// quarter of the window passing since its last vacuum that left history began. Marks the
    /// worker idle when none is due.
    pub fn due_vacuum(
        &self,
        window: Duration,
        skipped: &BTreeSet<RelId>,
        last: Option<RelId>,
    ) -> Due {
        let mut d = self.session.dead.lock(&self.rt);
        let now = self.rt.now();
        let mut next = None;
        let (after, before): (Vec<_>, Vec<_>) = (self.rels.values())
            .filter(|r| !skipped.contains(&r.id))
            .partition(|r| last.is_some_and(|l| r.id > l));
        for rel in after.into_iter().chain(before) {
            let clock = d.kept.get(&rel.id).map(|&at| at + window / 4);
            if self.dead_due(&d, rel) || clock.is_some_and(|at| at <= now) {
                d.idle = false;
                return Due::Now(rel.id);
            }
            next = [next, clock].into_iter().flatten().min();
        }
        d.idle = true;
        Due::At(next)
    }

    /// Record a vacuum of `rel` that began at `began` under watermark `horizon`, freed `freed`
    /// versions of each heap, left `pages` pages in its heaps, and left versions in history if
    /// `kept`.
    pub(super) fn vacuumed(
        &self,
        rel: RelId,
        began: Instant,
        horizon: Xid,
        freed: &BTreeMap<HeapId, u64>,
        pages: u64,
        kept: bool,
    ) {
        let mut d = self.session.dead.lock(&self.rt);
        for (heap, n) in freed {
            if let Some(count) = d.heaps.get_mut(heap) {
                *count = count.saturating_sub(*n);
            }
        }
        d.pages.insert(rel, pages);
        d.horizon.insert(rel, horizon);
        if kept {
            d.kept.insert(rel, began);
        } else {
            d.kept.remove(&rel);
        }
    }
}
