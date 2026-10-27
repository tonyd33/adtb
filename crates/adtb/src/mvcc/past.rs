use std::ops::Bound;
use std::time::Duration;

use crate::btree::{self, BTree, prefix_end};
use crate::runtime::Runtime;
use crate::storage::{Lsn, PageError, Pager, Position};

use super::{Snapshot, Xid};

const COMMIT: u8 = 0;
const MARKER: u8 = 1;

/// A safe snapshot kept for reads of the past.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marker {
    /// The position of the latest commit the snapshot reads.
    pub at: Position,
    /// The recorded time of the commit at `at`, in nanoseconds since the Unix epoch.
    pub time: u64,
    pub snapshot: Snapshot,
}

/// The record of the retention window: the commit time of each transaction committed within it
/// and the safe snapshots taken within it, on one B-link tree whose root never moves.
///
/// Invariants:
///
/// - A commit's time is never less than the time of any commit before it.
/// - At most one marker is kept per position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Past {
    tree: BTree,
    window: Duration,
}

fn commit_key(x: Xid) -> Vec<u8> {
    let mut k = vec![COMMIT];
    k.extend(x.0.to_be_bytes());
    k
}

/// Returns the prefix of every marker at or after `at` in the tree's order, which is the order
/// of decreasing positions.
fn marker_key(at: Position) -> Vec<u8> {
    let mut k = vec![MARKER];
    k.extend((!at.epoch).to_be_bytes());
    k.extend((!at.lsn.0).to_be_bytes());
    k
}

fn word(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

fn decode_marker(entry: &[u8]) -> Option<Marker> {
    let epoch = !u32::from_be_bytes(entry.get(1..5)?.try_into().ok()?);
    let lsn = !word(entry, 5)?;
    let time = word(entry, 13)?;
    let snapshot = Snapshot::decode(entry.get(21..)?)?;
    Some(Marker {
        at: Position {
            epoch,
            lsn: Lsn(lsn),
        },
        time,
        snapshot,
    })
}

impl Past {
    /// Allocate an empty record whose pages carry `owner`, keeping `window` of history.
    pub fn create<R: Runtime>(
        pager: &Pager<R>,
        owner: u32,
        window: Duration,
    ) -> Result<Past, PageError> {
        Ok(Past {
            tree: BTree::create(pager, owner)?,
            window,
        })
    }

    pub fn open(tree: BTree, window: Duration) -> Past {
        Past { tree, window }
    }

    pub fn tree(&self) -> BTree {
        self.tree
    }

    pub fn window(&self) -> Duration {
        self.window
    }

    /// Record that `x` commits at `time`.
    pub fn record_commit<R: Runtime>(
        &self,
        pager: &Pager<R>,
        x: Xid,
        time: u64,
    ) -> Result<(), btree::Error> {
        let mut e = commit_key(x);
        e.extend(time.to_be_bytes());
        self.tree.insert(pager, &e)?;
        Ok(())
    }

    /// Returns the recorded commit time of `x`, or `None` when none is kept.
    pub fn committed_at<R: Runtime>(
        &self,
        pager: &Pager<R>,
        x: Xid,
    ) -> Result<Option<u64>, btree::Error> {
        let found = self.tree.prefix(&commit_key(x)).next(pager)?;
        Ok(found.and_then(|e| word(&e, 9)))
    }

    /// Keep `marker`, unless one at its position is kept already or its snapshot is too long
    /// for an entry.
    pub fn record_marker<R: Runtime>(
        &self,
        pager: &Pager<R>,
        marker: &Marker,
    ) -> Result<(), btree::Error> {
        let prefix = marker_key(marker.at);
        if self.tree.prefix(&prefix).next(pager)?.is_some() {
            return Ok(());
        }
        let mut e = prefix;
        e.extend(marker.time.to_be_bytes());
        e.extend(marker.snapshot.encode());
        if e.len() > BTree::max_entry_len(pager.page_size()) {
            return Ok(());
        }
        self.tree.insert(pager, &e)?;
        Ok(())
    }

    /// Returns the latest marker at or before `at`.
    pub fn marker_at<R: Runtime>(
        &self,
        pager: &Pager<R>,
        at: Position,
    ) -> Result<Option<Marker>, btree::Error> {
        let from = marker_key(at);
        let end = prefix_end(&[MARKER]);
        let mut c = self
            .tree
            .range(Bound::Included(&from), end.as_ref().map(Vec::as_slice));
        Ok(c.next(pager)?.as_deref().and_then(decode_marker))
    }

    /// Returns the latest marker whose time is at or before `time`.
    pub fn marker_by_time<R: Runtime>(
        &self,
        pager: &Pager<R>,
        time: u64,
    ) -> Result<Option<Marker>, btree::Error> {
        let mut c = self.tree.prefix(&[MARKER]);
        while let Some(e) = c.next(pager)? {
            if let Some(m) = decode_marker(&e).filter(|m| m.time <= time) {
                return Ok(Some(m));
            }
        }
        Ok(None)
    }

    /// Returns every marker kept, the latest first.
    pub fn markers<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Marker>, btree::Error> {
        let entries = self.tree.prefix(&[MARKER]).collect(pager)?;
        Ok(entries.iter().filter_map(|e| decode_marker(e)).collect())
    }

    /// Forget every commit time and every marker before `cutoff`. Returns the number of entries
    /// dropped.
    pub fn expire<R: Runtime>(&self, pager: &Pager<R>, cutoff: u64) -> Result<usize, btree::Error> {
        self.tree.retain(pager, |e| {
            let time = match e.first() {
                Some(&COMMIT) => word(e, 9),
                _ => word(e, 13),
            };
            time.is_none_or(|t| t >= cutoff)
        })
    }
}
