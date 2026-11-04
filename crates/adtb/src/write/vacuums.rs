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

/// How far the vacuum of one constructor relation has got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stage {
    Waiting,
    Current,
    Done,
}

/// One constructor relation of the running vacuum, and its row as last read.
#[derive(Debug)]
struct Shown {
    id: i64,
    ctor: CtorId,
    heap: u32,
    stage: Stage,
    phase: &'static str,
    done: f64,
    dead_versions: usize,
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
        Phase::Sweep => "sweep",
    }
}

impl Shown {
    fn new(id: i64, ctor: CtorId, heap: u32) -> Shown {
        Shown {
            id,
            ctor,
            heap,
            stage: Stage::Waiting,
            phase: "waiting",
            done: 0.0,
            dead_versions: 0,
        }
    }

    /// Take the step of its heap in `steps`, if it runs. A heap whose step has gone keeps the
    /// last one taken.
    fn take(&mut self, steps: &BTreeMap<u32, Step>) {
        let Some(s) = steps
            .get(&self.heap)
            .filter(|_| self.stage == Stage::Current)
        else {
            return;
        };
        self.phase = phase_text(s.phase);
        self.done = self.done.max(s.done);
        self.dead_versions = s.dead_versions;
    }

    fn enter(&mut self, stage: Stage, steps: &BTreeMap<u32, Step>) {
        self.take(steps);
        self.stage = stage;
        match stage {
            Stage::Waiting => {}
            Stage::Current => self.phase = "scan",
            Stage::Done => {
                self.phase = "done";
                self.done = 1.0;
                self.dead_versions = 0;
            }
        }
    }

    fn row(&self, relation: u64, started: i64) -> Val {
        let key = |n: u64| Val::Int(i64::try_from(n).unwrap_or(i64::MAX));
        Val::ctor(
            0,
            vec![
                Val::Int(self.id),
                key(relation),
                key(u64::from(self.ctor.0)),
                Val::text(self.phase),
                Val::F64(F64::new(self.done)),
                Val::Int(i64::try_from(self.dead_versions).unwrap_or(i64::MAX)),
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

    /// Charge the vacuum step that began at `began` to the pacer and sleep as it says, looking at
    /// `stop` and for a close at least every `NAP`; at a budget of 0, until one of them fails
    /// the vacuum, on one I/O queue however long it sleeps. Sets `began` to the end of the
    /// sleep.
    pub(super) fn pace(&self, stop: &Stop, began: &mut Instant) -> Result<(), Error> {
        let until = self
            .pacer
            .charge(&self.rt, *began)
            .map(|gap| self.rt.now() + gap);
        let mut io = None;
        loop {
            self.rebuild_stopped()?;
            stop.check(self.rt.now())?;
            let left = until.map_or(NAP, |at| at.saturating_since(self.rt.now()));
            if left.is_zero() {
                break;
            }
            self.nap(io.get_or_insert_with(|| self.rt.io()), left.min(NAP));
        }
        *began = self.rt.now();
        Ok(())
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
                Shown::new(
                    self.vacuums.next_id.fetch_add(1, Ordering::Relaxed),
                    ctor,
                    heap,
                )
            })
            .collect();
        *self.vacuums.running.lock(&self.rt) = Some(Running {
            relation,
            started,
            ctors,
        });
        ShownVacuum { db: self }
    }

    /// Move the vacuum of constructor relation `ctor` to `stage`.
    pub(super) fn vacuum_stage(&self, ctor: CtorId, stage: Stage) {
        let steps = self.vacuum_steps();
        let mut running = self.vacuums.running.lock(&self.rt);
        if let Some(shown) = running
            .as_mut()
            .and_then(|r| r.ctors.iter_mut().find(|s| s.ctor == ctor))
        {
            shown.enter(stage, &steps);
        }
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
        let mut running = self.vacuums.running.lock(&self.rt);
        let Some(r) = running.as_mut() else {
            return BTreeSet::new();
        };
        r.ctors
            .iter_mut()
            .map(|s| {
                s.take(&steps);
                s.row(r.relation, r.started)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(phase: Phase, done: f64, dead_versions: usize) -> BTreeMap<u32, Step> {
        BTreeMap::from([(
            7,
            Step {
                heap: 7,
                phase,
                done,
                dead_versions,
            },
        )])
    }

    #[test]
    fn a_finished_constructor_relation_has_nothing_left_whatever_it_last_read() {
        let mut s = Shown::new(0, CtorId(1025), 7);
        s.enter(Stage::Current, &BTreeMap::new());
        s.take(&steps(Phase::Prune, 0.5, 5));
        assert_eq!(("prune", 0.5, 5), (s.phase, s.done, s.dead_versions));
        s.enter(Stage::Done, &BTreeMap::new());
        assert_eq!(("done", 1.0, 0), (s.phase, s.done, s.dead_versions));
    }

    #[test]
    fn a_waiting_constructor_relation_takes_no_step_of_its_heap() {
        let mut s = Shown::new(0, CtorId(1025), 7);
        s.take(&steps(Phase::Prune, 0.5, 5));
        assert_eq!(("waiting", 0.0, 0), (s.phase, s.done, s.dead_versions));
    }
}
