//! The vacuum running on a database, as `sys::vacuums` shows it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use adtb_sys::lock::Lock;

use crate::exec::{F64, Val};
use crate::key::CtorId;
use crate::runtime::{Instant, Runtime};
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
    /// looking at `stop` and for a close at least every `NAP`; at a budget of 0, until one of
    /// them fails the vacuum. Sets `began` to the end of the sleep, failed or not.
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
            self.nap(io.get_or_insert_with(|| self.rt.io()), left.min(NAP));
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
