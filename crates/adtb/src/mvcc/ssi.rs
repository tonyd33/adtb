use std::collections::{BTreeMap, BTreeSet};
use std::ops::{Bound, RangeBounds};

use crate::key::{CtorId, Tid};
use crate::storage::Pid;

use super::{SerializationFailure, Xid};

/// A transaction known to the serializer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SerialId(u64);

/// How a transaction may run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// A transaction that may write, as `xid`.
    ReadWrite(Xid),
    ReadOnly,
    /// A read-only transaction that reads only once its snapshot is safe.
    Deferrable,
}

/// Whether a read-only transaction's snapshot is safe: no concurrent read-write transaction can
/// still make it part of a dangerous structure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Safety {
    /// Safe: the transaction holds no SIREAD locks and never fails with a serialization failure.
    Safe,
    /// Unsafe: a concurrent writer committed with a rw-antidependency on a transaction that
    /// committed before the snapshot. A deferrable transaction retakes its snapshot.
    Unsafe,
    /// Undecided until each of these read-write transactions ends.
    Pending(Vec<Xid>),
}

/// The thresholds at which a transaction's SIREAD locks promote to coarser ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Tuple locks on one page beyond which they become a page lock.
    pub tuples_per_page: usize,
    /// Page locks on one heap beyond which they become a heap lock.
    pub pages_per_heap: usize,
    /// Range locks on one index beyond which they become an index lock.
    pub ranges_per_index: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            tuples_per_page: 8,
            pages_per_heap: 64,
            ranges_per_index: 64,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Point {
    Tuple(u32, Tid),
    Page(u32, u64),
    Heap(u32),
    Schema(u64),
}

#[derive(Clone, Debug)]
struct Range {
    holder: SerialId,
    lo: Bound<Vec<u8>>,
    hi: Bound<Vec<u8>>,
    ctors: BTreeSet<CtorId>,
}

impl Range {
    fn covers(&self, key: &[u8], ctor: CtorId) -> bool {
        self.ctors.contains(&ctor)
            && (
                self.lo.as_ref().map(Vec::as_slice),
                self.hi.as_ref().map(Vec::as_slice),
            )
                .contains(key)
    }
}

#[derive(Debug)]
struct Sxact {
    xid: Option<Xid>,
    read_only: bool,
    /// Commits made before the snapshot was taken.
    snapshot: u64,
    /// The commit's position in the order of commits, counting from 1.
    commit: Option<u64>,
    doomed: bool,
    safe: bool,
    unsafe_: bool,
    /// Transactions that read before this one wrote.
    ins: BTreeSet<SerialId>,
    /// Transactions this one read before they wrote.
    outs: BTreeSet<SerialId>,
    /// The earliest commit of a transaction in `outs`, kept after that transaction is forgotten.
    earliest_out: Option<u64>,
    /// For a read-only transaction, the read-write transactions concurrent with its snapshot
    /// that have not ended.
    pending: BTreeSet<SerialId>,
    points: BTreeSet<Point>,
    ranges: BTreeMap<Pid, usize>,
}

impl Sxact {
    fn active(&self) -> bool {
        self.commit.is_none()
    }
}

/// The SIREAD locks and rw-antidependencies of serializable snapshot isolation.
///
/// Preconditions on the caller:
///
/// - `begin` runs in the same critical section as taking the transaction's snapshot, and
///   `committed` in the same one as recording the commit in the commit log, so a transaction
///   counts as committed before a snapshot exactly when its commit preceded the snapshot.
/// - A read takes its lock before it reads the page, or in one critical section with the read; a
///   write checks the locks after its change is in the page, or in one critical section with it.
#[derive(Debug, Default)]
pub struct Serializer {
    limits: Limits,
    issued: u64,
    commits: u64,
    sxacts: BTreeMap<SerialId, Sxact>,
    by_xid: BTreeMap<Xid, SerialId>,
    points: BTreeMap<Point, BTreeSet<SerialId>>,
    ranges: BTreeMap<Pid, Vec<Range>>,
    indexes: BTreeMap<Pid, BTreeMap<SerialId, BTreeSet<CtorId>>>,
}

impl Serializer {
    pub fn new(limits: Limits) -> Serializer {
        Serializer {
            limits,
            ..Serializer::default()
        }
    }

    /// Register a transaction whose snapshot is being taken now.
    ///
    /// Panics if a running transaction already has the xid.
    pub fn begin(&mut self, access: Access) -> SerialId {
        self.issued += 1;
        let id = SerialId(self.issued);
        let (xid, read_only) = match access {
            Access::ReadWrite(x) => (Some(x), false),
            Access::ReadOnly | Access::Deferrable => (None, true),
        };
        let pending = if read_only {
            self.sxacts
                .iter()
                .filter(|(_, s)| s.active() && !s.read_only)
                .map(|(&i, _)| i)
                .collect()
        } else {
            BTreeSet::new()
        };
        let safe = read_only && pending.is_empty();
        if let Some(x) = xid {
            assert!(
                self.by_xid.insert(x, id).is_none(),
                "{x:?} already has a transaction"
            );
        }
        self.sxacts.insert(
            id,
            Sxact {
                xid,
                read_only,
                snapshot: self.commits,
                commit: None,
                doomed: false,
                safe,
                unsafe_: false,
                ins: BTreeSet::new(),
                outs: BTreeSet::new(),
                earliest_out: None,
                pending,
                points: BTreeSet::new(),
                ranges: BTreeMap::new(),
            },
        );
        id
    }

    fn get(&self, id: SerialId) -> &Sxact {
        self.sxacts
            .get(&id)
            .unwrap_or_else(|| panic!("{id:?} is not running"))
    }

    fn get_mut(&mut self, id: SerialId) -> &mut Sxact {
        self.sxacts
            .get_mut(&id)
            .unwrap_or_else(|| panic!("{id:?} is not running"))
    }

    /// Returns the safety of a read-only transaction's snapshot; a read-write transaction's is
    /// never safe.
    pub fn safety(&self, id: SerialId) -> Safety {
        let s = self.get(id);
        if s.safe {
            Safety::Safe
        } else if s.unsafe_ || !s.read_only {
            Safety::Unsafe
        } else {
            Safety::Pending(
                s.pending
                    .iter()
                    .filter_map(|p| self.sxacts.get(p).and_then(|p| p.xid))
                    .collect(),
            )
        }
    }

    /// Fails if `id` has been chosen to abort.
    pub fn check(&self, id: SerialId) -> Result<(), SerializationFailure> {
        if self.get(id).doomed {
            Err(SerializationFailure)
        } else {
            Ok(())
        }
    }

    fn records(&self, id: SerialId) -> Result<bool, SerializationFailure> {
        self.check(id)?;
        Ok(!self.get(id).safe)
    }

    /// Record that `id` read the version at `tid` of `heap`.
    pub fn read_tuple(
        &mut self,
        id: SerialId,
        heap: u32,
        tid: Tid,
    ) -> Result<(), SerializationFailure> {
        if !self.records(id)? || self.holds_tuple(id, heap, tid) {
            return Ok(());
        }
        self.lock(id, Point::Tuple(heap, tid));
        let page = tid.page();
        let on_page = self.tuples_on(id, heap, page);
        if on_page.len() > self.limits.tuples_per_page {
            for p in on_page {
                self.unlock(id, p);
            }
            self.lock_page(id, heap, page);
        }
        Ok(())
    }

    fn holds_tuple(&self, id: SerialId, heap: u32, tid: Tid) -> bool {
        let points = &self.get(id).points;
        points.contains(&Point::Tuple(heap, tid))
            || points.contains(&Point::Page(heap, tid.page()))
            || points.contains(&Point::Heap(heap))
    }

    fn tuples_on(&self, id: SerialId, heap: u32, page: u64) -> Vec<Point> {
        let first = Tid::new(page, 0).expect("a TID's page fits 48 bits");
        let last = Tid::new(page, u16::MAX).expect("a TID's page fits 48 bits");
        self.get(id)
            .points
            .range(Point::Tuple(heap, first)..=Point::Tuple(heap, last))
            .copied()
            .collect()
    }

    fn lock_page(&mut self, id: SerialId, heap: u32, page: u64) {
        self.lock(id, Point::Page(heap, page));
        let pages: Vec<Point> = self
            .get(id)
            .points
            .range(Point::Page(heap, 0)..=Point::Page(heap, u64::MAX))
            .copied()
            .collect();
        if pages.len() > self.limits.pages_per_heap {
            self.lock_heap(id, heap);
        }
    }

    fn lock_heap(&mut self, id: SerialId, heap: u32) {
        let finer: Vec<Point> = self
            .get(id)
            .points
            .iter()
            .filter(|p| matches!(p, Point::Tuple(h, _) | Point::Page(h, _) if *h == heap))
            .copied()
            .collect();
        for p in finer {
            self.unlock(id, p);
        }
        self.lock(id, Point::Heap(heap));
    }

    /// Record that `id` scanned all of `heap`.
    pub fn read_heap(&mut self, id: SerialId, heap: u32) -> Result<(), SerializationFailure> {
        if self.records(id)? {
            self.lock_heap(id, heap);
        }
        Ok(())
    }

    /// Record that `id` ran a statement at schema version `version`.
    pub fn read_schema(&mut self, id: SerialId, version: u64) -> Result<(), SerializationFailure> {
        if self.records(id)? {
            self.lock(id, Point::Schema(version));
        }
        Ok(())
    }

    /// Record that `id` scanned the keys of `index` between `lo` and `hi`, as encoded keys, for
    /// rows of the constructors `ctors`.
    ///
    /// A probe of a hash index records the probed hash, as its four big-endian bytes, as both
    /// bounds.
    pub fn read_range(
        &mut self,
        id: SerialId,
        index: Pid,
        lo: Bound<Vec<u8>>,
        hi: Bound<Vec<u8>>,
        ctors: BTreeSet<CtorId>,
    ) -> Result<(), SerializationFailure> {
        if !self.records(id)? {
            return Ok(());
        }
        if let Some(held) = self.indexes.get(&index).and_then(|i| i.get(&id))
            && ctors.is_subset(held)
        {
            return Ok(());
        }
        self.ranges.entry(index).or_default().push(Range {
            holder: id,
            lo,
            hi,
            ctors,
        });
        let count = self.get_mut(id).ranges.entry(index).or_default();
        *count += 1;
        if *count > self.limits.ranges_per_index {
            self.lock_index(id, index, BTreeSet::new());
        }
        Ok(())
    }

    fn lock_index(&mut self, id: SerialId, index: Pid, mut ctors: BTreeSet<CtorId>) {
        if let Some(ranges) = self.ranges.get_mut(&index) {
            ranges.retain(|r| {
                if r.holder == id {
                    ctors.extend(&r.ctors);
                }
                r.holder != id
            });
        }
        self.get_mut(id).ranges.insert(index, 0);
        self.indexes
            .entry(index)
            .or_default()
            .entry(id)
            .or_default()
            .extend(ctors);
    }

    fn lock(&mut self, id: SerialId, p: Point) {
        self.get_mut(id).points.insert(p);
        self.points.entry(p).or_default().insert(id);
    }

    fn unlock(&mut self, id: SerialId, p: Point) {
        self.get_mut(id).points.remove(&p);
        if let Some(holders) = self.points.get_mut(&p) {
            holders.remove(&id);
            if holders.is_empty() {
                self.points.remove(&p);
            }
        }
    }

    /// Record that `id` read a version whose state depends on `writer`, a writer its snapshot
    /// does not count as committed: an inserter of a version it misses, or the deleter of one it
    /// sees.
    ///
    /// Fails if the rw-antidependency completes a dangerous structure that only `id` can break.
    pub fn read_from(&mut self, id: SerialId, writer: Xid) -> Result<(), SerializationFailure> {
        if !self.records(id)? {
            return Ok(());
        }
        match self.by_xid.get(&writer) {
            Some(&w) => self.conflict(id, w, id),
            None => Ok(()),
        }
    }

    /// Record the write by `id` of the version at `tid` of `heap`: an update, delete or retag.
    pub fn write_tuple(
        &mut self,
        id: SerialId,
        heap: u32,
        tid: Tid,
    ) -> Result<(), SerializationFailure> {
        let points = [
            Point::Tuple(heap, tid),
            Point::Page(heap, tid.page()),
            Point::Heap(heap),
        ];
        self.conflict_in(id, &points)
    }

    /// Record the insert by `id` of a version into `heap`, including the successor an update or
    /// retag writes.
    pub fn insert_heap(&mut self, id: SerialId, heap: u32) -> Result<(), SerializationFailure> {
        self.conflict_in(id, &[Point::Heap(heap)])
    }

    /// Record the insert by `id` of an entry with encoded key `key` for constructor `ctor` into
    /// `index`.
    pub fn insert_entry(
        &mut self,
        id: SerialId,
        index: Pid,
        key: &[u8],
        ctor: CtorId,
    ) -> Result<(), SerializationFailure> {
        self.check(id)?;
        let readers = self.range_readers(index, |r| r.covers(key, ctor), |c| c.contains(&ctor));
        self.conflicts_in(id, readers)
    }

    /// Record a bulk load by `id` that fills `heaps` and adds entries for the constructors given
    /// with each index of `indexes`.
    pub fn bulk_insert(
        &mut self,
        id: SerialId,
        heaps: &[u32],
        indexes: &[(Pid, BTreeSet<CtorId>)],
    ) -> Result<(), SerializationFailure> {
        self.check(id)?;
        let mut readers = self.point_readers(heaps.iter().map(|&h| Point::Heap(h)));
        for (index, filled) in indexes {
            readers.extend(self.range_readers(
                *index,
                |r| !r.ctors.is_disjoint(filled),
                |c| !c.is_disjoint(filled),
            ));
        }
        self.conflicts_in(id, readers)
    }

    /// Record a migration by `id` that produces schema version `version`.
    pub fn migrate(&mut self, id: SerialId, version: u64) -> Result<(), SerializationFailure> {
        self.check(id)?;
        let readers = self
            .points
            .range(Point::Schema(0)..Point::Schema(version))
            .flat_map(|(_, h)| h.iter().copied())
            .collect();
        self.conflicts_in(id, readers)
    }

    /// Replace every tuple, page and heap lock on `heaps` and every lock on `indexes` by heap
    /// locks on each heap of `into`, for a split or merge of constructors that rewrites them.
    pub fn rewrite(&mut self, heaps: &[u32], indexes: &[Pid], into: &[u32]) {
        let mut holders = BTreeSet::new();
        for (id, s) in &self.sxacts {
            let on_heap = s.points.iter().any(|p| match p {
                Point::Tuple(h, _) | Point::Page(h, _) | Point::Heap(h) => heaps.contains(h),
                Point::Schema(_) => false,
            });
            let on_index = indexes.iter().any(|i| {
                s.ranges.contains_key(i) || self.indexes.get(i).is_some_and(|l| l.contains_key(id))
            });
            if on_heap || on_index {
                holders.insert(*id);
            }
        }
        for id in holders {
            let old: Vec<Point> = self
                .get(id)
                .points
                .iter()
                .filter(|p| match p {
                    Point::Tuple(h, _) | Point::Page(h, _) | Point::Heap(h) => heaps.contains(h),
                    Point::Schema(_) => false,
                })
                .copied()
                .collect();
            for p in old {
                self.unlock(id, p);
            }
            for i in indexes {
                self.release_index(id, *i);
            }
            for &h in into {
                self.lock(id, Point::Heap(h));
            }
        }
    }

    fn release_index(&mut self, id: SerialId, index: Pid) {
        if let Some(ranges) = self.ranges.get_mut(&index) {
            ranges.retain(|r| r.holder != id);
        }
        if let Some(locks) = self.indexes.get_mut(&index) {
            locks.remove(&id);
        }
        self.get_mut(id).ranges.remove(&index);
    }

    fn point_readers(&self, points: impl IntoIterator<Item = Point>) -> BTreeSet<SerialId> {
        points
            .into_iter()
            .filter_map(|p| self.points.get(&p))
            .flatten()
            .copied()
            .collect()
    }

    fn range_readers(
        &self,
        index: Pid,
        range: impl Fn(&Range) -> bool,
        whole: impl Fn(&BTreeSet<CtorId>) -> bool,
    ) -> BTreeSet<SerialId> {
        let ranges = self.ranges.get(&index).into_iter().flatten();
        let locks = self.indexes.get(&index).into_iter().flatten();
        ranges
            .filter(|r| range(r))
            .map(|r| r.holder)
            .chain(locks.filter(|(_, c)| whole(c)).map(|(&id, _)| id))
            .collect()
    }

    fn conflict_in(&mut self, id: SerialId, points: &[Point]) -> Result<(), SerializationFailure> {
        self.check(id)?;
        let readers = self.point_readers(points.iter().copied());
        self.conflicts_in(id, readers)
    }

    fn conflicts_in(
        &mut self,
        id: SerialId,
        readers: BTreeSet<SerialId>,
    ) -> Result<(), SerializationFailure> {
        for r in readers {
            self.conflict(r, id, id)?;
        }
        Ok(())
    }

    fn concurrent(a: &Sxact, b: &Sxact) -> bool {
        let before = |x: &Sxact, y: &Sxact| x.commit.is_some_and(|c| c <= y.snapshot);
        !before(a, b) && !before(b, a)
    }

    /// Add the rw-antidependency `reader` to `writer`, found by `me`, one of them.
    fn conflict(
        &mut self,
        reader: SerialId,
        writer: SerialId,
        me: SerialId,
    ) -> Result<(), SerializationFailure> {
        if reader == writer {
            return Ok(());
        }
        let (r, w) = (self.get(reader), self.get(writer));
        let others_doomed = (reader != me && r.doomed) || (writer != me && w.doomed);
        if others_doomed || r.safe || !Serializer::concurrent(r, w) {
            return Ok(());
        }
        let wc = w.commit;
        if self.dangerous(reader, writer) {
            if writer == me || wc.is_some() {
                return Err(SerializationFailure);
            }
            self.get_mut(writer).doomed = true;
        }
        let r = self.get_mut(reader);
        r.outs.insert(writer);
        if let Some(c) = wc {
            r.earliest_out = Some(r.earliest_out.map_or(c, |e| e.min(c)));
        }
        self.get_mut(writer).ins.insert(reader);
        Ok(())
    }

    /// Returns whether a new rw-antidependency `reader` to `writer` completes a dangerous
    /// structure whose last transaction committed first.
    fn dangerous(&self, reader: SerialId, writer: SerialId) -> bool {
        let (r, w) = (self.get(reader), self.get(writer));
        let through_writer = w.earliest_out.is_some_and(|c3| {
            r.commit.is_none_or(|rc| c3 <= rc) && (!r.read_only || c3 <= r.snapshot)
        });
        let into_reader = w.commit.is_some_and(|wc| {
            !r.read_only
                && r.ins.iter().any(|t0| {
                    let t0 = self.get(*t0);
                    !t0.doomed
                        && t0.commit.is_none_or(|c| c >= wc)
                        && (!t0.read_only || t0.snapshot >= wc)
                })
        });
        through_writer || into_reader
    }

    /// Check that `id` may commit, choosing to abort any transaction its commit would leave
    /// as the pivot of a dangerous structure.
    ///
    /// Fails if `id` has been chosen to abort.
    pub fn precommit(&mut self, id: SerialId) -> Result<(), SerializationFailure> {
        self.check(id)?;
        let mut doom = Vec::new();
        for near in &self.get(id).ins {
            let n = self.get(*near);
            if !n.active() || n.doomed {
                continue;
            }
            let far_live = n.ins.iter().any(|far| {
                let f = self.get(*far);
                *far == id || (f.active() && !f.read_only && !f.doomed)
            });
            if far_live {
                doom.push(*near);
            }
        }
        for n in doom {
            self.get_mut(n).doomed = true;
        }
        Ok(())
    }

    /// Record that `id` committed. Returns the read-only transactions whose snapshots are no
    /// longer pending.
    pub fn committed(&mut self, id: SerialId) -> Vec<SerialId> {
        self.commits += 1;
        let c = self.commits;
        let s = self.get_mut(id);
        s.commit = Some(c);
        let ins: Vec<SerialId> = s.ins.iter().copied().collect();
        let earliest_out = s.earliest_out;
        for r in ins {
            let r = self.get_mut(r);
            r.earliest_out = Some(r.earliest_out.map_or(c, |e| e.min(c)));
        }
        let mut resolved = Vec::new();
        for (&ro, s) in &mut self.sxacts {
            if s.read_only && s.pending.remove(&id) {
                if earliest_out.is_some_and(|e| e <= s.snapshot) {
                    s.unsafe_ = true;
                }
                if s.unsafe_ || s.pending.is_empty() {
                    resolved.push(ro);
                }
            }
        }
        self.settle(&resolved);
        self.forget();
        resolved
    }

    /// Record that `id` aborted, or that a read-only transaction ended or retakes its snapshot.
    /// Returns the read-only transactions whose snapshots are no longer pending.
    pub fn aborted(&mut self, id: SerialId) -> Vec<SerialId> {
        self.drop_sxact(id);
        let mut resolved = Vec::new();
        for (&ro, s) in &mut self.sxacts {
            if s.read_only && s.pending.remove(&id) && s.pending.is_empty() {
                resolved.push(ro);
            }
        }
        self.settle(&resolved);
        self.forget();
        resolved
    }

    /// Make every resolved read-only transaction that is not unsafe safe, dropping its locks and
    /// rw-antidependencies.
    fn settle(&mut self, resolved: &[SerialId]) {
        for &ro in resolved {
            let s = self.get(ro);
            if s.unsafe_ || !s.active() {
                continue;
            }
            self.release_locks(ro);
            let outs: Vec<SerialId> = std::mem::take(&mut self.get_mut(ro).outs)
                .into_iter()
                .collect();
            for w in outs {
                if let Some(w) = self.sxacts.get_mut(&w) {
                    w.ins.remove(&ro);
                }
            }
            self.get_mut(ro).safe = true;
        }
    }

    fn release_locks(&mut self, id: SerialId) {
        let points: Vec<Point> = self.get(id).points.iter().copied().collect();
        for p in points {
            self.unlock(id, p);
        }
        let indexes: Vec<Pid> = self
            .indexes
            .keys()
            .chain(self.ranges.keys())
            .copied()
            .collect();
        for i in indexes {
            self.release_index(id, i);
        }
    }

    fn drop_sxact(&mut self, id: SerialId) {
        self.release_locks(id);
        let s = self
            .sxacts
            .remove(&id)
            .unwrap_or_else(|| panic!("{id:?} is not running"));
        if let Some(x) = s.xid {
            self.by_xid.remove(&x);
        }
        for o in s.ins.iter().chain(&s.outs) {
            if let Some(o) = self.sxacts.get_mut(o) {
                o.ins.remove(&id);
                o.outs.remove(&id);
            }
        }
    }

    /// Forget every committed transaction that committed before the snapshot of every active
    /// one, so none running or yet to start is concurrent with it.
    fn forget(&mut self) {
        let horizon = self
            .sxacts
            .values()
            .filter(|s| s.active() && !s.safe)
            .map(|s| s.snapshot)
            .min()
            .unwrap_or(self.commits);
        let done: Vec<SerialId> = self
            .sxacts
            .iter()
            .filter(|(_, s)| s.commit.is_some_and(|c| c <= horizon))
            .map(|(&id, _)| id)
            .collect();
        for id in done {
            self.drop_sxact(id);
        }
    }

    /// Returns the transactions `id` read before they wrote.
    pub fn writers_after(&self, id: SerialId) -> BTreeSet<SerialId> {
        self.get(id).outs.clone()
    }

    /// Returns the transactions that read before `id` wrote.
    pub fn readers_before(&self, id: SerialId) -> BTreeSet<SerialId> {
        self.get(id).ins.clone()
    }

    /// Returns the number of transactions the serializer still tracks, running or committed.
    pub fn tracked(&self) -> usize {
        self.sxacts.len()
    }

    /// Returns the number of SIREAD locks `id` holds, each range and index lock counted once.
    pub fn locks(&self, id: SerialId) -> usize {
        let ranges = self
            .ranges
            .values()
            .flatten()
            .filter(|r| r.holder == id)
            .count();
        let indexes = self
            .indexes
            .values()
            .filter(|l| l.contains_key(&id))
            .count();
        self.get(id).points.len() + ranges + indexes
    }
}
