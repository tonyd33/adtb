use std::collections::{BTreeMap, BTreeSet};
use std::ops::{Bound, RangeBounds};

use crate::key::{CtorId, Tid};
use crate::storage::Pid;

use super::{Cause, SerializationFailure, Xid};

/// A transaction known to the serializer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SerialId(u64);

/// One scan of a heap known to the serializer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScanId(u64);

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
    /// SIREAD locks of every transaction together beyond which the largest holders' locks
    /// promote a level at a time.
    pub total: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            tuples_per_page: 8,
            pages_per_heap: 64,
            ranges_per_index: 64,
            total: 1 << 16,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Point {
    Tuple(u32, Tid),
    Page(u32, u64),
    Heap(u32),
    /// An open scan of a heap, conflicting as a heap lock does.
    Scan(u32, ScanId),
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

#[derive(Clone, Copy, Debug)]
struct Out {
    commit: u64,
    xid: Option<Xid>,
}

fn earlier(a: Option<Out>, b: Out) -> Option<Out> {
    Some(a.filter(|a| a.commit <= b.commit).unwrap_or(b))
}

#[derive(Debug)]
struct Sxact {
    xid: Option<Xid>,
    read_only: bool,
    /// Commits made before the snapshot was taken.
    snapshot: u64,
    /// The commit's position in the order of commits, counting from 1.
    commit: Option<u64>,
    /// The failure it reports once chosen to abort.
    doomed: Option<SerializationFailure>,
    safe: bool,
    unsafe_: bool,
    /// Transactions that read before this one wrote.
    ins: BTreeSet<SerialId>,
    /// Transactions this one read before they wrote.
    outs: BTreeSet<SerialId>,
    /// The earliest commit of a transaction in `outs`, kept after that transaction is forgotten.
    earliest_out: Option<Out>,
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
    scans: u64,
    commits: u64,
    sxacts: BTreeMap<SerialId, Sxact>,
    by_xid: BTreeMap<Xid, SerialId>,
    points: BTreeMap<Point, BTreeSet<SerialId>>,
    ranges: BTreeMap<Pid, Vec<Range>>,
    indexes: BTreeMap<Pid, BTreeMap<SerialId, BTreeSet<CtorId>>>,
    /// SIREAD locks held, each range and index lock counted once.
    held: usize,
    /// Read-only transactions found safe since [`Serializer::take_safe`] last ran.
    found_safe: Vec<SerialId>,
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
        if safe {
            self.found_safe.push(id);
        }
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
                doomed: None,
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
        match &self.get(id).doomed {
            Some(f) => Err(f.clone()),
            None => Ok(()),
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
        self.keep_budget();
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

    /// Record that `id` began a scan of `heap`, covering every version of the heap as a heap
    /// lock does until [`Serializer::end_scan`] ends it. Returns the scan, `None` when it takes
    /// no lock.
    pub fn read_scan(
        &mut self,
        id: SerialId,
        heap: u32,
    ) -> Result<Option<ScanId>, SerializationFailure> {
        if !self.records(id)? || self.get(id).points.contains(&Point::Heap(heap)) {
            return Ok(None);
        }
        self.scans += 1;
        let scan = ScanId(self.scans);
        self.lock(id, Point::Scan(heap, scan));
        self.keep_budget();
        Ok(Some(scan))
    }

    /// End `scan`, a scan of `heap` by `id`, leaving a heap lock in its place, or page locks on
    /// the pages of `read` alone when it is given.
    ///
    /// Preconditions: when `read` is given, what the scan was read for depends on no version on
    /// another page of the heap.
    pub fn end_scan(&mut self, id: SerialId, heap: u32, scan: ScanId, read: Option<&[u64]>) {
        let point = Point::Scan(heap, scan);
        if !self
            .sxacts
            .get(&id)
            .is_some_and(|s| s.points.contains(&point))
        {
            return;
        }
        self.unlock(id, point);
        match read {
            None => self.lock_heap(id, heap),
            Some(pages) => {
                for &page in pages {
                    if !self.get(id).points.contains(&Point::Heap(heap)) {
                        self.lock_page(id, heap, page);
                    }
                }
            }
        }
        self.keep_budget();
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
        self.held += 1;
        let count = self.get_mut(id).ranges.entry(index).or_default();
        *count += 1;
        if *count > self.limits.ranges_per_index {
            self.lock_index(id, index, BTreeSet::new());
        }
        self.keep_budget();
        Ok(())
    }

    fn lock_index(&mut self, id: SerialId, index: Pid, mut ctors: BTreeSet<CtorId>) {
        if let Some(ranges) = self.ranges.get_mut(&index) {
            let before = ranges.len();
            ranges.retain(|r| {
                if r.holder == id {
                    ctors.extend(&r.ctors);
                }
                r.holder != id
            });
            self.held -= before - ranges.len();
        }
        self.get_mut(id).ranges.insert(index, 0);
        let locks = self.indexes.entry(index).or_default();
        if !locks.contains_key(&id) {
            self.held += 1;
        }
        locks.entry(id).or_default().extend(ctors);
    }

    /// Promote the locks of the largest holders a level at a time until the total is within
    /// the budget, or nothing is left to promote.
    fn keep_budget(&mut self) {
        while self.held > self.limits.total {
            let mut holders: Vec<(usize, SerialId)> =
                self.sxacts.keys().map(|&id| (self.locks(id), id)).collect();
            holders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            if !holders.into_iter().any(|(_, id)| self.coarsen(id)) {
                return;
            }
        }
    }

    /// Promote every tuple lock of `id` to its page lock, or failing any, every page lock to its
    /// heap lock, or failing any, every range lock to its index lock. Returns whether a lock was
    /// promoted.
    fn coarsen(&mut self, id: SerialId) -> bool {
        let points: Vec<Point> = self.get(id).points.iter().copied().collect();
        let tuples: BTreeSet<(u32, u64)> = points
            .iter()
            .filter_map(|p| match p {
                Point::Tuple(h, t) => Some((*h, t.page())),
                _ => None,
            })
            .collect();
        if !tuples.is_empty() {
            for p in points.iter().filter(|p| matches!(p, Point::Tuple(..))) {
                self.unlock(id, *p);
            }
            for (heap, page) in tuples {
                self.lock_page(id, heap, page);
            }
            return true;
        }
        let pages: BTreeSet<u32> = points
            .iter()
            .filter_map(|p| match p {
                Point::Page(h, _) => Some(*h),
                _ => None,
            })
            .collect();
        if !pages.is_empty() {
            for heap in pages {
                self.lock_heap(id, heap);
            }
            return true;
        }
        let ranged: Vec<Pid> = self
            .ranges
            .iter()
            .filter(|(_, rs)| rs.iter().any(|r| r.holder == id))
            .map(|(&i, _)| i)
            .collect();
        for &index in &ranged {
            self.lock_index(id, index, BTreeSet::new());
        }
        !ranged.is_empty()
    }

    fn lock(&mut self, id: SerialId, p: Point) {
        if self.get_mut(id).points.insert(p) {
            self.held += 1;
        }
        self.points.entry(p).or_default().insert(id);
    }

    fn unlock(&mut self, id: SerialId, p: Point) {
        if self.get_mut(id).points.remove(&p) {
            self.held -= 1;
        }
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
        self.conflict_in(id, heap, &points)
    }

    /// Record the insert by `id` of a version into `heap`, including the successor an update or
    /// retag writes.
    pub fn insert_heap(&mut self, id: SerialId, heap: u32) -> Result<(), SerializationFailure> {
        self.conflict_in(id, heap, &[Point::Heap(heap)])
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
        for &h in heaps {
            readers.extend(self.scan_readers(h));
        }
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
                Point::Tuple(h, _) | Point::Page(h, _) | Point::Heap(h) | Point::Scan(h, _) => {
                    heaps.contains(h)
                }
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
                    Point::Tuple(h, _) | Point::Page(h, _) | Point::Heap(h) | Point::Scan(h, _) => {
                        heaps.contains(h)
                    }
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
            let before = ranges.len();
            ranges.retain(|r| r.holder != id);
            self.held -= before - ranges.len();
        }
        if let Some(locks) = self.indexes.get_mut(&index)
            && locks.remove(&id).is_some()
        {
            self.held -= 1;
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

    fn scan_readers(&self, heap: u32) -> impl Iterator<Item = SerialId> + '_ {
        self.points
            .range(Point::Scan(heap, ScanId(0))..=Point::Scan(heap, ScanId(u64::MAX)))
            .flat_map(|(_, holders)| holders.iter().copied())
    }

    /// Record a write by `id` into `heap` that the locks on `points` and the open scans of the
    /// heap cover.
    fn conflict_in(
        &mut self,
        id: SerialId,
        heap: u32,
        points: &[Point],
    ) -> Result<(), SerializationFailure> {
        self.check(id)?;
        let mut readers = self.point_readers(points.iter().copied());
        readers.extend(self.scan_readers(heap));
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
        let others_doomed =
            (reader != me && r.doomed.is_some()) || (writer != me && w.doomed.is_some());
        if others_doomed || r.safe || !Serializer::concurrent(r, w) {
            return Ok(());
        }
        let wc = w.commit.map(|commit| Out { commit, xid: w.xid });
        if let Some(members) = self.dangerous(reader, writer) {
            let victim = if writer == me || wc.is_some() {
                me
            } else {
                writer
            };
            let failure = self.failure(victim, members);
            if victim == me {
                return Err(failure);
            }
            self.get_mut(writer).doomed = Some(failure);
        }
        let r = self.get_mut(reader);
        r.outs.insert(writer);
        if let Some(c) = wc {
            r.earliest_out = earlier(r.earliest_out, c);
        }
        self.get_mut(writer).ins.insert(reader);
        Ok(())
    }

    /// Returns the xids of the transactions of the dangerous structure that a new
    /// rw-antidependency `reader` to `writer` completes, if it completes one whose last
    /// transaction committed first.
    fn dangerous(&self, reader: SerialId, writer: SerialId) -> Option<Vec<Option<Xid>>> {
        let (r, w) = (self.get(reader), self.get(writer));
        if let Some(t3) = w.earliest_out
            && r.commit.is_none_or(|rc| t3.commit <= rc)
            && (!r.read_only || t3.commit <= r.snapshot)
        {
            return Some(vec![r.xid, w.xid, t3.xid]);
        }
        let wc = w.commit?;
        if r.read_only {
            return None;
        }
        r.ins
            .iter()
            .map(|t0| self.get(*t0))
            .find(|t0| {
                t0.doomed.is_none()
                    && t0.commit.is_none_or(|c| c >= wc)
                    && (!t0.read_only || t0.snapshot >= wc)
            })
            .map(|t0| vec![t0.xid, r.xid, w.xid])
    }

    /// Returns the failure `victim` reports for breaking a conflict among `members`.
    fn failure(&self, victim: SerialId, members: Vec<Option<Xid>>) -> SerializationFailure {
        let own = self.get(victim).xid;
        let mut with: Vec<Xid> = Vec::new();
        for x in members.into_iter().flatten() {
            if Some(x) != own && !with.contains(&x) {
                with.push(x);
            }
        }
        SerializationFailure::new(Cause::Dangerous, with)
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
            if !n.active() || n.doomed.is_some() {
                continue;
            }
            let far_live = n.ins.iter().find(|far| {
                let f = self.get(**far);
                **far == id || (f.active() && !f.read_only && f.doomed.is_none())
            });
            if let Some(far) = far_live {
                let members = vec![self.get(*far).xid, n.xid, self.get(id).xid];
                doom.push((*near, self.failure(*near, members)));
            }
        }
        for (n, failure) in doom {
            self.get_mut(n).doomed = Some(failure);
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
        let out = Out {
            commit: c,
            xid: s.xid,
        };
        let ins: Vec<SerialId> = s.ins.iter().copied().collect();
        let earliest_out = s.earliest_out;
        for r in ins {
            let r = self.get_mut(r);
            r.earliest_out = earlier(r.earliest_out, out);
        }
        let mut resolved = Vec::new();
        for (&ro, s) in &mut self.sxacts {
            if s.read_only && s.pending.remove(&id) {
                if earliest_out.is_some_and(|e| e.commit <= s.snapshot) {
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
            self.found_safe.push(ro);
        }
    }

    /// Returns the read-only transactions whose snapshots were found safe since the last call,
    /// in the order they were found. The caller logs a safe snapshot marker for each.
    pub fn take_safe(&mut self) -> Vec<SerialId> {
        std::mem::take(&mut self.found_safe)
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

    /// Returns the number of SIREAD locks held, each range and index lock counted once.
    pub fn held(&self) -> usize {
        self.held
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
