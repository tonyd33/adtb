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

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use adtb_lang::types::RelId;
use adtb_sys::lock::HeldRank;

use crate::btree::build::{BuildError, IndexLatched, LatchedTree, RunEntry, merge_runs, retire};
use crate::btree::{self, BTree};
use crate::exec;
use crate::heap::{Heap, Refused};
use crate::key::{CtorId, Entry};
use crate::mvcc::{Gate, Gated};
use crate::plan::IndexId;
use crate::ranks;
use crate::runtime::{Instant, Io, Op, Runtime, ThreadId};
use crate::steps::{Reindex, Step};
use crate::storage::{self, Damage, Lsn, Pid, Quarantine};
use crate::vacuum::{Busy, Holder, Vacuuming};

use super::past::Retained;
use super::schema::{Indexed, Rel};
use super::{Bounds, Broken, Database, Error, Rerun, Stop, Txn};

/// One index to rebuild, how, and who follows it.
pub struct Rebuild<'a> {
    pub rel: RelId,
    pub index: IndexId,
    pub online: bool,
    /// Told each phase as it starts.
    pub phase: &'a dyn Fn(Reindex),
}

/// Why a rebuild kept the old index.
#[derive(Debug)]
pub struct Failure {
    /// What went wrong, `None` for a rebuild stopped by its caller.
    pub cause: Option<String>,
    /// Whether every page it allocated is free again.
    pub reclaimed: bool,
}

/// How many catch-ups run while writes continue before the one that holds them.
const CATCH_UPS: usize = 2;

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

/// The vacuum lock. Holds off vacuum, the one other writer that removes entries and frees slots,
/// every other rebuild, and every other holder.
pub struct VacuumHold<'d, R: Runtime> {
    db: &'d Database<R>,
    vacuuming: Vacuuming<'d, R>,
    _rank: HeldRank,
}

impl<'d, R: Runtime> VacuumHold<'d, R> {
    /// Returns the database whose lock this is.
    pub(super) fn db(&self) -> &'d Database<R> {
        self.db
    }

    /// Returns the lock as the vacuum, the sweep of a large-value store and a copy to a history
    /// heap take it.
    pub(super) fn vacuuming(&self) -> &Vacuuming<'d, R> {
        &self.vacuuming
    }

    /// Rebuild one index under this lock and swap it in, clearing its mark in the same atomic
    /// step. Returns whether every old page was freed.
    pub fn rebuild(&self, job: &Rebuild<'_>) -> Result<bool, Failure> {
        self.db.rebuild(job)
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
    /// Ended once the relation has left the offline set.
    _gated: Gated<'d, R>,
}

/// The wait of the calling thread on a hold of another, recorded until dropped.
struct Waiting<'d, R: Runtime> {
    db: &'d Database<R>,
}

impl<R: Runtime> Drop for Waiting<'_, R> {
    fn drop(&mut self) {
        let me = self.db.rt.current();
        self.db.waiting.lock(&self.db.rt).remove(&me);
    }
}

impl<R: Runtime> Drop for Offline<'_, R> {
    fn drop(&mut self) {
        self.db.offline.lock(&self.db.rt).remove(&self.rel);
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
            let on_page = 'read: loop {
                let mut versions = Vec::new();
                heap.scan_rooted(&db.pager, pid, |v, roots| versions.push((v, roots)))?;
                let mut on_page = Vec::new();
                for (v, roots) in versions {
                    let Some(key) = self.ix.keys.key(c, &v.row, &types)? else {
                        continue;
                    };
                    let counted = match reader.as_ref().map(|r| r.sees(&v.header.writers)) {
                        Some(Ok(seen)) => seen.visible(),
                        Some(Err(e)) => match v.refuse(&db.pager, e) {
                            Refused::Page(e) => return Err(e.into()),
                            Refused::Gone => continue 'read,
                        },
                        None => false,
                    };
                    on_page.push((key, roots, counted));
                }
                break on_page;
            };
            for (key, roots, counted) in on_page {
                let key_len = schema.encode(&key)?.len();
                for root in roots {
                    let entry = schema.encode_entry(&Entry {
                        key: key.clone(),
                        ctor,
                        tid: root,
                    })?;
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
            let id = (self.schema.ids().index_id(rel, i))
                .unwrap_or_else(|| panic!("index {i} of {} is stored", r.name));
            ids.insert(IndexId(id.raw()));
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
        ranks::VACUUM.check();
        let _waiting = self.waiting();
        loop {
            match self.try_hold_vacuum() {
                Ok(hold) => return hold,
                Err(holder) => self.pause_for_rebuild(holder),
            }
        }
    }

    /// Returns the vacuum lock unless another holds it, else the holder's thread.
    /// [`Gate::VacuumHolder`] runs exactly while one does, so a call that watches it and finds it
    /// ended finds the lock free.
    fn try_hold_vacuum(&self) -> Result<VacuumHold<'_, R>, ThreadId> {
        self.try_hold(Holder::Keeps {
            thread: self.rt.current(),
        })
    }

    /// Returns the vacuum lock as [`Database::try_hold_vacuum`] does, for the calling thread to
    /// give up once `asked` is set. Another call that fails to take it sets `asked` and unparks
    /// the calling thread.
    pub(super) fn try_hold_yielding(&self, asked: Arc<AtomicBool>) -> Option<VacuumHold<'_, R>> {
        self.try_hold(Holder::Yields {
            asked,
            thread: self.rt.current(),
        })
        .ok()
    }

    fn try_hold(&self, holder: Holder) -> Result<VacuumHold<'_, R>, ThreadId> {
        match self.session.txns.take_vacuum(holder) {
            Ok(vacuuming) => Ok(VacuumHold {
                db: self,
                vacuuming,
                _rank: ranks::VACUUM.hold(),
            }),
            Err(Busy::Kept(thread)) => Err(thread),
            Err(Busy::Asked(thread)) => {
                self.vacuum_yields.fetch_add(1, Ordering::Relaxed);
                self.rt.unpark(thread);
                Err(thread)
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
        ranks::VACUUM.check();
        let _waiting = self.waiting();
        loop {
            let holder = match self.try_hold_vacuum() {
                Ok(hold) => return Ok(hold),
                Err(holder) => holder,
            };
            self.rebuild_stopped()?;
            if let Some(rerun) = stop.suspend {
                return Err(Error::Blocked {
                    on: Gate::VacuumHolder.xid(),
                    heap: None,
                    rerun,
                });
            }
            stop.check(self.rt.now())?;
            self.pause_for_rebuild(holder);
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

    /// Returns the thread of an offline rebuild that holds writes to any of `rels`.
    pub(super) fn writes_held_by(&self, mut rels: impl Iterator<Item = RelId>) -> Option<ThreadId> {
        let held = self.offline.lock(&self.rt);
        rels.find_map(|r| held.get(&r).copied())
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

    /// Returns the record of the calling thread's wait on a hold, cleared when dropped.
    pub(super) fn waiting(&self) -> impl Drop + '_ {
        Waiting { db: self }
    }

    /// Count a wait for a rebuild, record the calling thread as waiting on a hold of `holder`,
    /// and block it until `GATE_POLL` has passed.
    ///
    /// Precondition: the caller holds what [`Database::waiting`] returned for the whole wait.
    pub(super) fn pause_for_rebuild(&self, holder: ThreadId) {
        self.rebuild_waits.fetch_add(1, Ordering::Relaxed);
        let me = self.rt.current();
        self.waiting.lock(&self.rt).insert(me, holder);
        self.pause_a_poll();
    }

    /// Returns whether a thread waits on a hold of `holder` or on a transaction begun on it.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn waits_on(&self, holder: ThreadId) -> bool {
        let gated = self.waiting.lock(&self.rt).values().any(|h| *h == holder);
        gated || self.session.waits().watched_on(holder)
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

    fn began(&self, job: &Rebuild<'_>, phase: Reindex) {
        (job.phase)(phase);
        self.pager.steps().at(Step::Reindex(phase));
    }

    fn rebuild(&self, job: &Rebuild<'_>) -> Result<bool, Failure> {
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
                self.began(job, Reindex::Reclaim);
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
            let gated = self.session.waits().gate(Gate::Offline);
            let me = self.rt.current();
            self.offline.lock(&self.rt).insert(job.rel, me);
            Offline {
                db: self,
                rel: job.rel,
                _gated: gated,
            }
        });
        self.began(job, Reindex::Scan);
        let mut since = self.pager.appended();
        let snapshot = self.session.begin_reader(false)?;
        let runs = scan.runs(None, Some(&snapshot));
        let runs = self.abort_returning(snapshot, runs)?;
        self.rebuild_stopped()?;
        self.began(job, Reindex::Build);
        let stop = self.build_stop();
        let entries = merge_runs(runs, ix.desc.unique, &stop)?;
        let built = BTree::bulk(&self.pager, tree.owner(), entries, &mut pages.built, &stop)?;
        self.began(job, Reindex::CatchUp);
        if job.online {
            for _ in 0..CATCH_UPS {
                since = scan.catch_up(built, since)?;
                self.rebuild_stopped()?;
            }
        }
        self.began(job, Reindex::Swap);
        let held = self.latches[&job.index].lock(&self.rt);
        scan.catch_up(built, since)?;
        self.rebuild_stopped()?;
        self.admit_reindex(job.rel)?;
        self.mend_history(&held, job, &scan, pages)?;
        let release = Some(Quarantine::Index(tree.owner()));
        let swap = (held.index(), built);
        self.swap_in(swap, release, job, &mut pages.built, &mut pages.old)
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
        held: &IndexLatched<'_>,
        job: &Rebuild<'_>,
        scan: &Scan<'_, R>,
        pages: &mut Pages,
    ) -> Result<(), Error> {
        let (Some(retained), Some(latched)) = (self.retained.as_deref(), held.history()) else {
            return Ok(());
        };
        let tree = latched.tree();
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
            (latched, built),
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
        (latched, built): (LatchedTree<'_>, BTree),
        release: Option<Quarantine>,
        job: &Rebuild<'_>,
        made: &mut Vec<Pid>,
        old: &mut Replaced,
    ) -> Result<(), Error> {
        let tree = latched.tree();
        let (reached_pages, reached) = tree.reachable(&self.pager);
        self.pager.retire(reached_pages.iter().copied());
        if let Err(e) = latched.take_over(&self.pager, built, release) {
            self.pager.unretire(&reached_pages);
            return Err(e.into());
        }
        made.clear();
        old.pages.extend(reached_pages);
        if reached {
            return Ok(());
        }
        self.began(job, Reindex::Sweep);
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
            BuildError::Unsorted(entry) => Broken::UnsortedBuild(entry).into(),
        }
    }
}

/// Append to `old` the pages of `strays` it does not hold, keeping its order.
fn append_strays(old: &mut Vec<Pid>, strays: Vec<Pid>) {
    let held: BTreeSet<Pid> = old.iter().copied().collect();
    old.extend(strays.into_iter().filter(|p| !held.contains(p)));
}
