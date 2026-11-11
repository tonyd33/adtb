//! The rebuild of one index: runs of entries from each heap, a new tree, a catch-up with writes
//! made meanwhile, and the swap into the root page that never moves.
//!
//! Every physical version enters the new tree, whichever snapshot sees it.
//!
//! The last catch-up holds the index's latch from before it reads the heaps until the swap. A
//! writer writes a version before its entry and holds that latch across the entry, so a version
//! the catch-up misses has its entry inserted after the swap, into the new tree.
//!
//! SIREAD locks on an index are key ranges on its root page, whose id the swap keeps.

use std::any::Any;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use adtb_lang::types::RelId;

use crate::btree::build::{BuildError, RunEntry, merge_runs, retire};
use crate::btree::{self, BTree};
use crate::exec;
use crate::heap::Heap;
use crate::key::{CtorId, Entry};
use crate::mvcc::Xid;
use crate::plan::IndexId;
use crate::runtime::{Instant, Io, Op, Runtime, ThreadId};
use crate::storage::{self, Damage, Lsn, Pid, Quarantine};

use super::past::Retained;
use super::schema::{Indexed, Rel};
use super::{Bounds, Broken, Database, Error, Rerun, Stop, Txn};

/// One index to rebuild, how, and who follows it.
pub struct Rebuild<'a> {
    pub rel: RelId,
    pub index: IndexId,
    pub online: bool,
    /// Told each phase as it starts, and the fraction of the rebuild done.
    pub phase: &'a dyn Fn(&str, f64),
}

/// Why a rebuild kept the old index.
#[derive(Debug)]
pub struct Failure {
    /// What went wrong, `None` for a rebuild stopped by its caller.
    pub cause: Option<String>,
    /// Whether every page it allocated is free again.
    pub reclaimed: bool,
}

/// Returns the text of a `String` or `&str` panic payload.
pub fn panic_text(payload: &(dyn Any + Send)) -> Option<String> {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
}

/// How many catch-ups run while writes continue before the one that holds them.
const CATCH_UPS: usize = 2;

/// The transaction a statement gated by an offline rebuild waits for.
pub(super) const OFFLINE_GATE: Xid = Xid(u64::MAX);

/// The transaction a call waiting for the vacuum lock waits for: running while a holder has it.
pub(super) const VACUUM_HOLDER: Xid = Xid(u64::MAX - 1);

/// How often a statement that cannot suspend looks at what it waits for and at what stops it.
pub(super) const GATE_POLL: Duration = Duration::from_millis(5);

/// What stops a call that waits for the vacuum lock.
#[derive(Clone, Copy, Debug)]
pub struct HoldWait<'b> {
    bounds: Option<&'b Bounds>,
    /// The position among the call's items of the command that waits.
    at: usize,
}

impl<'b> HoldWait<'b> {
    /// Returns the wait of the command at `at`, stopped by `bounds`.
    pub fn of(bounds: &'b Bounds, at: usize) -> HoldWait<'b> {
        HoldWait {
            bounds: Some(bounds),
            at,
        }
    }

    /// Returns a wait only a close stops.
    pub fn unbounded() -> HoldWait<'static> {
        HoldWait {
            bounds: None,
            at: 0,
        }
    }

    /// Returns what stops the wait, from `now`.
    pub(super) fn stop(&self, now: Instant) -> Stop {
        let Some(bounds) = self.bounds else {
            return Stop::default();
        };
        let rerun = match bounds.rerun {
            Some(r) if self.at <= r.through => r,
            _ => Rerun {
                through: self.at,
                since: now,
            },
        };
        let limited = self.at >= rerun.through;
        Stop::of(
            bounds,
            limited.then_some(rerun.since),
            bounds.suspend.then_some(rerun),
        )
    }
}

/// The holder of the vacuum lock.
pub(super) enum Holder {
    /// One that keeps the lock until it is done.
    Keeps,
    /// One that gives the lock up once `asked` is set, looking at it each time `thread` unparks.
    Yields {
        asked: Arc<AtomicBool>,
        thread: ThreadId,
    },
}

/// The vacuum lock. Holds off vacuum, the one other writer that removes entries and frees slots,
/// every other rebuild, and every other holder.
pub struct VacuumHold<'d, R: Runtime> {
    db: &'d Database<R>,
}

impl<R: Runtime> Drop for VacuumHold<'_, R> {
    fn drop(&mut self) {
        *self.db.vacuuming.lock(&self.db.rt) = None;
        self.db.session.waits.ended(VACUUM_HOLDER);
    }
}

impl<R: Runtime> std::fmt::Debug for VacuumHold<'_, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VacuumHold")
    }
}

/// What a rebuild has allocated and replaced so far.
struct Pages {
    built: Vec<Pid>,
    built_past: Vec<Pid>,
    old: Replaced,
}

/// The pages of the trees a rebuild has replaced.
struct Replaced {
    /// Each tree's pages in the order [`retire`] asks.
    pages: Vec<Pid>,
    /// Whether `pages` holds every page of those trees.
    found: bool,
}

/// Remove a relation from the offline set and open the gate when dropped, a panic included.
struct Offline<'d, R: Runtime> {
    db: &'d Database<R>,
    rel: RelId,
}

impl<R: Runtime> Drop for Offline<'_, R> {
    fn drop(&mut self) {
        self.db.offline.lock(&self.db.rt).remove(&self.rel);
        self.db.session.waits.ended(OFFLINE_GATE);
    }
}

/// How the rows of one index's constructors give its entries.
pub(super) struct Scan<'d, R: Runtime> {
    pub(super) db: &'d Database<R>,
    pub(super) rel: &'d Rel,
    pub(super) id: IndexId,
    pub(super) ix: &'d Indexed,
}

impl<R: Runtime> Scan<'_, R> {
    /// Returns a sorted run of the entries of every version in the constructor's heap, on pages
    /// changed at or after `since`. An entry counts when `tx` sees its version.
    ///
    /// Fail with `Cancelled` once rebuilds are stopped.
    fn run(
        &self,
        heap: &Heap,
        (ctor, c): (CtorId, u32),
        since: Option<Lsn>,
        tx: Option<&Txn>,
    ) -> Result<Vec<RunEntry>, Error> {
        let db = self.db;
        let stored = &db.bindings.indexes()[&self.id];
        let schema = stored.index.schema();
        let types: Vec<_> = schema.components().iter().map(|(t, _)| t.clone()).collect();
        let reader = tx.map(|tx| db.session.txns.reader(&db.pager, tx.hold(), None));
        let mut out = Vec::new();
        for pid in heap.pages(&db.pager)? {
            db.rebuild_stopped()?;
            if let Some(since) = since
                && storage::lsn(&db.pager.read(pid)?) < since
            {
                continue;
            }
            let mut versions = Vec::new();
            heap.scan_rooted(&db.pager, pid, |v, roots| versions.push((v, roots)))?;
            for (v, roots) in versions {
                let Some(key) = self.ix.keys.key(c, &v.row, &types)? else {
                    continue;
                };
                let counted = match &reader {
                    Some(r) => r
                        .sees(&v.header.writers)
                        .map_err(|e| v.refuse(&db.pager, e))?
                        .visible(),
                    None => false,
                };
                let key_len = schema.encode(&key).len();
                for root in roots {
                    let entry = schema.encode_entry(&Entry {
                        key: key.clone(),
                        ctor,
                        tid: root,
                    });
                    out.push(RunEntry {
                        entry,
                        key_len,
                        counted,
                    });
                }
            }
        }
        out.sort_unstable();
        Ok(out)
    }

    pub(super) fn runs(
        &self,
        since: Option<Lsn>,
        tx: Option<&Txn>,
    ) -> Result<Vec<Vec<RunEntry>>, Error> {
        self.db.bindings.indexes()[&self.id]
            .ctors
            .iter()
            .map(|(&ctor, &c)| {
                let heap = &self.db.bindings.heaps()[&self.rel.ctors[c as usize].heap].heap;
                self.run(heap, (ctor, c), since, tx)
            })
            .collect()
    }

    /// Returns a sorted run of the entries of every version in each history heap of `retained`.
    fn past_runs(&self, retained: &Retained) -> Result<Vec<Vec<RunEntry>>, Error> {
        self.db.bindings.indexes()[&self.id]
            .ctors
            .iter()
            .map(|(&ctor, &c)| {
                let heap = &retained.heaps[&self.rel.ctors[c as usize].heap].heap;
                self.run(heap, (ctor, c), None, None)
            })
            .collect()
    }

    /// Insert into `tree` the entries of versions on pages changed at or after `since`. Returns
    /// the log's end when it started.
    fn catch_up(&self, tree: BTree, since: Lsn) -> Result<Lsn, Error> {
        let end = self.db.pager.appended();
        for run in self.runs(Some(since), None)? {
            for e in run {
                tree.insert(&self.db.pager, &e.entry)?;
            }
        }
        Ok(end)
    }
}

impl<R: Runtime> Database<R> {
    /// Returns the indexes of `rel` at `positions` among its declared indexes, or every index of
    /// it, its key index included, when `None`.
    pub fn reindexed_ids(
        &self,
        rel: RelId,
        positions: Option<&[usize]>,
    ) -> Result<Vec<IndexId>, Error> {
        let r = (self.rels.get(&rel))
            .unwrap_or_else(|| panic!("a relation a reindex names has heaps: {rel:?} has none"));
        let Some(positions) = positions else {
            return Ok(r.indexes.iter().map(|(id, _)| *id).collect());
        };
        let mut ids = BTreeSet::new();
        for &i in positions {
            let id = (self.schema.ids.indexes.get(&(rel, i)))
                .unwrap_or_else(|| panic!("index {i} of {} is stored", r.name));
            ids.insert(IndexId(*id));
        }
        Ok(ids.into_iter().collect())
    }

    /// Fail if a rebuild over `rel` cannot start: the database or a heap of `rel` is quarantined.
    /// A quarantined index does not stop it, as a build reads only heaps.
    pub fn admit_reindex(&self, rel: RelId) -> Result<(), Error> {
        self.admit_database()?;
        let Some(r) = self.rels.get(&rel) else {
            return Ok(());
        };
        let marks = self.pager.quarantined();
        for c in &r.ctors {
            let object = Quarantine::Heap(self.bindings.heaps()[&c.heap].heap.id().0);
            if marks.contains(&object) {
                return Err(Error::admitted(object));
            }
        }
        Ok(())
    }

    /// Returns the vacuum lock for a rebuild job, waiting as long as another holds it.
    pub fn holding_off_vacuum(&self) -> VacuumHold<'_, R> {
        loop {
            if let Some(hold) = self.try_hold_vacuum() {
                return hold;
            }
            self.pause_for_rebuild();
        }
    }

    /// Returns the vacuum lock unless another holds it. [`VACUUM_HOLDER`] runs exactly while one
    /// does, so a call that watches it and finds it ended finds the lock free.
    fn try_hold_vacuum(&self) -> Option<VacuumHold<'_, R>> {
        self.try_hold(Holder::Keeps)
    }

    /// Returns the vacuum lock as [`Database::try_hold_vacuum`] does, for the calling thread to
    /// give up once `asked` is set. Another call that fails to take it sets `asked` and unparks
    /// the calling thread.
    pub(super) fn try_hold_yielding(&self, asked: Arc<AtomicBool>) -> Option<VacuumHold<'_, R>> {
        self.try_hold(Holder::Yields {
            asked,
            thread: self.rt.current(),
        })
    }

    fn try_hold(&self, holder: Holder) -> Option<VacuumHold<'_, R>> {
        let mut held = self.vacuuming.lock(&self.rt);
        match &*held {
            Some(Holder::Keeps) => None,
            Some(Holder::Yields { asked, thread }) => {
                asked.store(true, Ordering::Release);
                self.vacuum_yields.fetch_add(1, Ordering::Relaxed);
                let thread = *thread;
                drop(held);
                self.rt.unpark(thread);
                None
            }
            None => {
                *held = Some(holder);
                self.session.waits.started(VACUUM_HOLDER);
                Some(VacuumHold { db: self })
            }
        }
    }

    /// Returns the vacuum lock for a call.
    ///
    /// Fail while another holds it: with `Blocked` on a call that suspends, whose rerun waits with
    /// no worker; else with what stops the call, looking every few milliseconds; and with
    /// `Cancelled` once rebuilds are stopped.
    pub fn hold_vacuum(&self, waiting: HoldWait<'_>) -> Result<VacuumHold<'_, R>, Error> {
        self.hold_vacuum_until(&waiting.stop(self.rt.now()))
    }

    /// Returns the vacuum lock as [`Database::hold_vacuum`] does, stopping as `stop` says.
    pub(super) fn hold_vacuum_until(&self, stop: &Stop) -> Result<VacuumHold<'_, R>, Error> {
        loop {
            if let Some(hold) = self.try_hold_vacuum() {
                return Ok(hold);
            }
            self.rebuild_stopped()?;
            if let Some(rerun) = stop.suspend {
                return Err(Error::Blocked {
                    on: VACUUM_HOLDER,
                    rerun,
                });
            }
            stop.check(self.rt.now())?;
            self.pause_for_rebuild();
        }
    }

    /// Returns how many pauses calls have made at a rebuild's gate or for the vacuum lock.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn rebuild_waits(&self) -> u64 {
        self.rebuild_waits.load(Ordering::Relaxed)
    }

    /// Returns how many times a call asked a background vacuum to give up the vacuum lock.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn vacuum_yields(&self) -> u64 {
        self.vacuum_yields.load(Ordering::Relaxed)
    }

    /// Returns whether an offline rebuild holds writes to any of `rels`.
    pub(super) fn holds_writes(&self, mut rels: impl Iterator<Item = RelId>) -> bool {
        let held = self.offline.lock(&self.rt);
        !held.is_empty() && rels.any(|r| held.contains(&r))
    }

    /// Stop every rebuild before its swap and refuse statements its gate holds, from now on.
    pub fn stop_rebuilds(&self) {
        self.rebuilds_stopped.store(true, Ordering::Release);
    }

    /// Fail with `Cancelled` once [`Database::stop_rebuilds`] has run.
    pub(super) fn rebuild_stopped(&self) -> Result<(), Error> {
        if self.rebuilds_stopped.load(Ordering::Acquire) {
            Err(Error::Exec(exec::Error::Cancelled))
        } else {
            Ok(())
        }
    }

    /// Count a wait for a rebuild and block the calling thread until `GATE_POLL` has passed.
    pub(super) fn pause_for_rebuild(&self) {
        self.rebuild_waits.fetch_add(1, Ordering::Relaxed);
        self.pause_a_poll();
    }

    /// Block the calling thread until `GATE_POLL` has passed.
    pub(super) fn pause_a_poll(&self) {
        self.nap(&mut self.rt.io(), GATE_POLL);
    }

    /// Block the calling thread until `d` has passed, waiting on `io`.
    pub(super) fn nap(&self, io: &mut R::Io, d: Duration) {
        io.submit(Op::Timer {
            at: self.rt.now() + d,
        });
        while io.poll().is_none() {
            self.rt.park();
        }
    }

    /// Returns how a client names index `id` of `rel`.
    pub fn index_clause(&self, rel: RelId, id: IndexId) -> String {
        let r = &self.rels[&rel];
        let clause = r.indexes.iter().find(|(i, _)| *i == id);
        format!(
            "{} {}",
            r.qualified,
            clause.map_or("", |(_, x)| x.clause.as_str())
        )
    }

    /// Rebuild one index and swap it in, clearing its mark in the same atomic step. Returns
    /// whether every old page was freed.
    ///
    /// Precondition: the caller holds [`Database::holding_off_vacuum`].
    pub fn rebuild(&self, job: &Rebuild<'_>) -> Result<bool, Failure> {
        let mut pages = Pages {
            built: Vec::new(),
            built_past: Vec::new(),
            old: Replaced {
                pages: Vec::new(),
                found: true,
            },
        };
        match self.rebuild_and_swap(job, &mut pages) {
            Ok(()) => {
                (job.phase)("reclaim", 0.9);
                let freed = retire(&self.pager, &pages.old.pages);
                Ok(pages.old.found && freed.is_ok())
            }
            Err(e) => {
                let mut reclaimed = self.free_built(
                    self.bindings.indexes()[&job.index].index.tree().owner(),
                    &pages.built,
                );
                if let Some(past) = self.history_tree(job.index) {
                    reclaimed &= self.free_built(past.owner(), &pages.built_past);
                }
                reclaimed &= pages.old.found && retire(&self.pager, &pages.old.pages).is_ok();
                let cause = match e {
                    Error::Exec(exec::Error::Cancelled) => None,
                    e => Some(self.detail(self.fail(e).error())),
                };
                Err(Failure { cause, reclaimed })
            }
        }
    }

    /// Free the pages of an abandoned build. Returns whether all of them were.
    fn free_built(&self, owner: u32, made: &[Pid]) -> bool {
        let top = made.last().map(|p| BTree::open(*p, owner));
        let mut pages: BTreeSet<Pid> = made.iter().copied().collect();
        let mut complete = true;
        if let Some(top) = top {
            let (reached, all) = top.reachable(&self.pager);
            pages.extend(reached);
            complete = all;
        }
        for p in pages {
            complete &= self.pager.free(p).is_ok();
        }
        complete
    }

    fn rebuild_and_swap(&self, job: &Rebuild<'_>, pages: &mut Pages) -> Result<(), Error> {
        self.admit_reindex(job.rel)?;
        let r = &self.rels[&job.rel];
        let (_, ix) = r
            .indexes
            .iter()
            .find(|(i, _)| *i == job.index)
            .unwrap_or_else(|| panic!("{:?} is an index of {}", job.index, r.name));
        let tree = self.bindings.indexes()[&job.index].index.tree();
        let scan = Scan {
            db: self,
            rel: r,
            id: job.index,
            ix,
        };
        let _offline = (!job.online).then(|| {
            self.session.waits.started(OFFLINE_GATE);
            self.offline.lock(&self.rt).insert(job.rel);
            Offline {
                db: self,
                rel: job.rel,
            }
        });
        (job.phase)("scan", 0.1);
        let mut since = self.pager.appended();
        let snapshot = self.session.begin_reader(false)?;
        let runs = scan.runs(None, Some(&snapshot));
        let runs = self.abort_returning(snapshot, runs)?;
        self.rebuild_stopped()?;
        (job.phase)("build", 0.4);
        let stop = self.build_stop();
        let entries = merge_runs(runs, ix.desc.unique, &stop)?;
        let built = BTree::bulk(&self.pager, tree.owner(), entries, &mut pages.built, &stop)?;
        (job.phase)("catch up", 0.6);
        if job.online {
            for _ in 0..CATCH_UPS {
                since = scan.catch_up(built, since)?;
                self.rebuild_stopped()?;
            }
        }
        (job.phase)("swap", 0.8);
        let _held = self.latches[&job.index].lock(&self.rt);
        scan.catch_up(built, since)?;
        self.rebuild_stopped()?;
        self.admit_reindex(job.rel)?;
        self.mend_history(job, &scan, pages)?;
        let release = Some(Quarantine::Index(tree.owner()));
        self.swap_in(tree, built, release, job, &mut pages.built, &mut pages.old)
    }

    /// Returns whether a build should stop.
    fn build_stop(&self) -> impl Fn() -> bool + '_ {
        || self.rebuild_stopped().is_err()
    }

    /// Returns the history key index tree of `id`, if it has one.
    fn history_tree(&self, id: IndexId) -> Option<BTree> {
        let retained = self.retained.as_deref()?;
        Some(retained.indexes.get(&id)?.tree())
    }

    /// Rebuild the history key index of the job's index from the history heaps if that tree is
    /// damaged, leaving any mark. Nothing writes that tree while vacuum is held off, so a walk
    /// that fails on anything but an I/O error shows damage.
    fn mend_history(
        &self,
        job: &Rebuild<'_>,
        scan: &Scan<'_, R>,
        pages: &mut Pages,
    ) -> Result<(), Error> {
        let (Some(retained), Some(tree)) = (self.retained.as_deref(), self.history_tree(job.index))
        else {
            return Ok(());
        };
        match tree.pages(&self.pager) {
            Ok(_) => return Ok(()),
            Err(btree::Error::Page(e)) if e.damage == Damage::Unreadable => return Err(e.into()),
            Err(_) => {}
        }
        let stop = self.build_stop();
        let entries = merge_runs(scan.past_runs(retained)?, false, &stop)?;
        let built = BTree::bulk(
            &self.pager,
            tree.owner(),
            entries,
            &mut pages.built_past,
            &stop,
        )?;
        self.swap_in(
            tree,
            built,
            None,
            job,
            &mut pages.built_past,
            &mut pages.old,
        )
    }

    /// Replace the contents of `tree` with those of `built`, recording the old pages in `old`.
    /// Empties `made`, the pages of `built`, which from the swap on belong to `tree`.
    fn swap_in(
        &self,
        tree: BTree,
        built: BTree,
        release: Option<Quarantine>,
        job: &Rebuild<'_>,
        made: &mut Vec<Pid>,
        old: &mut Replaced,
    ) -> Result<(), Error> {
        let (reached_pages, reached) = tree.reachable(&self.pager);
        self.pager.retire(reached_pages.iter().copied());
        if let Err(e) = tree.take_over(&self.pager, built, release) {
            self.pager.unretire(&reached_pages);
            return Err(e.into());
        }
        made.clear();
        old.pages.extend(reached_pages);
        if reached {
            return Ok(());
        }
        (job.phase)("sweep", 0.85);
        let (strays, judged) = tree.strays(&self.pager).unwrap_or((Vec::new(), false));
        self.pager.retire(strays.iter().copied());
        append_strays(&mut old.pages, strays);
        old.found &= judged;
        Ok(())
    }
}

impl From<BuildError> for Error {
    fn from(e: BuildError) -> Error {
        match e {
            BuildError::Stopped => Error::Exec(exec::Error::Cancelled),
            BuildError::Page(e) => e.into(),
            BuildError::Duplicate(key) => Broken::DuplicateKey(key).into(),
        }
    }
}

/// Append to `old` the pages of `strays` it does not hold, keeping its order.
fn append_strays(old: &mut Vec<Pid>, strays: Vec<Pid>) {
    let held: BTreeSet<Pid> = old.iter().copied().collect();
    old.extend(strays.into_iter().filter(|p| !held.contains(p)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strays_follow_the_reachable_order() {
        let mut old = vec![Pid(9), Pid(3), Pid(7)];
        append_strays(&mut old, vec![Pid(1), Pid(3), Pid(8)]);
        assert_eq!(vec![Pid(9), Pid(3), Pid(7), Pid(1), Pid(8)], old);
    }
}
