//! Read and keep a relation's rows as of a past time.

use crate::btree::{self, Index};
use crate::heap::{self, Flags, Header, Heap};
use crate::key::{CtorId, Entry, Value};
use crate::mvcc::{Horizon, Transactions};
use crate::runtime::Runtime;
use crate::storage::{PageError, Pager};

/// Why versions could not be kept in history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Page(PageError),
    Index(btree::Error),
    Heap(heap::Error),
}

impl From<PageError> for Error {
    fn from(e: PageError) -> Error {
        Error::Page(e)
    }
}

impl From<btree::Error> for Error {
    fn from(e: btree::Error) -> Error {
        Error::Index(e)
    }
}

impl From<heap::Error> for Error {
    fn from(e: heap::Error) -> Error {
        Error::Heap(e)
    }
}

/// Returns the key of a row of a constructor.
pub type Key<'a> = dyn Fn(CtorId, &[Value]) -> Vec<Value> + Sync + 'a;

/// The history heaps of some constructors of one sum and the key index over them, whose
/// entries are `(key, ctor, tid)`.
///
/// Invariant: every version of a history heap has a committed inserter and a committed deleter.
pub struct History<'a> {
    pub heaps: Vec<(CtorId, &'a Heap)>,
    pub index: &'a Index,
    pub key: &'a Key<'a>,
}

impl std::fmt::Debug for History<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("History")
            .field("heaps", &self.heaps)
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}

impl History<'_> {
    pub(crate) fn heap(&self, ctor: CtorId) -> &Heap {
        self.heaps
            .iter()
            .find(|(c, _)| *c == ctor)
            .map(|(_, h)| *h)
            .unwrap_or_else(|| panic!("constructor {ctor:?} has a history heap"))
    }
}

/// Copy every version on page `pid` of `heap`, constructor `ctor`'s, that the window bounded by
/// `horizon` keeps to `ctor`'s history heap in `into`, with its entry in the history key index.
/// A version flagged kept whose copy is left is not copied again: the copy gets the index entry
/// it lacks. Returns the number of versions copied.
pub(crate) fn keep<R: Runtime>(
    pager: &Pager<R>,
    txns: &Transactions<R>,
    heap: &Heap,
    ctor: CtorId,
    into: &History,
    horizon: &Horizon,
    pid: crate::storage::Pid,
) -> Result<usize, Error> {
    let past = into.heap(ctor);
    let mut judge = txns.judge(pager, horizon.past());
    let mut found = Vec::new();
    heap.scan_pages(pager, &[pid], |v| found.push(v))?;
    let mut copied = 0;
    for v in found {
        let kept = txns.kept_by_window(pager, &v.header.writers, horizon);
        if !kept.map_err(|e| v.refuse(pager, e))? {
            continue;
        }
        let key = (into.key)(ctor, &v.row);
        let earlier = if v.flags.contains(Flags::KEPT) {
            held(pager, into, past, ctor, &key, &v.header)?
        } else {
            Held::Not
        };
        let tid = match earlier {
            Held::Indexed => continue,
            Held::Unindexed(tid) => tid,
            Held::Not => match past.copy_version_pruning(pager, heap, &v, &mut judge)? {
                Some(tid) => {
                    copied += 1;
                    tid
                }
                None => continue,
            },
        };
        into.index.insert(pager, &Entry { key, ctor, tid })?;
    }
    Ok(copied)
}

/// Where `past` holds the copy of a version flagged kept.
enum Held {
    /// An entry of the history key index names the copy.
    Indexed,
    /// The copy at this TID has no entry.
    Unindexed(crate::key::Tid),
    Not,
}

/// Returns where `past` holds the copy of a version of `ctor` keyed `key` written as `header`
/// says, looking through the history key index and then through every version of `past`.
fn held<R: Runtime>(
    pager: &Pager<R>,
    into: &History,
    past: &Heap,
    ctor: CtorId,
    key: &[Value],
    header: &Header,
) -> Result<Held, Error> {
    let writers = header.writers;
    for e in into.index.probe(pager, key)? {
        if e.ctor != ctor {
            continue;
        }
        if let Some(v) = past.read(pager, e.tid)?
            && v.header.writers == writers
        {
            return Ok(Held::Indexed);
        }
    }
    let mut found = Held::Not;
    past.scan(pager, |v| {
        if v.header.writers == writers && (into.key)(ctor, &v.row) == key {
            found = Held::Unindexed(v.tid);
        }
    })?;
    Ok(found)
}
