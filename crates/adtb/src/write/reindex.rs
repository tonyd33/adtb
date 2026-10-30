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
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::MutexGuard;
use std::sync::atomic::Ordering;
use std::time::Duration;

use adtb_lang::types::RelId;

use crate::btree::build::{RunEntry, merge_runs, retire};
use crate::btree::{self, BTree};
use crate::heap::Heap;
use crate::key::{CtorId, Entry};
use crate::mvcc::Xid;
use crate::plan::IndexId;
use crate::runtime::{Io, Op, Runtime};
use crate::storage::{self, Damage, Lsn, Pid, Quarantine};

use super::past::Retained;
use super::schema::{Indexed, Rel};
use super::{Database, Error, Txn};

/// One index to rebuild, how, and who follows it.
#[expect(
    missing_debug_implementations,
    reason = "holds a closure; removing it needs the phase callback to be a Debug type"
)]
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

/// Returns the text of a panic's payload, or `None` for an unwinding that is not a failure of the
/// job, such as a runtime stopping its threads.
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

/// How often a statement that cannot suspend looks at the gate and at what stops it.
pub(super) const GATE_POLL: Duration = Duration::from_millis(5);

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
        self.db.waits.ended(OFFLINE_GATE);
    }
}

/// How the rows of one index's constructors give its entries.
struct Scan<'d, R: Runtime> {
    db: &'d Database<R>,
    rel: &'d Rel,
    id: IndexId,
    ix: &'d Indexed,
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
        let stored = &db.indexes[&self.id];
        let schema = stored.index.schema();
        let types: Vec<_> = schema.components().iter().map(|(t, _)| t.clone()).collect();
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
                let counted = match tx {
                    Some(tx) => {
                        db.txns
                            .sees(&db.pager, &tx.snapshot, None, v.header.xmin, v.header.xmax)?
                    }
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

    fn runs(&self, since: Option<Lsn>, tx: Option<&Txn>) -> Result<Vec<Vec<RunEntry>>, Error> {
        self.db.indexes[&self.id]
            .ctors
            .iter()
            .map(|(&ctor, &c)| {
                let heap = &self.db.heaps[&self.rel.ctors[c as usize].heap].heap;
                self.run(heap, (ctor, c), since, tx)
            })
            .collect()
    }

    /// Returns a sorted run of the entries of every version in each history heap of `retained`.
    fn past_runs(&self, retained: &Retained) -> Result<Vec<Vec<RunEntry>>, Error> {
        self.db.indexes[&self.id]
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
        let r = self
            .rels
            .get(&rel)
            .ok_or_else(|| Error::Internal(format!("reindex of {rel:?}, which has no heaps")))?;
        let Some(positions) = positions else {
            return Ok(r.indexes.iter().map(|(id, _)| *id).collect());
        };
        let mut ids = BTreeSet::new();
        for &i in positions {
            let id = self
                .schema
                .ids
                .indexes
                .get(&(rel, i))
                .ok_or_else(|| Error::Internal(format!("index {i} of {} is stored", r.name)))?;
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
            let object = Quarantine::Heap(self.heaps[&c.heap].heap.id().0);
            if marks.contains(&object) {
                return Err(Error::Quarantined(self.quarantined_name(object)));
            }
        }
        Ok(())
    }

    /// Returns the guard that holds off vacuum, the one other writer that removes entries and
    /// frees slots, and one rebuild at a time.
    pub fn holding_off_vacuum(&self) -> MutexGuard<'_, ()> {
        self.vacuuming.lock(&self.rt)
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
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Block the calling thread until `GATE_POLL` has passed.
    pub(super) fn pause_at_gate(&self) {
        let mut io = self.rt.io();
        io.submit(Op::Timer {
            at: self.rt.now() + GATE_POLL,
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
        let ran = catch_unwind(AssertUnwindSafe(|| self.rebuild_and_swap(job, &mut pages)));
        let ran = ran.unwrap_or_else(|payload| match panic_text(&*payload) {
            Some(text) => Err(Error::Internal(text)),
            None => resume_unwind(payload),
        });
        match ran {
            Ok(()) => {
                (job.phase)("reclaim", 0.9);
                let freed = retire(&self.pager, &pages.old.pages);
                Ok(pages.old.found && freed.is_ok())
            }
            Err(e) => {
                let mut reclaimed =
                    self.free_built(self.indexes[&job.index].index.tree().owner(), &pages.built);
                if let Some(past) = self.history_tree(job.index) {
                    reclaimed &= self.free_built(past.owner(), &pages.built_past);
                }
                reclaimed &= pages.old.found && retire(&self.pager, &pages.old.pages).is_ok();
                let cause = match e {
                    Error::Cancelled => None,
                    e => Some(format!("{:?}", self.quarantining(e))),
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
            .ok_or_else(|| Error::Internal(format!("{:?} is an index of {}", job.index, r.name)))?;
        let tree = self.indexes[&job.index].index.tree();
        let scan = Scan {
            db: self,
            rel: r,
            id: job.index,
            ix,
        };
        let _offline = (!job.online).then(|| {
            self.waits.started(OFFLINE_GATE);
            self.offline.lock(&self.rt).insert(job.rel);
            Offline {
                db: self,
                rel: job.rel,
            }
        });
        (job.phase)("scan", 0.1);
        let mut since = self.pager.appended();
        let snapshot = self.begin_reader(false)?;
        let runs = scan.runs(None, Some(&snapshot));
        self.abort(snapshot)?;
        let runs = runs?;
        self.rebuild_stopped()?;
        (job.phase)("build", 0.4);
        let entries = merge_runs(runs, ix.desc.unique).map_err(|key| {
            Error::Internal(format!("a duplicate of key {key:?} in a unique index"))
        })?;
        let built = BTree::bulk(&self.pager, tree.owner(), entries, &mut pages.built)?;
        self.rebuild_stopped()?;
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
        let entries = merge_runs(scan.past_runs(retained)?, false).map_err(|key| {
            Error::Internal(format!(
                "a duplicate of {key:?} in a merge that allows them"
            ))
        })?;
        let built = BTree::bulk(&self.pager, tree.owner(), entries, &mut pages.built_past)?;
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
        tree.take_over(&self.pager, built, release)?;
        made.clear();
        old.pages.extend(reached_pages);
        if reached {
            return Ok(());
        }
        let found = std::mem::replace(&mut old.found, false);
        (job.phase)("sweep", 0.85);
        let (strays, judged) = tree.strays(&self.pager).unwrap_or((Vec::new(), false));
        append_strays(&mut old.pages, strays);
        old.found = found && judged;
        Ok(())
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
