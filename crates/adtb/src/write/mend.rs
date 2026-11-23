//! The mend pass a file gets after an open that recovered a log ending at no clean close.

use std::collections::BTreeSet;

use crate::btree::BTree;
use crate::mvcc::Xid;
use crate::runtime::Runtime;
use crate::verify::{Mended, finish_splits, mend_store, stores};

use super::Database;

const STOPPED: &str = "the pass was stopped";

impl<R: Runtime> Database<R> {
    /// Finish every split a crash left incomplete, each under the latch of its index as an
    /// insert finishes one; then, holding vacuum off, lower every head counting more versions
    /// than point at it and settle every directory promising less than its child holds, one
    /// large-value store and its heaps at a time. Returns what it did. Checks `stopped` and
    /// whether rebuilds are stopped before each page of a tree's walk and each store, and calls
    /// `pause` to wait for a transaction or the vacuum lock.
    pub fn mend(&self, stopped: &dyn Fn() -> bool, pause: &mut dyn FnMut()) -> Mended {
        let go = || !stopped() && self.rebuild_stopped().is_ok();
        let mut mended = Mended::default();
        let structures = self.structures();
        let inventory = structures.inventory(None);
        let mut latched = BTreeSet::new();
        for latch in self.latches.values() {
            let trees: Vec<BTree> = {
                let held = latch.lock(&self.rt);
                std::iter::once(held.index())
                    .chain(held.history())
                    .map(|t| t.tree())
                    .collect()
            };
            for tree in trees {
                latched.insert(tree.root());
                let hold = || latch.lock(&self.rt);
                if !finish_splits(&self.pager, tree, hold, go, &mut mended) {
                    mended.stop(STOPPED.into());
                    return mended;
                }
            }
        }
        for &tree in inventory
            .trees
            .iter()
            .filter(|t| !latched.contains(&t.root()))
        {
            if !finish_splits(&self.pager, tree, || (), go, &mut mended) {
                mended.stop(STOPPED.into());
                return mended;
            }
        }
        let since = self.pager.appended();
        if !self.ended_below(self.txns().next(), go, pause) {
            mended.stop(STOPPED.into());
            return mended;
        }
        let _no_vacuum = loop {
            if !go() {
                mended.stop(STOPPED.into());
                return mended;
            }
            match self.try_hold_vacuum() {
                Ok(hold) => break hold,
                Err(_) => pause(),
            }
        };
        for (store, heaps) in stores(&inventory) {
            if !go() {
                mended.stop(STOPPED.into());
                return mended;
            }
            let settled = || self.ended_below(self.txns().next(), go, pause);
            mend_store(&self.pager, store, &heaps, since, settled, &mut mended);
        }
        mended
    }

    /// Wait until every transaction below `next` has ended, calling `pause` between looks.
    /// Returns false, waiting no longer, once `go` fails.
    fn ended_below(&self, next: Xid, go: impl Fn() -> bool, pause: &mut dyn FnMut()) -> bool {
        loop {
            if !go() {
                return false;
            }
            if self.txns().ended_below() >= next {
                return true;
            }
            pause();
        }
    }
}
