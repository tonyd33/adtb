use std::collections::BTreeSet;
use std::ops::Bound;
use std::sync::Arc;

use crate::heap::{Heap, HeapNo, Refused, Slots};
use crate::key::{CtorId, Tid};
use crate::mvcc::{Conflict, Logged, Reader, ScanId, SerialId, Serializer, StatusError, Writers};
use crate::runtime::Runtime;
use crate::steps::{Step, Steps};
use crate::storage::{Pager, Pid};
use adtb_sys::lock::Ranked;

use super::Error;
use crate::mvcc::StopsEarly;

/// What a query's reads see and record: the visibility of versions at its snapshot, and the
/// SIREAD locks its transaction takes.
pub trait Reads: Send + Sync {
    /// Returns whether the version on page `at` of heap `heap` written by `writers` is visible
    /// at the reader's snapshot. Record that what the reader saw depends on the writer the read
    /// is before: the inserter of a version it missed or the deleter of one it saw, when its
    /// snapshot does not count that writer as committed and that writer did not abort. Refuse
    /// page `at` when a writer it looks up is not reserved. Returns `None` when no slot of the
    /// page names that writer under its exclusive latch: the caller reads the page again, as
    /// often as [`Heap::refuse_unreserved`] bounds.
    fn saw(&self, at: (Pid, HeapNo), writers: &Writers) -> Result<Option<bool>, Error>;

    /// Record that a scan of heap `heap` began, covering every version of the heap until
    /// [`Reads::scanned`] ends it. Returns the scan, `None` when it records none.
    fn scanning(&self, heap: HeapNo) -> Result<Option<ScanId>, Conflict>;

    /// Record that `scan`, a scan of heap `heap`, ended having read every version of the heap,
    /// or, when it stopped early, only those on the pages it read.
    fn scanned(&self, heap: HeapNo, scan: ScanId, read: Option<(StopsEarly, &[Pid])>);

    /// Record that the encoded keys of the index rooted at `index` between `lo` and `hi` were
    /// read, for rows of `ctors`.
    fn probed(
        &self,
        index: Pid,
        lo: Bound<Vec<u8>>,
        hi: Bound<Vec<u8>>,
        ctors: BTreeSet<CtorId>,
    ) -> Result<(), Conflict>;

    /// Record that the version at `tid` of heap `heap` was read.
    fn fetched(&self, heap: HeapNo, tid: Tid) -> Result<(), Conflict>;

    /// Returns whether a read of heap `heap` sees what the reader's snapshot sees.
    fn covers(&self, heap: HeapNo) -> bool;

    /// Call the step hook of a serializable reader with `step`.
    fn reached(&self, _: Step) {}
}

/// The reads at a snapshot of the transactions layer, recording SIREAD locks with `serial` when it
/// is given.
pub(super) struct AtSnapshot<R: Runtime> {
    rt: R,
    reader: Reader<Logged<R>>,
    serial: Option<(Arc<Ranked<Serializer>>, SerialId)>,
    steps: Arc<Steps>,
}

impl<R: Runtime> AtSnapshot<R> {
    pub(super) fn new(
        reader: Reader<Logged<R>>,
        serial: Option<(Arc<Ranked<Serializer>>, SerialId)>,
        steps: Arc<Steps>,
    ) -> AtSnapshot<R> {
        AtSnapshot {
            rt: reader.pager().rt().clone(),
            reader,
            serial,
            steps,
        }
    }
}

impl<R: Runtime> std::fmt::Debug for AtSnapshot<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AtSnapshot")
            .field("reader", &self.reader)
            .finish_non_exhaustive()
    }
}

/// Returns the failure of a status lookup for a writer of a version on page `at` of heap `heap`,
/// `None` when the page no longer names it.
fn unreserved<R: Runtime>(
    pager: &Pager<R>,
    (at, heap): (Pid, HeapNo),
    e: StatusError,
) -> Result<Option<bool>, Error> {
    match Heap::refuse_unreserved(pager, (at, Slots::Versions(heap.0)), (), e) {
        Refused::Page(e) => Err(e.into()),
        Refused::Gone => Ok(None),
    }
}

impl<R: Runtime> Reads for AtSnapshot<R> {
    fn saw(&self, at: (Pid, HeapNo), writers: &Writers) -> Result<Option<bool>, Error> {
        let seen = match self.reader.sees(writers) {
            Ok(seen) => seen,
            Err(e) => return unreserved(self.reader.pager(), at, e),
        };
        if let Some((s, id)) = &self.serial {
            let writer = match self.reader.before(seen) {
                Ok(writer) => writer,
                Err(e) => return unreserved(self.reader.pager(), at, e),
            };
            if let Some(writer) = writer {
                s.lock(&self.rt)
                    .read_from(*id, writer, at.1.0)
                    .map_err(Error::Serialization)?;
            }
        }
        Ok(Some(seen.visible()))
    }

    fn scanning(&self, heap: HeapNo) -> Result<Option<ScanId>, Conflict> {
        match &self.serial {
            Some((s, id)) => s.lock(&self.rt).read_scan(*id, heap.0),
            None => Ok(None),
        }
    }

    fn scanned(&self, heap: HeapNo, scan: ScanId, read: Option<(StopsEarly, &[Pid])>) {
        if let Some((s, id)) = &self.serial {
            let read = read.map(|(early, r)| (early, r.iter().map(|p| p.0).collect::<Vec<_>>()));
            let read = read.as_ref().map(|(early, r)| (early, r.as_slice()));
            s.lock(&self.rt).end_scan(*id, heap.0, scan, read);
        }
    }

    fn probed(
        &self,
        index: Pid,
        lo: Bound<Vec<u8>>,
        hi: Bound<Vec<u8>>,
        ctors: BTreeSet<CtorId>,
    ) -> Result<(), Conflict> {
        match &self.serial {
            Some((s, id)) => s.lock(&self.rt).read_range(*id, index, lo, hi, ctors),
            None => Ok(()),
        }
    }

    fn fetched(&self, heap: HeapNo, tid: Tid) -> Result<(), Conflict> {
        match &self.serial {
            Some((s, id)) => s.lock(&self.rt).read_tuple(*id, heap.0, tid),
            None => Ok(()),
        }
    }

    fn covers(&self, heap: HeapNo) -> bool {
        self.reader.covers(heap.0)
    }

    fn reached(&self, step: Step) {
        if self.serial.is_some() {
            self.steps.at(step);
        }
    }
}

/// What a read of the past sees: the versions the snapshot of a pinned marker sees, taking no
/// SIREAD locks.
pub(super) struct AtMarker<R: Runtime> {
    reader: Reader<Logged<R>>,
}

impl<R: Runtime> AtMarker<R> {
    pub(super) fn new(reader: Reader<Logged<R>>) -> AtMarker<R> {
        AtMarker { reader }
    }
}

impl<R: Runtime> Reads for AtMarker<R> {
    fn saw(&self, at: (Pid, HeapNo), writers: &Writers) -> Result<Option<bool>, Error> {
        match self.reader.sees(writers) {
            Ok(seen) => Ok(Some(seen.visible())),
            Err(e) => unreserved(self.reader.pager(), at, e),
        }
    }

    fn scanning(&self, _: HeapNo) -> Result<Option<ScanId>, Conflict> {
        Ok(None)
    }

    fn scanned(&self, _: HeapNo, _: ScanId, _: Option<(StopsEarly, &[Pid])>) {}

    fn probed(
        &self,
        _: Pid,
        _: Bound<Vec<u8>>,
        _: Bound<Vec<u8>>,
        _: BTreeSet<CtorId>,
    ) -> Result<(), Conflict> {
        Ok(())
    }

    fn fetched(&self, _: HeapNo, _: Tid) -> Result<(), Conflict> {
        Ok(())
    }

    fn covers(&self, heap: HeapNo) -> bool {
        self.reader.covers(heap.0)
    }
}
