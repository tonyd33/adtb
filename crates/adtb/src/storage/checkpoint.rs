//! The checkpointer: a background thread that writes the dirty pages and takes a fuzzy
//! checkpoint each time the log forced since the last checkpoint reaches the threshold of the
//! pager's options.
//!
//! Lock order: the worker lock is a leaf, taken by a force under any page latch, with nothing
//! taken under it. A cycle takes what a write-out and a checkpoint
//! take: a page's shared latch, then the flush lock and the log lock to make the log durable to
//! its LSN, then the meta lock; then the force lock and the meta lock; then the damage list's
//! exclusive latch, the log lock and the meta lock; then the header lock alone.

use std::sync::{Arc, Weak};

use crate::runtime::{Runtime, ThreadId};

use super::pager::{Pager, Shared};
use super::record::Record;
use super::{Lsn, Pid};

/// The checkpointer's thread, started on the first checkpoint that falls due.
#[derive(Debug)]
pub(crate) struct Worker {
    every: u64,
    thread: Option<ThreadId>,
    stop: bool,
}

impl Worker {
    pub(crate) fn new(every: u64) -> Worker {
        Worker {
            every,
            thread: None,
            stop: false,
        }
    }
}

impl<R: Runtime> Pager<R> {
    /// Write up to `max` changed pages, oldest first by the LSN that dirtied them, and force the
    /// data file. Returns the number written.
    pub fn flush_pages(&self, max: usize) -> usize {
        let oldest: Vec<Pid> = {
            let m = self.meta();
            let mut by_lsn: Vec<(Lsn, Pid)> = m
                .dirty
                .iter()
                .filter(|(p, _)| m.changed.contains(p))
                .map(|(p, l)| (*l, *p))
                .collect();
            by_lsn.sort_unstable();
            by_lsn.into_iter().take(max).map(|(_, p)| p).collect()
        };
        let mut n = 0;
        for pid in oldest {
            let Some(g) = self.mem.lock_shared(pid.0, &self.rt) else {
                continue;
            };
            if self.meta().changed.contains(&pid) {
                self.write_out(pid, &g);
                n += 1;
            }
        }
        self.sync_data();
        n
    }

    /// Force the data file and drop from the dirty page table every page written before the force
    /// with no change since. A page changed since moves its `rec_lsn` up to the record its replay
    /// may start from, as `Meta::after_write` holds it.
    pub(crate) fn sync_data(&self) {
        let _forcing = self.forcing.lock(&self.rt);
        let covered = std::mem::take(&mut self.meta().written);
        self.io(|d| d.sync_all(&[self.data]));
        let mut m = self.meta();
        for pid in covered {
            // A page in `written` was written after the take, and this force may have missed it.
            if m.written.contains(&pid) {
                continue;
            }
            let start = m.after_write.remove(&pid);
            if !m.changed.contains(&pid) {
                m.dirty.remove(&pid);
                m.images.remove(&pid);
            } else if let Some(start) = start
                && let Some(rec_lsn) = m.dirty.get_mut(&pid)
            {
                *rec_lsn = start.max(*rec_lsn);
            }
        }
    }

    /// Log a fuzzy checkpoint of the dirty page table, force it, and point the file header at it.
    pub fn checkpoint(&self) {
        self.log_checkpoint();
    }

    /// Take a checkpoint as [`Pager::checkpoint`] does. Returns its LSN, its redo LSN and the
    /// bitmap groups it captured.
    pub(super) fn log_checkpoint(&self) -> (Lsn, Lsn, u64) {
        let (mut list, (at, redo, groups, folded)) = loop {
            let list = self.list_to_fold();
            let mut log = self.log();
            if list.is_none() && self.to_fold().is_some() {
                continue;
            }
            let (groups, map) = {
                let m = self.meta();
                (m.groups, m.map)
            };
            let lsn = log.next_lsn();
            let dirty: Vec<(Pid, Lsn)> = self.meta().dirty.iter().map(|(p, l)| (*p, *l)).collect();
            let redo = dirty.iter().map(|(_, l)| *l).min().unwrap_or(lsn);
            let folded = list.as_ref().map(|_| self.records_to_fold());
            let at = log.append(&Record::Checkpoint { redo, dirty, map });
            break (list, (at, redo, groups, folded));
        };
        // The fold is logged after the checkpoint record: every record before it is in the fold,
        // and recovery from it reads every record after.
        if let (Some(g), Some(folded)) = (&mut list, &folded) {
            Self::fold_records(g, folded);
        }
        drop(list);
        self.flush_log();
        if folded.is_some() {
            let mut h = self.header.lock(&self.rt);
            h.0.log_damage.clear();
            h.0.log_damage_full = false;
        }
        self.write_header(Some((at, groups)));
        self.log().checkpointed(at, redo);
        (at, redo, groups)
    }

    /// Returns the dirty page table: each page's `rec_lsn`.
    #[doc(hidden)]
    pub fn dirty_pages(&self) -> std::collections::BTreeMap<Pid, Lsn> {
        self.meta().dirty.clone()
    }

    /// Make checkpoints fall due from now on, each once the log forced since the last reaches the
    /// threshold.
    pub(super) fn start_checkpoints(&self) {
        let every = self.checkpointer.lock(&self.rt).every;
        self.log().checkpoint_every(every);
    }

    /// Wake the checkpointer, starting it if it has not run, unless it is stopped.
    pub(super) fn wake_checkpointer(&self) {
        let mut w = self.checkpointer.lock(&self.rt);
        if w.stop {
            return;
        }
        match w.thread {
            Some(t) => self.rt.unpark(t),
            None => {
                let shared = Arc::downgrade(&self.0);
                w.thread = Some(self.rt.spawn_daemon("checkpoint", move || run(&shared)));
            }
        }
    }

    /// Stop the checkpointer, waiting for a running checkpoint to end. No later wake starts it.
    pub(super) fn stop_checkpointer(&self) {
        let thread = {
            let mut w = self.checkpointer.lock(&self.rt);
            w.stop = true;
            w.thread.take()
        };
        if let Some(t) = thread {
            self.rt.unpark(t);
            self.rt.join(t);
        }
    }
}

/// Write every changed page and take a checkpoint whenever one is due, until stopped.
fn run<R: Runtime>(shared: &Weak<Shared<R>>) {
    loop {
        let Some(pager) = shared.upgrade().map(Pager) else {
            return;
        };
        if pager.checkpointer.lock(&pager.rt).stop {
            return;
        }
        if pager.log().checkpoint_due() {
            pager.flush_pages(usize::MAX);
            pager.checkpoint();
            continue;
        }
        let rt = pager.rt.clone();
        drop(pager);
        rt.park();
    }
}
