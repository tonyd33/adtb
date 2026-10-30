use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ops::{Bound, ControlFlow, RangeBounds};
use std::sync::Arc;

use adtb_lang::schema::Schema;
use adtb_lang::types::Ty;

use crate::batch::{Batch, BatchBuilder, Schema as BatchSchema};
use crate::btree::{self, Index, prefix_end};
use crate::catalog::IndexKeys;
use crate::heap::{FieldType, Header, Heap, HeapNo, Version, walk};
use crate::key::{CtorId, Dir, KeySchema, KeyType, Scalar, Tid, Value};
use crate::mvcc::{
    ScanId, SerialId, SerializationFailure, Serializer, Snapshot, Status, Transactions, Xid,
};
use crate::plan::{HeapId, IndexId};
use crate::runtime::Runtime;
use crate::storage::{PageError, Pager, Pid, Quarantine};
use adtb_sys::lock::Lock;

use super::Error;
use super::memory::{Arena, Grant};
use super::types::{ctors, repr};
use super::value::{F32, F64, Val};

/// What a query's reads see and record: the visibility of versions at its snapshot, and the
/// SIREAD locks its transaction takes.
pub trait Reads: Send + Sync {
    /// Returns whether the version with header `h` is visible at the reader's snapshot.
    fn sees(&self, h: &Header) -> Result<bool, PageError>;

    /// Record that a scan of heap `heap` began, covering every version of the heap until
    /// [`Reads::scanned`] ends it. Returns the scan, `None` when it records none.
    fn scanning(&self, heap: HeapNo) -> Result<Option<ScanId>, SerializationFailure>;

    /// Record that `scan`, a scan of heap `heap`, ended having read every version of the heap,
    /// or only those on the pages of `read` when it is given.
    ///
    /// Preconditions: when `read` is given, what the scan was read for depends on no version on
    /// another page of the heap.
    fn scanned(&self, heap: HeapNo, scan: ScanId, read: Option<&[Pid]>);

    /// Record that the encoded keys of the index rooted at `index` between `lo` and `hi` were
    /// read, for rows of `ctors`.
    fn probed(
        &self,
        index: Pid,
        lo: Bound<Vec<u8>>,
        hi: Bound<Vec<u8>>,
        ctors: BTreeSet<CtorId>,
    ) -> Result<(), SerializationFailure>;

    /// Record that the version at `tid` of heap `heap` was read.
    fn fetched(&self, heap: HeapNo, tid: Tid) -> Result<(), SerializationFailure>;

    /// Record that what the reader saw depends on `writer`, the inserter of a version it missed
    /// or the deleter of one it saw, when its snapshot does not count `writer` as committed and
    /// `writer` did not abort.
    fn depends(&self, writer: Xid) -> Result<(), Error>;
}

/// Returns the writer the visibility of the version with header `h` depends on, `seen` its
/// visibility: its deleter when seen, its inserter when not; `None` when it has no deleter.
fn writer_of(h: &Header, seen: bool) -> Option<Xid> {
    let x = Xid(if seen { h.xmax } else { h.xmin });
    (x != Xid::NONE).then_some(x)
}

/// Reads at a snapshot of the transactions layer, recording SIREAD locks with `serial` when it
/// is given.
pub struct AtSnapshot<R: Runtime> {
    pub rt: R,
    pub pager: Pager<R>,
    pub txns: Arc<Transactions<R>>,
    pub snapshot: Snapshot,
    pub reader: Option<Xid>,
    pub serial: Option<(Arc<Lock<Serializer>>, SerialId)>,
}

impl<R: Runtime> std::fmt::Debug for AtSnapshot<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AtSnapshot")
            .field("snapshot", &self.snapshot)
            .field("reader", &self.reader)
            .finish_non_exhaustive()
    }
}

impl<R: Runtime> Reads for AtSnapshot<R> {
    fn sees(&self, h: &Header) -> Result<bool, PageError> {
        self.txns
            .sees(&self.pager, &self.snapshot, self.reader, h.xmin, h.xmax)
    }

    fn scanning(&self, heap: HeapNo) -> Result<Option<ScanId>, SerializationFailure> {
        match &self.serial {
            Some((s, id)) => s.lock(&self.rt).read_scan(*id, heap.0),
            None => Ok(None),
        }
    }

    fn scanned(&self, heap: HeapNo, scan: ScanId, read: Option<&[Pid]>) {
        if let Some((s, id)) = &self.serial {
            let pages: Option<Vec<u64>> = read.map(|r| r.iter().map(|p| p.0).collect());
            s.lock(&self.rt)
                .end_scan(*id, heap.0, scan, pages.as_deref());
        }
    }

    fn probed(
        &self,
        index: Pid,
        lo: Bound<Vec<u8>>,
        hi: Bound<Vec<u8>>,
        ctors: BTreeSet<CtorId>,
    ) -> Result<(), SerializationFailure> {
        match &self.serial {
            Some((s, id)) => s.lock(&self.rt).read_range(*id, index, lo, hi, ctors),
            None => Ok(()),
        }
    }

    fn fetched(&self, heap: HeapNo, tid: Tid) -> Result<(), SerializationFailure> {
        match &self.serial {
            Some((s, id)) => s.lock(&self.rt).read_tuple(*id, heap.0, tid),
            None => Ok(()),
        }
    }

    fn depends(&self, writer: Xid) -> Result<(), Error> {
        let Some((s, id)) = &self.serial else {
            return Ok(());
        };
        if Some(writer) == self.reader {
            return Ok(());
        }
        let status = |x| self.txns.status(&self.pager, x);
        if self
            .snapshot
            .counts_committed(writer, status)
            .map_err(Error::Page)?
            || status(writer).map_err(Error::Page)? == Status::Aborted
        {
            return Ok(());
        }
        s.lock(&self.rt)
            .read_from(*id, writer)
            .map_err(Error::Serialization)
    }
}

/// A heap a plan names, with the field types of its rows.
#[derive(Clone, Debug)]
pub struct StoredHeap {
    pub heap: Arc<Heap>,
    pub row: Vec<FieldType>,
    /// The heap of every constructor of its relation, its own among them.
    pub sum: Vec<HeapNo>,
}

/// An index a plan names, with the constructor position of each constructor id it holds.
#[derive(Clone, Debug)]
pub struct StoredIndex {
    pub index: Index,
    pub ctors: BTreeMap<CtorId, u32>,
    /// What answers a probe from the heaps while the index is quarantined; a probe of a
    /// quarantined index without it reads the index.
    pub rows: Option<IndexedRows>,
}

/// How the rows of a heap give the entries of an index over them.
#[derive(Clone, Debug)]
pub struct IndexedRows {
    pub keys: Arc<IndexKeys>,
    /// The heap of each constructor position the index holds.
    pub heaps: BTreeMap<u32, HeapId>,
}

/// What bounds a probe answered from the heaps while its index is quarantined.
#[derive(Clone, Copy)]
pub struct ProbeBounds<'a> {
    /// The grant holding the versions it scans.
    pub grant: &'a Arc<Grant>,
    /// Fail when the call is cancelled or its deadline has passed.
    pub check: &'a dyn Fn() -> Result<(), Error>,
}

impl std::fmt::Debug for ProbeBounds<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProbeBounds")
            .field("grant", self.grant)
            .finish_non_exhaustive()
    }
}

/// The history heap of each heap and the history key index of each key index, which a read of
/// the past reads besides them.
#[derive(Clone, Debug, Default)]
pub struct Histories {
    pub heaps: BTreeMap<HeapId, StoredHeap>,
    pub indexes: BTreeMap<IndexId, Index>,
}

/// Reads of the past a statement makes.
pub trait PastReads<R: Runtime>: Send + Sync {
    /// Returns the store that reads the state `at` names, a `Position` or, when `instant`, an
    /// `Instant`, for a block reading `heaps` and typed at version `pins`, or at the version of
    /// that state for `None`. The state stays readable while the store lives.
    ///
    /// Fail with [`Error::HistoryUnavailable`] when the state is outside the retention window,
    /// after the reader's snapshot, or before history of a heap read begins, and with
    /// [`Error::WrongVersion`] when a heap read is stored otherwise at that state than at `pins`.
    fn store(
        &self,
        at: &Val,
        instant: bool,
        pins: Option<u64>,
        heaps: &BTreeSet<HeapId>,
    ) -> Result<Arc<Store<R>>, Error>;
}

/// The heaps and indexes a plan's reads go to, and what those reads see.
///
/// Invariants: `heaps` and `indexes` hold every heap and index of the plan's `Reads`. When
/// `history` holds a heap, the store reads the past: a scan of that heap reads its history heap
/// too, a key lookup falls back to its history key index, and no plan probes another index.
pub struct Store<R: Runtime> {
    pub pager: Pager<R>,
    pub reads: Arc<dyn Reads>,
    pub heaps: BTreeMap<HeapId, StoredHeap>,
    pub indexes: BTreeMap<IndexId, StoredIndex>,
    pub history: Histories,
    /// The reads of the past the store's statement may make; `None` once it reads the past.
    pub past: Option<Arc<dyn PastReads<R>>>,
}

impl<R: Runtime> std::fmt::Debug for Store<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("heaps", &self.heaps.keys().collect::<Vec<_>>())
            .field("indexes", &self.indexes.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// The pages of a heap a scan lists, in directory order, with the scan's record that it reads
/// the heap. Dropping it records that the scan read every version of the heap.
pub struct Listing {
    reads: Arc<dyn Reads>,
    heap: HeapNo,
    scan: Option<ScanId>,
    pub pages: Vec<Pid>,
}

impl Listing {
    /// Record that the scan read only the versions on the pages of `read`.
    ///
    /// Preconditions: what the scan was read for depends on no version on another page of the
    /// heap.
    pub fn stopped(mut self, read: &[Pid]) {
        if let Some(scan) = self.scan.take() {
            self.reads.scanned(self.heap, scan, Some(read));
        }
    }
}

impl Drop for Listing {
    fn drop(&mut self) {
        if let Some(scan) = self.scan.take() {
            self.reads.scanned(self.heap, scan, None);
        }
    }
}

impl std::fmt::Debug for Listing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listing")
            .field("heap", &self.heap)
            .field("scan", &self.scan)
            .field("pages", &self.pages)
            .finish_non_exhaustive()
    }
}

/// The batch a scan is filling, its rows held in an arena of the call's grant.
struct Filling {
    schema: BatchSchema,
    rows: usize,
    batch: BatchBuilder,
    held: Arena,
}

impl Filling {
    fn new(row: &[FieldType], rows: usize, grant: &Arc<Grant>) -> Filling {
        let schema = BatchSchema::of_heap(row);
        Filling {
            batch: BatchBuilder::new(schema.clone()),
            schema,
            rows,
            held: grant.arena(),
        }
    }

    /// Append `row`. Returns the batch once it holds its `rows` rows, with the arena holding it.
    fn push(&mut self, row: &[Value]) -> Result<Option<(Batch, Arena)>, Error> {
        self.batch.push_row(row);
        self.held.grow(self.batch.bytes() - self.held.bytes())?;
        Ok((self.batch.len() == self.rows).then(|| self.take()))
    }

    fn take(&mut self) -> (Batch, Arena) {
        let fresh = BatchBuilder::new(self.schema.clone());
        let arena = self.held.grant().arena();
        (
            std::mem::replace(&mut self.batch, fresh).finish(),
            std::mem::replace(&mut self.held, arena),
        )
    }

    /// Returns the rows appended since the last full batch, with the arena holding them.
    fn rest(mut self) -> Option<(Batch, Arena)> {
        (!self.batch.is_empty()).then(|| self.take())
    }
}

/// The deleted versions a read of the past passed from a current heap, by inserter and deleter,
/// held in an arena of the call's grant.
struct Passed {
    versions: HashMap<(u64, u64), HashSet<Vec<Value>>>,
    held: Arena,
}

impl Passed {
    fn new(grant: &Arc<Grant>) -> Passed {
        Passed {
            versions: HashMap::new(),
            held: grant.arena(),
        }
    }

    fn hold(&mut self, v: &Version) -> Result<(), Error> {
        self.held.grow(2 * size_of::<u64>() + row_size(&v.row))?;
        self.versions
            .entry((v.header.xmin, v.header.xmax))
            .or_default()
            .insert(v.row.clone());
        Ok(())
    }

    /// Returns whether a version with `v`'s inserter, deleter and row was passed.
    fn holds(&self, v: &Version) -> bool {
        self.versions
            .get(&(v.header.xmin, v.header.xmax))
            .is_some_and(|rows| rows.contains(v.row.as_slice()))
    }
}

/// Returns an estimate of the bytes `row` occupies.
fn row_size(row: &[Value]) -> usize {
    row.iter().map(value_size).sum()
}

fn value_size(v: &Value) -> usize {
    size_of::<Value>()
        + match v {
            Value::Text(t) | Value::ZonedDateTime { zone: t, .. } => t.len(),
            Value::Bytes(b) | Value::SortKey(b) => b.len(),
            Value::Record(fs) | Value::Sum { fields: fs, .. } => row_size(fs),
            _ => 0,
        }
}

fn index_error(e: btree::Error) -> Error {
    match e {
        btree::Error::Page(p) => Error::Page(p),
        other => Error::Corrupt(format!("{other:?}")),
    }
}

impl<R: Runtime> Store<R> {
    fn heap(&self, id: HeapId) -> &StoredHeap {
        self.heaps
            .get(&id)
            .unwrap_or_else(|| panic!("the store holds heap {id:?}"))
    }

    fn index(&self, id: IndexId) -> &StoredIndex {
        self.indexes
            .get(&id)
            .unwrap_or_else(|| panic!("the store holds index {id:?}"))
    }

    /// Pass the visible rows of heap `id` to `each` as batches of at most `rows` rows each, as
    /// [`Store::scan_pages`] does, and then those of its history heap when the store reads the
    /// past.
    pub fn scan(
        &self,
        id: HeapId,
        rows: usize,
        grant: &Arc<Grant>,
        each: impl FnMut(Batch) -> Result<ControlFlow<()>, Error>,
    ) -> Result<(), Error> {
        let listing = self.pages(id)?;
        if let Some(past) = self.history.heaps.get(&id) {
            return self.scan_past(id, &listing.pages, past, rows, grant, each);
        }
        if let ControlFlow::Break(n) = self.scan_pages(id, &listing.pages, rows, grant, each)? {
            let read = listing.pages[..n].to_vec();
            listing.stopped(&read);
        }
        Ok(())
    }

    /// Pass the visible rows on `pages` of heap `id` and then on every page of `past`, its
    /// history heap, to `each` as [`Store::scan_pages`] does, each version once though vacuum has
    /// copied it to `past` and not yet freed it. Hold every deleted version passed from `pages`
    /// in `grant` until the scan ends.
    fn scan_past(
        &self,
        id: HeapId,
        pages: &[Pid],
        past: &StoredHeap,
        rows: usize,
        grant: &Arc<Grant>,
        mut each: impl FnMut(Batch) -> Result<ControlFlow<()>, Error>,
    ) -> Result<(), Error> {
        let h = self.heap(id);
        let mut b = Filling::new(&h.row, rows, grant);
        let mut deleted = Passed::new(grant);
        for now in [true, false] {
            let history;
            let (heap, pages) = if now {
                (&h.heap, pages)
            } else {
                history = past.heap.pages(&self.pager)?;
                (&past.heap, history.as_slice())
            };
            for &pid in pages {
                let mut versions = Vec::new();
                heap.scan_pages(&self.pager, &[pid], |v| versions.push(v))?;
                for v in versions {
                    if !self.reads.sees(&v.header)? {
                        continue;
                    }
                    if Xid(v.header.xmax) != Xid::NONE {
                        if now {
                            deleted.hold(&v)?;
                        } else if deleted.holds(&v) {
                            continue;
                        }
                    }
                    if let Some((full, _held)) = b.push(&v.row)?
                        && each(full)?.is_break()
                    {
                        return Ok(());
                    }
                }
            }
        }
        match b.rest() {
            Some((last, _held)) => each(last).map(|_last| ()),
            None => Ok(()),
        }
    }

    /// Returns the version the history key index of key index `index` names for `key` and
    /// the store's snapshot sees, from the history heap of the heap its route in `routes`, by
    /// constructor, names; `None` when there is none or the store does not read the past.
    /// While the key index is quarantined, scan those history heaps within `bounds`.
    pub fn fetch_past(
        &self,
        index: IndexId,
        key: &[Value],
        routes: &[(u32, HeapId)],
        bounds: ProbeBounds<'_>,
    ) -> Result<Option<(HeapId, Vec<Value>)>, Error> {
        let Some(past) = self.history.indexes.get(&index) else {
            return Ok(None);
        };
        let ix = self.index(index);
        if let Some(rows) = &ix.rows
            && self.pager.any_quarantined()
            && self
                .pager
                .is_quarantined(Quarantine::Index(ix.index.tree().owner()))
        {
            return self.past_from_heaps(index, rows, key, routes, bounds);
        }
        let ctors = &ix.ctors;
        for e in past.probe(&self.pager, key).map_err(index_error)? {
            let Some(c) = ctors.get(&e.ctor) else {
                continue;
            };
            let Some(&(_, heap)) = routes.iter().find(|(r, _)| r == c) else {
                continue;
            };
            let Some(stored) = self.history.heaps.get(&heap) else {
                continue;
            };
            if let Some(v) = stored.heap.read(&self.pager, e.tid)?
                && self.reads.sees(&v.header)?
            {
                return Ok(Some((heap, v.row)));
            }
        }
        Ok(None)
    }

    /// Returns what [`Store::fetch_past`] returns, found in the history heaps of `routes`.
    fn past_from_heaps(
        &self,
        index: IndexId,
        rows: &IndexedRows,
        key: &[Value],
        routes: &[(u32, HeapId)],
        bounds: ProbeBounds<'_>,
    ) -> Result<Option<(HeapId, Vec<Value>)>, Error> {
        let schema = self.index(index).index.schema();
        let wanted = schema.encode(key);
        let types = self.key_types(index);
        for &(c, heap) in routes {
            let Some(stored) = self.history.heaps.get(&heap) else {
                continue;
            };
            for pid in stored.heap.pages(&self.pager)? {
                (bounds.check)()?;
                let mut versions = Vec::new();
                stored
                    .heap
                    .scan_pages(&self.pager, &[pid], |v| versions.push(v))?;
                let size: usize = versions.iter().map(|v| row_size(&v.row)).sum();
                let _page = bounds.grant.take(size)?;
                for v in versions {
                    if rows
                        .keys
                        .key(c, &v.row, &types)?
                        .is_some_and(|k| schema.encode(&k) == wanted)
                        && self.reads.sees(&v.header)?
                    {
                        return Ok(Some((heap, v.row)));
                    }
                }
            }
        }
        Ok(None)
    }

    /// Returns the pages of heap `id` in directory order, recording that a scan of the heap
    /// began.
    pub fn pages(&self, id: HeapId) -> Result<Listing, Error> {
        let heap = &self.heap(id).heap;
        let scan = self
            .reads
            .scanning(heap.id())
            .map_err(Error::Serialization)?;
        let mut listing = Listing {
            reads: self.reads.clone(),
            heap: heap.id(),
            scan,
            pages: Vec::new(),
        };
        listing.pages = heap.pages(&self.pager)?;
        Ok(listing)
    }

    /// Pass the visible rows on `pages`, a range of what [`Store::pages`] returned for heap `id`,
    /// to `each` as batches of at most `rows` rows each, reading a page only once the batches
    /// before it are passed, each batch held in `grant` from its first row until `each` returns.
    /// Stop when `each` breaks. Returns the number of pages read when `each` breaks.
    ///
    /// Postconditions: the writers recorded with [`Reads::depends`] are those of the versions read
    /// before the last batch passed, and of every version on `pages` when `each` never breaks.
    pub fn scan_pages(
        &self,
        id: HeapId,
        pages: &[Pid],
        rows: usize,
        grant: &Arc<Grant>,
        mut each: impl FnMut(Batch) -> Result<ControlFlow<()>, Error>,
    ) -> Result<ControlFlow<usize>, Error> {
        let h = self.heap(id);
        let mut b = Filling::new(&h.row, rows, grant);
        let mut writers = BTreeSet::new();
        for (read, pid) in (1..).zip(pages) {
            let mut versions = Vec::new();
            h.heap
                .scan_pages(&self.pager, std::slice::from_ref(pid), |v| versions.push(v))?;
            for v in versions {
                let seen = self.reads.sees(&v.header)?;
                writers.extend(writer_of(&v.header, seen));
                if !seen {
                    continue;
                }
                if let Some((full, _held)) = b.push(&v.row)? {
                    self.depend_all(std::mem::take(&mut writers))?;
                    if each(full)?.is_break() {
                        return Ok(ControlFlow::Break(read));
                    }
                }
            }
        }
        self.depend_all(writers)?;
        let flow = match b.rest() {
            Some((last, _held)) => each(last)?,
            None => ControlFlow::Continue(()),
        };
        Ok(flow.map_break(|()| pages.len()))
    }

    fn depend_all(&self, writers: BTreeSet<Xid>) -> Result<(), Error> {
        writers.into_iter().try_for_each(|w| self.reads.depends(w))
    }

    /// Returns the version of heap `id` visible from the chain starting at `tid`, with its TID.
    pub fn fetch(&self, id: HeapId, tid: Tid) -> Result<Option<(Tid, Vec<Value>)>, Error> {
        let StoredHeap { heap, sum, .. } = self.heap(id);
        let mut failed = None;
        let mut writers = BTreeSet::new();
        let link = walk(&self.pager, heap.id(), sum, tid, |l| {
            match self.reads.sees(&l.header) {
                Ok(seen) => {
                    writers.extend(writer_of(&l.header, seen));
                    seen
                }
                Err(e) => {
                    failed = Some(e);
                    true
                }
            }
        })?;
        if let Some(e) = failed {
            return Err(Error::Page(e));
        }
        self.depend_all(writers)?;
        let Some(link) = link.filter(|l| l.heap == heap.id()) else {
            return Ok(None);
        };
        self.reads
            .fetched(link.heap, link.tid)
            .map_err(Error::Serialization)?;
        let v = heap
            .read(&self.pager, link.tid)?
            .expect("a walked version is there");
        Ok(Some((link.tid, v.row)))
    }

    /// Returns the entries of index `id` whose leading key fields are `prefix`, each with its
    /// constructor position, for the constructors of `routes`.
    pub fn probe(
        &self,
        id: IndexId,
        prefix: &[Value],
        routes: &BTreeSet<u32>,
        bounds: ProbeBounds<'_>,
    ) -> Result<Vec<(u32, Tid)>, Error> {
        let bytes = self.encode_prefix(id, prefix);
        let end = prefix_end(&bytes);
        self.entries(id, Bound::Included(bytes), end, routes, bounds)
    }

    /// Returns the entries of index `id` whose leading key fields are `prefix` and whose next
    /// field lies within `lo` and `hi` in the language's order, each bound its value and whether
    /// it is included, as [`Store::probe`] does.
    ///
    /// Preconditions: the index has a field after `prefix`, and the field's type is ordered.
    pub fn range(
        &self,
        id: IndexId,
        prefix: &[Value],
        lo: Option<(Value, bool)>,
        hi: Option<(Value, bool)>,
        routes: &BTreeSet<u32>,
        bounds: ProbeBounds<'_>,
    ) -> Result<Vec<(u32, Tid)>, Error> {
        let base = self.encode_prefix(id, prefix);
        let next = self.index(id).index.schema().components()[prefix.len()].clone();
        let dir = next.1;
        let field = KeySchema::new(vec![next]);
        let at = |v: &Value| {
            let mut b = base.clone();
            field.encode_into(std::slice::from_ref(v), &mut b);
            b
        };
        let (lo, hi) = match dir {
            Dir::Asc => (lo, hi),
            Dir::Desc => (hi, lo),
        };
        let start = match lo {
            None => Bound::Included(base.clone()),
            Some((v, true)) => Bound::Included(at(&v)),
            Some((v, false)) => match prefix_end(&at(&v)) {
                Bound::Excluded(end) => Bound::Included(end),
                _ => return Ok(Vec::new()),
            },
        };
        let end = match hi {
            None => prefix_end(&base),
            Some((v, true)) => prefix_end(&at(&v)),
            Some((v, false)) => Bound::Excluded(at(&v)),
        };
        self.entries(id, start, end, routes, bounds)
    }

    fn encode_prefix(&self, id: IndexId, prefix: &[Value]) -> Vec<u8> {
        let components = self.index(id).index.schema().components();
        KeySchema::new(components[..prefix.len()].to_vec()).encode(prefix)
    }

    /// Returns the entries of index `id` whose encodings lie between `lo` and `hi`, each with
    /// its constructor position, for the constructors of `routes`.
    fn entries(
        &self,
        id: IndexId,
        lo: Bound<Vec<u8>>,
        hi: Bound<Vec<u8>>,
        routes: &BTreeSet<u32>,
        bounds: ProbeBounds<'_>,
    ) -> Result<Vec<(u32, Tid)>, Error> {
        let ix = self.index(id);
        if let Some(rows) = &ix.rows
            && self.pager.any_quarantined()
            && self
                .pager
                .is_quarantined(Quarantine::Index(ix.index.tree().owner()))
        {
            return self.entries_from_heaps(id, rows, (&lo, &hi), routes, bounds);
        }
        let ctors: BTreeSet<CtorId> = ix
            .ctors
            .iter()
            .filter(|(_, c)| routes.contains(c))
            .map(|(id, _)| *id)
            .collect();
        self.reads
            .probed(ix.index.tree().root(), lo.clone(), hi.clone(), ctors)
            .map_err(Error::Serialization)?;
        let entries = ix
            .index
            .scan(
                &self.pager,
                lo.as_ref().map(Vec::as_slice),
                hi.as_ref().map(Vec::as_slice),
            )
            .map_err(index_error)?;
        Ok(entries
            .into_iter()
            .filter_map(|e| {
                let c = *ix
                    .ctors
                    .get(&e.ctor)
                    .unwrap_or_else(|| panic!("index {id:?} maps constructor {:?}", e.ctor));
                routes.contains(&c).then_some((c, e.tid))
            })
            .collect())
    }

    /// Returns the entries [`Store::entries`] returns, computed from the visible rows of the
    /// heaps of `routes`, in index order. Each entry names the version the store sees. Hold each
    /// page's versions and the entries found in the grant of `bounds`, and check `bounds` at
    /// each page.
    fn entries_from_heaps(
        &self,
        id: IndexId,
        rows: &IndexedRows,
        range: (&Bound<Vec<u8>>, &Bound<Vec<u8>>),
        routes: &BTreeSet<u32>,
        bounds: ProbeBounds<'_>,
    ) -> Result<Vec<(u32, Tid)>, Error> {
        let schema = self.index(id).index.schema();
        let types = self.key_types(id);
        let (lo, hi) = range;
        let mut found = Vec::new();
        let mut held = bounds.grant.arena();
        for (&c, &heap) in rows.heaps.iter().filter(|(c, _)| routes.contains(c)) {
            let listing = self.pages(heap)?;
            let mut writers = BTreeSet::new();
            for &pid in &listing.pages {
                (bounds.check)()?;
                let mut versions = Vec::new();
                self.heap(heap)
                    .heap
                    .scan_pages(&self.pager, &[pid], |v| versions.push(v))?;
                let size: usize = versions.iter().map(|v| row_size(&v.row)).sum();
                let _page = bounds.grant.take(size)?;
                for v in versions {
                    let seen = self.reads.sees(&v.header)?;
                    writers.extend(writer_of(&v.header, seen));
                    if seen && let Some(key) = rows.keys.key(c, &v.row, &types)? {
                        let bytes = schema.encode(&key);
                        let span = (
                            lo.as_ref().map(Vec::as_slice),
                            hi.as_ref().map(Vec::as_slice),
                        );
                        if span.contains(bytes.as_slice()) {
                            held.grow(bytes.len() + size_of::<(u32, Tid)>())?;
                            found.push((bytes, c, v.tid));
                        }
                    }
                }
            }
            self.depend_all(writers)?;
        }
        found.sort();
        Ok(found.into_iter().map(|(_, c, t)| (c, t)).collect())
    }

    /// Returns the key types of index `id`'s fields.
    pub fn key_types(&self, id: IndexId) -> Vec<KeyType> {
        self.index(id)
            .index
            .schema()
            .components()
            .iter()
            .map(|(t, _)| t.clone())
            .collect()
    }
}

/// How a stored value of one kernel type becomes a [`Val`].
#[derive(Clone, Debug)]
pub enum Conv {
    Scalar,
    /// A declared sum or record, by constructor.
    Decl(Vec<Vec<Conv>>),
    AnonRecord(Vec<Conv>),
}

/// Returns how a stored value of type `ty` converts.
pub fn conv(schema: &Schema, ty: &Ty) -> Conv {
    let t = repr(schema, ty);
    match &t {
        Ty::Base(_) => Conv::Scalar,
        Ty::Record(fs) => Conv::AnonRecord(fs.iter().map(|(_, f)| conv(schema, f)).collect()),
        Ty::Key { rel, .. } => {
            let r = schema.relation(*rel);
            let row = ctors(schema, &r.elem).expect("a relation's element type is a sum");
            let field_ty = |n: &str| {
                row[0]
                    .iter()
                    .find(|(f, _)| f == n)
                    .unwrap_or_else(|| panic!("a row of `{}` has key field `{n}`", r.name))
                    .1
                    .clone()
            };
            match r.key.as_slice() {
                [k] => match field_ty(k) {
                    Ty::Key { rel: own, .. } if own == *rel => Conv::Scalar,
                    f => conv(schema, &f),
                },
                ks => {
                    let mut names = ks.to_vec();
                    names.sort();
                    Conv::AnonRecord(names.iter().map(|n| conv(schema, &field_ty(n))).collect())
                }
            }
        }
        _ => match ctors(schema, &t) {
            Some(cs) => Conv::Decl(
                cs.iter()
                    .map(|fs| fs.iter().map(|(_, f)| conv(schema, f)).collect())
                    .collect(),
            ),
            None => panic!("a stored value has no type {t:?}"),
        },
    }
}

/// Returns stored value `v` of a type converting by `c`.
pub fn from_stored(c: &Conv, v: &Value) -> Val {
    let all = |cs: &[Conv], fs: &[Value]| -> Vec<Val> {
        cs.iter().zip(fs).map(|(c, v)| from_stored(c, v)).collect()
    };
    match (c, v) {
        (Conv::Decl(cs), Value::Record(fs)) => Val::ctor(0, all(&cs[0], fs)),
        (Conv::Decl(cs), Value::Sum { ctor, fields }) => {
            Val::ctor(*ctor, all(&cs[*ctor as usize], fields))
        }
        (Conv::AnonRecord(cs), Value::Record(fs)) => Val::record(all(cs, fs)),
        (_, v) => scalar(v),
    }
}

fn scalar(v: &Value) -> Val {
    match v {
        Value::I8(x) => Val::Int(i64::from(*x)),
        Value::I16(x) => Val::Int(i64::from(*x)),
        Value::I32(x) => Val::Int(i64::from(*x)),
        Value::I64(x) => Val::Int(*x),
        Value::F32(x) => Val::F32(F32::new(x.get())),
        Value::F64(x) => Val::F64(F64::new(x.get())),
        Value::Decimal(d) => Val::Decimal(*d),
        Value::Text(t) => Val::text(t),
        Value::Bytes(b) => Val::Bytes(Arc::from(b.as_slice())),
        Value::SortKey(b) => Val::SortKey(Arc::from(b.as_slice())),
        Value::Instant(x) => Val::Instant(*x),
        Value::Date(x) => Val::Date(*x),
        Value::Time(x) => Val::Time(*x),
        Value::DateTime(x) => Val::DateTime(*x),
        Value::ZonedDateTime { local, zone } => Val::Zoned {
            local: *local,
            zone: Arc::from(zone.as_str()),
        },
        Value::Duration(x) => Val::Duration(*x),
        Value::Period { months, days } => Val::Period {
            months: *months,
            days: *days,
        },
        other => panic!("a scalar conversion of {other:?}"),
    }
}

/// Returns `v` as a heap stores a field of type `t`.
pub fn to_stored(t: &FieldType, v: &Val) -> Result<Value, Error> {
    Ok(match (t, v) {
        (FieldType::Scalar(s), v) => scalar_key(*s, v)?,
        (FieldType::Record(ts), Val::Ctor(_, fs) | Val::Record(fs)) => Value::Record(
            ts.iter()
                .zip(fs.iter())
                .map(|(t, v)| to_stored(t, v))
                .collect::<Result<_, _>>()?,
        ),
        (FieldType::Sum(cs), Val::Ctor(c, fs)) => Value::Sum {
            ctor: *c,
            fields: cs
                .get(*c as usize)
                .ok_or_else(|| not_stored(v, t))?
                .fields
                .iter()
                .zip(fs.iter())
                .map(|(t, v)| to_stored(t, v))
                .collect::<Result<_, _>>()?,
        },
        (t, v) => return Err(not_stored(v, t)),
    })
}

/// Returns `v` as a value of key type `t`.
pub fn to_key(t: &KeyType, v: &Val) -> Result<Value, Error> {
    Ok(match (t, v) {
        (KeyType::Scalar(s), v) => scalar_key(*s, v)?,
        (KeyType::Record(ts), Val::Ctor(_, fs) | Val::Record(fs)) => Value::Record(
            ts.iter()
                .zip(fs.iter())
                .map(|(t, v)| to_key(t, v))
                .collect::<Result<_, _>>()?,
        ),
        (KeyType::AnonRecord(r), Val::Record(fs)) => Value::Record(
            r.fields()
                .iter()
                .zip(fs.iter())
                .map(|((_, t), v)| to_key(t, v))
                .collect::<Result<_, _>>()?,
        ),
        (KeyType::Sum(s), Val::Ctor(c, fs)) => Value::Sum {
            ctor: *c,
            fields: s
                .ctors()
                .get(*c as usize)
                .ok_or_else(|| not_stored(v, t))?
                .fields
                .iter()
                .zip(fs.iter())
                .map(|(t, v)| to_key(t, v))
                .collect::<Result<_, _>>()?,
        },
        (KeyType::AnonSum(s), Val::Ctor(c, fs)) => Value::Sum {
            ctor: *c,
            fields: s
                .ctors()
                .get(*c as usize)
                .ok_or_else(|| not_stored(v, t))?
                .1
                .fields()
                .iter()
                .zip(fs.iter())
                .map(|((_, t), v)| to_key(t, v))
                .collect::<Result<_, _>>()?,
        },
        (t, v) => return Err(not_stored(v, t)),
    })
}

fn scalar_key(s: Scalar, v: &Val) -> Result<Value, Error> {
    Ok(match (s, v) {
        (Scalar::I8, Val::Int(x)) => Value::I8(*x as i8),
        (Scalar::I16, Val::Int(x)) => Value::I16(*x as i16),
        (Scalar::I32, Val::Int(x)) => Value::I32(*x as i32),
        (Scalar::I64, Val::Int(x)) => Value::I64(*x),
        (Scalar::F32, Val::F32(x)) => Value::F32(crate::key::F32::new(x.get())),
        (Scalar::F64, Val::F64(x)) => Value::F64(crate::key::F64::new(x.get())),
        (Scalar::Decimal(_), Val::Decimal(d)) => Value::Decimal(*d),
        (Scalar::Text, Val::Text(t)) => Value::Text(t.to_string()),
        (Scalar::Bytes, Val::Bytes(b)) => Value::Bytes(b.to_vec()),
        (Scalar::SortKey, Val::SortKey(b)) => Value::SortKey(b.to_vec()),
        (Scalar::Instant, Val::Instant(x)) => Value::Instant(*x),
        (Scalar::Date, Val::Date(x)) => Value::Date(*x),
        (Scalar::Time, Val::Time(x)) => Value::Time(*x),
        (Scalar::DateTime, Val::DateTime(x)) => Value::DateTime(*x),
        (Scalar::ZonedDateTime, Val::Zoned { local, zone }) => Value::ZonedDateTime {
            local: *local,
            zone: zone.to_string(),
        },
        (Scalar::Duration, Val::Duration(x)) => Value::Duration(*x),
        (Scalar::Period, Val::Period { months, days }) => Value::Period {
            months: *months,
            days: *days,
        },
        (s, v) => return Err(not_stored(v, s)),
    })
}

fn not_stored(v: &Val, t: impl std::fmt::Debug) -> Error {
    Error::Internal(format!("{v:?} is not a value of {t:?}"))
}
