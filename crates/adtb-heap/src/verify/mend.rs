//! The mend pass: what a crash may leave on a sound file, set right.

use std::collections::BTreeMap;

use crate::btree::BTree;
use crate::heap::{Heap, LargeStore, Refused};
use crate::runtime::Runtime;
use crate::storage::{Lsn, Pager, Pid};

use super::{Problem, verify_store};

/// What one mend pass did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mended {
    /// The splits it finished.
    splits: usize,
    /// The heads it lowered.
    heads: usize,
    /// The heads it left overcounted, since a write changed their page during the pass.
    skipped: usize,
    /// The heaps and stores whose directories it settled.
    directories: usize,
    /// What kept it from mending a structure.
    stopped: Vec<String>,
}

impl Mended {
    /// Returns whether the pass left nothing a crash left that it mends.
    pub fn finished(&self) -> bool {
        self.skipped == 0 && self.stopped.is_empty()
    }

    /// Returns the splits it finished.
    pub fn splits(&self) -> usize {
        self.splits
    }

    /// Returns the heads it lowered.
    pub fn heads(&self) -> usize {
        self.heads
    }

    /// Returns the heaps and stores whose directories it settled.
    pub fn directories(&self) -> usize {
        self.directories
    }

    /// Record that `why` kept the pass from mending a structure.
    pub fn stop(&mut self, why: String) {
        self.stopped.push(why);
    }
}

/// Finish every split of `tree` whose page lacks its downlink, holding what `hold` returns
/// around each finish. Returns false once `go` fails, checked before each page. A tree whose
/// walk fails is left as it is, the failure added to `mended`.
pub fn finish_splits<R: Runtime, H>(
    pager: &Pager<R>,
    tree: BTree,
    hold: impl Fn() -> H,
    go: impl Fn() -> bool,
    mended: &mut Mended,
) -> bool {
    match tree.repair_with(pager, hold, go) {
        Ok(Some(n)) => mended.splits += n,
        Ok(None) => return false,
        Err(e) => mended.stop(format!(
            "tree {} at page {}: {e}",
            tree.owner(),
            tree.root().0
        )),
    }
    true
}

/// Lower every head of `store` counting more versions of `heaps` than point at it, and settle
/// every directory of `store` and `heaps` with an entry promising less than its child holds,
/// from one complete check of them. An incomplete check mends nothing and adds its first finding
/// to `mended`.
///
/// Preconditions:
/// - `heaps` are every heap whose versions point into `store`.
/// - No vacuum runs, and every other change to a version or a count is made by a transaction.
/// - Every transaction running when the log's next position was `since` has ended.
/// - `settled` returns once every transaction running when it is called has ended, or returns
///   false when the pass is stopped.
///
/// Postcondition: a head is lowered only when no update of its page was logged at or after
/// `since`, and never below the versions that point at it.
pub fn mend_store<R: Runtime>(
    pager: &Pager<R>,
    store: LargeStore,
    heaps: &[&Heap],
    since: Lsn,
    settled: impl FnOnce() -> bool,
    mended: &mut Mended,
) {
    let report = verify_store(pager, store, heaps);
    if !report.complete {
        let first = report
            .findings
            .iter()
            .find(|f| matches!(f.problem, Problem::Damaged(_) | Problem::Structure(_)));
        mended.stop(first.map_or_else(
            || "a structure could not be read whole".to_owned(),
            ToString::to_string,
        ));
        return;
    }
    if !settled() {
        mended.stop("the pass was stopped".into());
        return;
    }
    let mut unsettled = Vec::new();
    let mut overcounted: BTreeMap<Pid, Vec<(u16, u32)>> = BTreeMap::new();
    for f in &report.findings {
        match &f.problem {
            Problem::Overcounted { slot, versions, .. } => {
                overcounted
                    .entry(f.pid)
                    .or_default()
                    .push((*slot, *versions));
            }
            Problem::Unsettled(_) => match pager.read(f.pid) {
                Ok(page) => unsettled.push(page.owner()),
                Err(e) => mended.stop(e.to_string()),
            },
            _ => {}
        }
    }
    for (pid, heads) in overcounted {
        match store.lower_unchanged(pager, pid, &heads, since) {
            Ok(Some(n)) => mended.heads += n,
            Ok(None) => mended.skipped += heads.len(),
            Err(Refused::Page(e)) => mended.stop(e.to_string()),
            Err(Refused::Gone) => mended.stop(format!("page {} was freed during the pass", pid.0)),
        }
    }
    unsettled.sort_unstable();
    unsettled.dedup();
    for owner in unsettled {
        let settled = match heaps.iter().find(|h| h.id().0 == owner) {
            Some(h) => h.settle_directory(pager),
            None if store.id() == owner => store.settle_directory(pager),
            None => {
                mended.stop(format!("no heap or store of the check has id {owner}"));
                continue;
            }
        };
        match settled {
            Ok(()) => mended.directories += 1,
            Err(e) => mended.stop(e.to_string()),
        }
    }
}
