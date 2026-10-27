use std::collections::BTreeMap;

use crate::btree::Index;
use crate::heap::{Fate, Flags, Header, Heap, HeapNo, Version, walk};
use crate::key::{CtorId, Entry, Value};
use crate::mvcc::{Marker, Status, Transactions, Xid};
use crate::runtime::Runtime;
use crate::storage::{PageError, Pager};
use crate::vacuum::{Error, Horizon};

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
    fn heap(&self, ctor: CtorId) -> &Heap {
        self.heaps
            .iter()
            .find(|(c, _)| *c == ctor)
            .map(|(_, h)| *h)
            .unwrap_or_else(|| panic!("constructor {ctor:?} has a history heap"))
    }
}

/// Returns whether the version whose header is `h` is one the retention window bounded by
/// `horizon` keeps once it is dead to every snapshot: its inserter committed, and another
/// transaction deleted it and committed below the watermark within the window.
pub(crate) fn movable<R: Runtime>(
    txns: &Transactions<R>,
    pager: &Pager<R>,
    horizon: &Horizon,
    h: &Header,
) -> Result<bool, PageError> {
    let (xmin, xmax) = (Xid(h.xmin), Xid(h.xmax));
    if xmax == Xid::NONE || xmin == xmax || xmax >= horizon.watermark {
        return Ok(false);
    }
    if txns.status(pager, xmin)? != Status::Committed
        || txns.status(pager, xmax)? != Status::Committed
    {
        return Ok(false);
    }
    txns.retains(pager, xmax, horizon.cutoff)
}

/// Returns the fate of a version of a history heap whose header is `h`: kept while the
/// retention window bounded by `horizon` keeps it.
pub(crate) fn judge_past<R: Runtime>(
    txns: &Transactions<R>,
    pager: &Pager<R>,
    horizon: &Horizon,
    h: &Header,
) -> Result<Fate, PageError> {
    Ok(if txns.retains(pager, Xid(h.xmax), horizon.cutoff)? {
        Fate::Keep
    } else {
        Fate::Remove
    })
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
    let mut found = Vec::new();
    heap.scan_pages(pager, &[pid], |v| found.push(v))?;
    let mut copied = 0;
    for v in found {
        if !movable(txns, pager, horizon, &v.header)? {
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
            Held::Not => match past.copy_version(pager, heap, &v)? {
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
    let writers = (header.xmin, header.xmax);
    for e in into.index.probe(pager, key)? {
        if e.ctor != ctor {
            continue;
        }
        if let Some(v) = past.read(pager, e.tid)?
            && (v.header.xmin, v.header.xmax) == writers
        {
            return Ok(Held::Indexed);
        }
    }
    let mut found = Held::Not;
    past.scan(pager, |v| {
        if (v.header.xmin, v.header.xmax) == writers && (into.key)(ctor, &v.row) == key {
            found = Held::Unindexed(v.tid);
        }
    })?;
    Ok(found)
}

fn sees<R: Runtime>(
    txns: &Transactions<R>,
    pager: &Pager<R>,
    marker: &Marker,
    h: &Header,
) -> Result<bool, PageError> {
    marker
        .snapshot
        .sees(None, h.xmin, h.xmax, |x| txns.status(pager, x))
}

/// Returns the rows of a constructor as the safe snapshot of `marker` reads them, from `heaps`,
/// its current heap, while it has one, and its history heap, each row once.
///
/// Precondition: `marker` is held by [`Transactions::pin`].
pub fn rows<R: Runtime>(
    pager: &Pager<R>,
    txns: &Transactions<R>,
    marker: &Marker,
    heaps: &[&Heap],
) -> Result<Vec<Vec<Value>>, PageError> {
    let mut seen: BTreeMap<(u64, u64), Vec<Vec<Value>>> = BTreeMap::new();
    let mut versions: Vec<Version> = Vec::new();
    for heap in heaps {
        heap.scan(pager, |v| versions.push(v))?;
    }
    let mut out = Vec::new();
    for v in versions {
        if !sees(txns, pager, marker, &v.header)? {
            continue;
        }
        let same = seen.entry((v.header.xmin, v.header.xmax)).or_default();
        if same.contains(&v.row) {
            continue;
        }
        same.push(v.row.clone());
        out.push(v.row);
    }
    Ok(out)
}

/// Returns the row of constructor `ctor` keyed `key` as the safe snapshot of `marker` reads it:
/// the version reached from an entry of `index`, the current key index, through `current` and
/// the other heaps of `sum`, the heaps of its relation, or else the version an entry of `into`'s
/// history key index names.
///
/// Precondition: `marker` is held by [`Transactions::pin`].
pub fn probe<R: Runtime>(
    pager: &Pager<R>,
    txns: &Transactions<R>,
    marker: &Marker,
    ctor: CtorId,
    (current, sum, index): (&Heap, &[HeapNo], &Index),
    into: &History,
    key: &[Value],
) -> Result<Option<Vec<Value>>, Error> {
    for e in index.probe_ctor(pager, key, ctor)? {
        let mut failed = None;
        let link = walk(pager, current.id(), sum, e.tid, |l| {
            sees(txns, pager, marker, &l.header).unwrap_or_else(|err| {
                failed = Some(err);
                true
            })
        })?;
        if let Some(err) = failed {
            return Err(Error::Page(err));
        }
        if let Some(l) = link
            && l.heap == current.id()
            && let Some(v) = current.read(pager, l.tid)?
        {
            return Ok(Some(v.row));
        }
    }
    let past = into.heap(ctor);
    for e in into.index.probe(pager, key)? {
        if e.ctor != ctor {
            continue;
        }
        if let Some(v) = past.read(pager, e.tid)?
            && sees(txns, pager, marker, &v.header)?
        {
            return Ok(Some(v.row));
        }
    }
    Ok(None)
}
