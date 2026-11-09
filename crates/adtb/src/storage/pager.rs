use std::collections::{BTreeMap, BTreeSet};
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use adtb_sys::pages::{self, PageMemory, PageState};

use crate::runtime::{AlignedBuf, FileId, Runtime, ThreadId};

use super::alloc::{Allocation, Hold, Reservation, diff};
use super::damage_log::Recorded;
use super::driver::Driver;
use super::error::{Damage, OpenError};
use super::evidence::{Against, Evidence, Judged, Met, Refusal, Unmapped, file_copy};
use super::header::{FileHeader, Header};
use super::holding::{Holding, NO_HOLDER};
use super::log::{Log, Pinned, Segments};
use super::page::{self, FIRST_EDITABLE, HEADER_LEN, Verdict};
use super::pagetable::{DirtyTable, Logged};
use super::quarantine::{Bitmapped, Marks, Quarantine};
use super::record::{Edit, MapRoot, Record};
use super::recovery::Replaying;
use super::{Lsn, Pid, Position};
use adtb_sys::lock::Lock;

#[derive(Clone, Debug)]
pub struct Options {
    /// The data file's path; the log's segments live in the directory [`log_dir`](Options::log_dir).
    pub name: String,
    pub page_size: u32,
    /// The device's logical block size, and the log's block size.
    pub log_block: u32,
    pub segment_size: u64,
    /// The most pages resident at once.
    pub pool_frames: usize,
    /// The page ids the pool reserves address space for; no page at or above it is reachable.
    pub max_pages: u64,
    /// The device writes a whole page atomically, so no full-page images are logged.
    pub atomic_page_writes: bool,
    /// The threads that may latch pages at once; the pool holds [`LATCHES_PER_THREAD`] frames
    /// for each.
    pub threads: usize,
    /// How long a page-in waits for a frame, with every frame latched, before it reports a
    /// stalled pool.
    pub stall_after: Duration,
    /// The position a new file's history starts after: the file begins an epoch a random step
    /// of 1 to 2^32 above it, that position ending its epoch. `None` begins epoch 1.
    pub after: Option<Position>,
    /// The log, in bytes, forced since the last checkpoint that makes the checkpointer write
    /// every changed page and take the next. The redo LSN of the checkpoint it takes is at or
    /// past the checkpoint before it, so recovery reads at most this much log plus what the
    /// last two cycles logged, unless a page refused on a file quarantined whole holds the redo
    /// LSN. Once the open's readers of the log have run, a cycle also recycles to the redo LSN of
    /// the checkpoint before the one it takes, unless a pin or the archive holds the log.
    pub checkpoint_every: u64,
}

/// The pages one thread latches at most at once: a B-link operation's four and the allocation
/// bitmap page.
pub const LATCHES_PER_THREAD: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageError {
    pub pid: Pid,
    pub damage: Damage,
}

/// What the pool knows of its pages beyond their memory and state words.
#[derive(Debug)]
struct Meta {
    table: DirtyTable,
    evidence: Evidence,
    /// Resident pages and pages being read in, in clock order.
    resident: Vec<Pid>,
    hand: usize,
}

/// The dirty page table, under the meta lock.
pub(super) struct Locked<'a>(Holding<'a, Meta>);

impl Deref for Locked<'_> {
    type Target = DirtyTable;

    fn deref(&self) -> &DirtyTable {
        &self.0.table
    }
}

impl DerefMut for Locked<'_> {
    fn deref_mut(&mut self) -> &mut DirtyTable {
        &mut self.0.table
    }
}

/// The evidence of every page, under the meta lock.
pub(super) struct Evidential<'a>(Holding<'a, Meta>);

impl Deref for Evidential<'_> {
    type Target = Evidence;

    fn deref(&self) -> &Evidence {
        &self.0.evidence
    }
}

impl DerefMut for Evidential<'_> {
    fn deref_mut(&mut self) -> &mut Evidence {
        &mut self.0.evidence
    }
}

/// The log lock's value: the log and the page LSN map root its records name, `None` while the
/// file has no map.
pub(super) struct Appended {
    log: Log,
    map: Option<MapRoot>,
}

/// The log lock, held without the meta lock.
pub(crate) struct LogGuard<'a>(Holding<'a, Appended>);

impl Deref for LogGuard<'_> {
    type Target = Log;

    fn deref(&self) -> &Log {
        &self.0.log
    }
}

impl DerefMut for LogGuard<'_> {
    fn deref_mut(&mut self) -> &mut Log {
        &mut self.0.log
    }
}

mod witness {
    use super::{Append, Appended, Holding, Meta, Pager, Runtime};

    /// The witness of an [`Append`]: the log and meta locks held together.
    pub(in crate::storage) struct Appending(());

    impl<'a, R: Runtime> Append<'a, R> {
        pub(super) fn new(
            pager: &'a Pager<R>,
            log: Holding<'a, Appended>,
            meta: Holding<'a, Meta>,
        ) -> Self {
            Append {
                pager,
                log,
                meta,
                witness: Appending(()),
            }
        }
    }

    #[cfg(test)]
    pub(in crate::storage) fn appending_for_test() -> Appending {
        Appending(())
    }
}

pub(super) use witness::Appending;
#[cfg(test)]
pub(super) use witness::appending_for_test;

/// The log and meta locks, held together: a record and its in-memory consequences are made in
/// one hold.
pub(super) struct Append<'a, R: Runtime> {
    pager: &'a Pager<R>,
    log: Holding<'a, Appended>,
    meta: Holding<'a, Meta>,
    witness: Appending,
}

/// A checkpoint's LSN, its redo LSN, the bitmap groups it captured, the damage records to fold
/// into the list page when it folds, and the pin of its redo LSN when it pins.
pub(super) type Checkpointed = (Lsn, Lsn, u64, Option<Recorded>, Option<Pinned>);

impl<R: Runtime> Append<'_, R> {
    /// Apply `edits` to the page under `g` and log its record: a full image starting a chain when
    /// `start`, else the record [`DirtyTable::record_kind`] chooses. Returns the record's LSN.
    fn change_page(&mut self, g: &mut PageExclusive<'_, R>, edits: Vec<Edit>, start: bool) -> Lsn {
        let pid = g.pid();
        let Appended { log, map } = &mut *self.log;
        let m = &mut *self.meta;
        let prev = page::lsn(g);
        let lsn = log.next_lsn();
        g.held_mut().apply_change(&edits, lsn);
        let record = if start {
            g.kind = Kind::Latched;
            Record::Image {
                pid,
                prev: Lsn::UNCHAINED,
                page: g.to_vec(),
            }
        } else if m
            .table
            .record_kind(&self.witness, pid, log.last_checkpoint())
            == Logged::Change
        {
            Record::Change { pid, prev, edits }
        } else {
            Record::Image {
                pid,
                prev,
                page: g.to_vec(),
            }
        };
        let (kind, met) = match record {
            Record::Image { prev, .. } if prev == Lsn::UNCHAINED => {
                (Logged::Image, Some(Met::Start))
            }
            Record::Image { .. } => (Logged::Image, Some(Met::Image)),
            _ => (Logged::Change, None),
        };
        log.append(&record);
        m.table.logged(&self.witness, pid, lsn, kind);
        m.evidence.logged_page(&self.witness, pid, lsn, met);
        if let Some(map) = map.as_mut()
            && map.root == pid
        {
            map.lsn = lsn;
        }
        lsn
    }

    /// Drop the damage recorded of each page of `pids`, which the batch frees, logging each free
    /// unless the file is quarantined whole. Returns the LSN of the last free logged.
    fn free(&mut self, pids: &[Pid]) -> Option<Lsn> {
        let whole = self.pager.is_quarantined(Quarantine::Database);
        let mut last = None;
        for &pid in pids {
            let log = &mut self.log.log;
            let freed = self.meta.evidence.free(&self.witness, log, pid, whole);
            if let Some(lsn) = freed {
                self.meta.table.freeing(&self.witness, pid, lsn);
                last = Some(lsn);
            }
        }
        last
    }

    /// Make the page `root`, at `lsn`, the map's root of `height` levels and log the record naming
    /// it. Returns the record's LSN.
    fn map_root(&mut self, root: Pid, lsn: Lsn, height: u32) -> Lsn {
        self.log.map = Some(MapRoot { root, lsn, height });
        self.log.log.append(&Record::MapRoot { root, height })
    }

    /// Log `record`, which changes no page. Returns its LSN.
    fn record(&mut self, record: &Record) -> Lsn {
        self.log.log.append(record)
    }

    /// Refuse the page under `latch` as `damage` from now on, its resident copy dropped, and
    /// record it, unless it is refused as `damage` already. Returns the end of the damage record
    /// logged, `None` when none is.
    pub(super) fn refuse(&mut self, latch: &Held<'_, R>, damage: Damage) -> Option<Lsn> {
        let refusal = Refusal::of(latch);
        let whole = self.pager.is_quarantined(Quarantine::Database);
        let by = self.pager.rt.current();
        let m = &mut *self.meta;
        let log = &mut self.log.log;
        let logged = m
            .evidence
            .refuse(&self.witness, log, &refusal, damage, whole, by);
        m.table.refused(&refusal);
        logged
    }

    /// Record `damage` of `pid`, logging it unless recovery has not ended or the file is
    /// quarantined whole. Returns the log's end after the record.
    pub(super) fn record_damage(&mut self, pid: Pid, damage: Damage) -> Option<Lsn> {
        let whole = self.pager.is_quarantined(Quarantine::Database);
        let log = &mut self.log.log;
        self.meta
            .evidence
            .record(&self.witness, log, pid, damage, whole)
    }

    /// Record every page refused so far, and log the records from now on.
    pub(super) fn start_recording(&mut self) {
        let whole = self.pager.is_quarantined(Quarantine::Database);
        let log = &mut self.log.log;
        self.meta
            .evidence
            .start_recording(&self.witness, log, whole);
    }

    /// Returns whether a checkpoint is to fold the damage records into the list page.
    pub(super) fn folds(&self) -> bool {
        self.meta.evidence.to_fold().is_some()
    }

    /// Log a checkpoint of the dirty page table and the map root, pinning its redo LSN when `pin`,
    /// and take the damage records to fold when `fold`.
    pub(super) fn checkpoint(&mut self, fold: bool, pin: bool) -> Checkpointed {
        let Appended { log, map } = &mut *self.log;
        let m = &mut *self.meta;
        let lsn = log.next_lsn();
        let dirty = m.table.snapshot();
        let redo = dirty.iter().map(|(_, l)| *l).min().unwrap_or(lsn);
        let folded = fold.then(|| m.evidence.snapshot(self.pager.damage_list_capacity()));
        let at = log.append(&Record::Checkpoint {
            redo,
            dirty,
            map: *map,
        });
        let pinned = pin.then(|| log.pin(redo));
        let groups = self.pager.allocation().started();
        (at, redo, groups, folded, pinned)
    }
}

/// The value of the lock a force of the data file holds.
#[derive(Debug)]
pub(super) struct Forcing(());

/// A write of a page to the data file.
pub(super) struct Wrote(Pid);

impl Wrote {
    pub(super) fn pid(&self) -> Pid {
        self.0
    }
}

#[cfg(test)]
pub(super) fn forcing_for_test() -> Forcing {
    Forcing(())
}

#[cfg(test)]
pub(super) fn wrote_for_test(pid: Pid) -> Wrote {
    Wrote(pid)
}

/// The data file, its buffer pool and its redo log, shared by every thread of the engine.
///
/// Invariants:
///
/// - A page is written only under a latch, and only once the log is durable past its LSN.
/// - A page is in the dirty page table from the change that dirties its clean page, which logs a
///   full-page image, until a data file force covers a write of it with no change since, or, for
///   a page refused holding changes the file lacks, until a free of it is durable or a chain
///   start of it is logged and then written.
/// - A record and every in-memory consequence of it (the page's dirty entry, its expected
///   LSN, its damage record, the LSN map root) are made in one hold of the log, so a
///   checkpoint, which takes its snapshot in the same hold, sees all of them or none.
///
/// Clones are handles on one pager.
pub struct Pager<R: Runtime>(std::sync::Arc<Shared<R>>);

impl<R: Runtime> Pager<R> {
    /// Returns a handle on this pager that does not keep it open.
    pub(super) fn downgrade(&self) -> std::sync::Weak<Shared<R>> {
        std::sync::Arc::downgrade(&self.0)
    }

    /// Returns a handle on the pager `weak` names, `None` once every other handle has dropped.
    pub(super) fn upgrade(weak: &std::sync::Weak<Shared<R>>) -> Option<Pager<R>> {
        weak.upgrade().map(Pager)
    }
}

impl<R: Runtime> Clone for Pager<R> {
    fn clone(&self) -> Self {
        Pager(self.0.clone())
    }
}

impl<R: Runtime> Deref for Pager<R> {
    type Target = Shared<R>;

    fn deref(&self) -> &Shared<R> {
        &self.0
    }
}

/// The state every handle on one pager shares.
///
/// Lock order, as the code takes them: the recycle lock; page latches, with the allocation lock
/// taken after a data page's latch and before a bitmap page's; the segments lock; the log lock;
/// the meta lock; the marks lock. Only an [`Append`] holds the log and meta locks together. The
/// force lock is taken with no latch held and before the meta lock, the archive and header locks
/// under the recycle lock alone. The marks, driver and checkpointer locks are leaves. A latch
/// tried under the meta lock waits for nothing.
///
/// The accessors of the log and meta locks check their order at runtime: a thread that holds
/// either lock and takes either panics, except for the meta lock an [`Append`] takes under its
/// log lock.
pub struct Shared<R: Runtime> {
    rt: R,
    name: String,
    page_size: usize,
    pool_frames: usize,
    stall_after: Duration,
    data: FileId,
    mem: PageMemory,
    drivers: Lock<BTreeMap<ThreadId, Driver<R>>>,
    /// Held by a force of the data file from taking the pages written to dropping pages from the
    /// dirty page table.
    forcing: Lock<Forcing>,
    header: Header,
    log: Lock<Appended>,
    /// The thread holding the log lock, `NO_HOLDER` while none does.
    log_holder: AtomicU64,
    /// The thread holding the meta lock, `NO_HOLDER` while none does.
    meta_holder: AtomicU64,
    /// Held by the one thread writing and forcing a batch: the group commit leader.
    segments: Lock<Segments>,
    meta: Lock<Meta>,
    alloc: Allocation,
    marks: Marks,
    unmapped: Unmapped,
    /// Set while an [`super::Opening`] of this pager lives.
    opening: std::sync::Arc<std::sync::atomic::AtomicBool>,
    archive: Lock<Option<ArchiveHook>>,
    /// Held across a recycle of the log, taken before any other lock or latch.
    recycling: Lock<()>,
    checkpointer: Lock<super::checkpoint::Worker>,
}

/// Returns the LSN below which the log's archive holds every segment, `None` while it holds none.
pub type ArchiveHook = std::sync::Arc<dyn Fn() -> Option<Lsn> + Send + Sync>;

impl<R: Runtime> std::fmt::Debug for Pager<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<R: Runtime> std::fmt::Debug for Shared<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pager")
            .field("name", &self.name)
            .field("page_size", &self.page_size)
            .finish_non_exhaustive()
    }
}

pub(super) fn parent(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some(("", _)) => "/",
        Some((dir, _)) => dir,
        None => "",
    }
}

/// A shared latch on a resident page.
#[derive(Debug)]
pub struct PageShared<'a> {
    g: pages::Shared<'a>,
}

impl PageShared<'_> {
    /// Returns the id of the structure the page's header names as its owner.
    pub fn owner(&self) -> u32 {
        page::owner_of(self)
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn pid(&self) -> Pid {
        Pid(self.g.pid())
    }
}

impl Deref for PageShared<'_> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.g
    }
}

/// The exclusive latch on a resident page, through which it changes.
pub struct PageExclusive<'a, R: Runtime> {
    /// Taken only by the drop.
    held: Option<Held<'a, R>>,
    kind: Kind,
}

/// Where the bytes under an exclusive latch came from: whether its next change starts a chain,
/// and what its drop undoes.
#[derive(Clone, Copy)]
enum Kind {
    /// The page as read in or as last changed.
    Latched,
    /// A reservation's blank frame no allocation has committed, with the refusal it replaced.
    Blank { prior: Option<Damage> },
    /// A damaged page overwritten unread: its next change starts a chain, and its drop before
    /// one refuses the page again.
    Overwriting,
}

impl<R: Runtime> std::fmt::Debug for PageExclusive<'_, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.held().g.fmt(f)
    }
}

impl<R: Runtime> Deref for PageExclusive<'_, R> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        self.held()
    }
}

impl<'a, R: Runtime> PageExclusive<'a, R> {
    fn held(&self) -> &Held<'a, R> {
        self.held.as_ref().expect("held until the drop")
    }

    fn held_mut(&mut self) -> &mut Held<'a, R> {
        self.held.as_mut().expect("held until the drop")
    }

    pub fn pid(&self) -> Pid {
        self.held().pid()
    }

    /// Returns the id of the structure the page's header names as its owner.
    pub fn owner(&self) -> u32 {
        page::owner_of(self)
    }

    /// Change the page through `f`, which sees a copy, and log the bytes it changed.
    ///
    /// Returns the LSN of the record, or the LSN the next record gets if nothing changed.
    pub fn update(&mut self, f: impl FnOnce(&mut [u8])) -> Lsn {
        let mut copy = self.to_vec();
        f(&mut copy);
        let edits = diff(self, &copy);
        self.change(edits)
    }

    /// Apply `edits` to the page and log them.
    ///
    /// Panics if an edit writes the page's LSN or checksum or past its end.
    pub fn change(&mut self, edits: Vec<Edit>) -> Lsn {
        let pager = self.held().pager;
        pager.apply(&mut [self], vec![edits], None)
    }
}

impl<R: Runtime> Drop for PageExclusive<'_, R> {
    /// Undo a frame no logged change took over: evict a reservation's blank frame and refuse the
    /// page again as before it, or refuse a damaged page overwritten unread.
    fn drop(&mut self) {
        let Some(held) = self.held.take() else {
            return;
        };
        match self.kind {
            Kind::Latched => {}
            Kind::Blank { prior } => held.unblank(prior),
            Kind::Overwriting => {
                held.pager.refuse_latched(held, Damage::Checksum);
            }
        }
    }
}

/// An exclusive latch taken without reading the page in, resident or not. Only pager.rs writes
/// through it.
pub(crate) struct Held<'a, R: Runtime> {
    pager: &'a Pager<R>,
    g: pages::Exclusive<'a>,
}

impl<R: Runtime> Deref for Held<'_, R> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.g
    }
}

impl<'a, R: Runtime> Held<'a, R> {
    pub(crate) fn pid(&self) -> Pid {
        Pid(self.g.pid())
    }

    pub(crate) fn resident(&self) -> bool {
        self.g.resident()
    }

    /// Returns the latch as the guard a logged change goes through.
    ///
    /// Panic if the page is not resident.
    fn latched(self) -> PageExclusive<'a, R> {
        assert!(
            self.resident(),
            "a logged change applies to a resident page"
        );
        PageExclusive {
            held: Some(self),
            kind: Kind::Latched,
        }
    }

    /// Fill the frame with `buf`, the page as read from the file.
    #[expect(
        clippy::disallowed_methods,
        reason = "permanent: the page-in writer clippy.toml admits"
    )]
    fn read_in(&mut self, buf: &[u8]) {
        self.g.write(0, buf);
    }

    /// Install `image`, a logged full-page image, as the page at `lsn`.
    #[expect(
        clippy::disallowed_methods,
        reason = "permanent: the replay writer of a logged image clippy.toml admits"
    )]
    fn install_image(&mut self, image: &[u8], lsn: Lsn) {
        assert_eq!(
            self.pager.page_size,
            image.len(),
            "a page image is one page"
        );
        self.g.write(0, image);
        let (at, field) = page::lsn_field(lsn);
        self.g.write(at, &field);
    }

    /// Apply `edits` to the page and give it `lsn`, the LSN of the record logging them.
    #[expect(
        clippy::disallowed_methods,
        reason = "permanent: the writer of a logged or replayed change clippy.toml admits"
    )]
    fn apply_change(&mut self, edits: &[Edit], lsn: Lsn) {
        for e in edits {
            self.g.write(e.offset as usize, &e.bytes);
        }
        let (at, field) = page::lsn_field(lsn);
        self.g.write(at, &field);
    }

    /// Returns the guard of a reservation's blank frame: the page with `header` and zero
    /// contents, standing in for `prior`, its refusal before.
    #[expect(
        clippy::disallowed_methods,
        reason = "permanent: the blank-frame writer clippy.toml admits"
    )]
    fn blank(mut self, header: &[u8], prior: Option<Damage>) -> PageExclusive<'a, R> {
        self.pager.make_room(self.pid());
        let mut blank = vec![0; self.pager.page_size];
        blank[FIRST_EDITABLE..HEADER_LEN].copy_from_slice(header);
        self.g.write(0, &blank);
        PageExclusive {
            held: Some(self),
            kind: Kind::Blank { prior },
        }
    }

    /// Returns the guard of a damaged page overwritten unread, its frame zero unless resident.
    #[expect(
        clippy::disallowed_methods,
        reason = "permanent: the overwrite writer clippy.toml admits"
    )]
    fn overwrite(mut self) -> PageExclusive<'a, R> {
        if !self.resident() {
            self.pager.make_room(self.pid());
            self.g.write(0, &vec![0; self.pager.page_size]);
        }
        PageExclusive {
            held: Some(self),
            kind: Kind::Overwriting,
        }
    }

    /// Returns the guard of the page holding `copy`, the file's copy a settle changes from.
    #[expect(
        clippy::disallowed_methods,
        reason = "permanent: the settle's restore writer clippy.toml admits"
    )]
    fn restore_copy(mut self, copy: &[u8]) -> PageExclusive<'a, R> {
        if !self.resident() {
            self.pager.make_room(self.pid());
        }
        self.g.write(0, copy);
        self.latched()
    }

    /// Evict a reservation's blank frame no allocation committed, and refuse the page again with
    /// `prior`.
    fn unblank(self, prior: Option<Damage>) {
        let pid = self.pid();
        let pager = self.pager;
        let mut alloc = pager.allocation().lock(&pager.rt);
        if !alloc.is_blank(pid) {
            return;
        }
        if let Some(damage) = prior {
            pager.evidence().restore(&Refusal::of(&self), damage);
        }
        self.g.evict();
        pager.unreserve(pid);
        alloc.release(pid, Hold::Blank);
    }
}

impl<R: Runtime> Pager<R> {
    /// Returns the exclusive latch on `pid`, a free page a reservation holds, without reading the
    /// file: when not resident it is read in with `header` and zero contents, and no longer
    /// refused. The latch's drop undoes both unless an allocation of `pid` committed.
    pub(super) fn latch_blank(&self, pid: Pid, header: &[u8]) -> PageExclusive<'_, R> {
        let held = self.hold(pid);
        self.check_bound();
        if held.resident() {
            return held.latched();
        }
        let refusal = {
            let mut alloc = self.allocation().lock(&self.rt);
            alloc.blank(pid);
            self.evidence().unrefuse(&Refusal::of(&held))
        };
        held.blank(header, refusal)
    }

    /// Returns the exclusive latch on `pid`, damaged, over a blank frame unless resident, and no
    /// longer refused. Its next change logs an unchained image; its drop before one refuses the
    /// page again.
    pub(super) fn latch_overwritten(&self, pid: Pid) -> PageExclusive<'_, R> {
        let held = self.hold(pid);
        self.evidence().unrefuse(&Refusal::of(&held));
        held.overwrite()
    }

    /// Returns the exclusive latch on the page `stale` refuses, holding `copy`, the file's copy
    /// of that page, and no longer refused.
    ///
    /// Fail with `stale` when the page is resident, and with the page's refusal when it is refused
    /// other than as stale.
    pub(super) fn latch_restored(
        &self,
        stale: PageError,
        copy: &[u8],
    ) -> Result<PageExclusive<'_, R>, PageError> {
        let held = self.hold(stale.pid);
        if held.resident() {
            return Err(stale);
        }
        match self.refusal(stale.pid) {
            None | Some(Damage::Stale { .. }) => {}
            Some(damage) => return Err(PageError { damage, ..stale }),
        }
        self.evidence().unrefuse(&Refusal::of(&held));
        Ok(held.restore_copy(copy))
    }

    /// Read `pid` in for replay from the file's copy. Returns why the copy cannot be read in,
    /// leaving the page not resident.
    ///
    /// Preconditions: `pid` is not resident.
    pub(super) fn replay_page_in(&self, pid: Pid) -> Option<Damage> {
        let mut g = self.hold(pid);
        self.make_room(pid);
        let read = self.read_disk(pid);
        let failed = self.judge(pid, file_copy(&read), Against::LogEnd).damage();
        match read {
            Ok((buf, _)) if failed.is_none() => g.read_in(&buf),
            _ => self.unreserve(pid),
        }
        failed
    }

    /// Install `image`, the logged full-page image at `lsn`, as page `pid`.
    pub(super) fn replay_image(&self, pid: Pid, image: &[u8], lsn: Lsn) {
        let mut g = self.hold(pid);
        if !g.resident() {
            self.make_room(pid);
        }
        g.install_image(image, lsn);
    }

    /// Apply `edits`, the logged change at `lsn`, to page `pid`.
    ///
    /// Preconditions: `pid` is resident.
    pub(super) fn replay_change(&self, pid: Pid, edits: &[Edit], lsn: Lsn) {
        let mut g = self.hold(pid);
        assert!(g.resident(), "a replayed change applies to a resident page");
        g.apply_change(edits, lsn);
    }
}

/// An optimistic read of a page, trusted only once it validates.
pub struct PageOptimistic<'a, R: Runtime> {
    pager: &'a Pager<R>,
    o: pages::Optimistic<'a>,
}

impl<R: Runtime> std::fmt::Debug for PageOptimistic<'_, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.o.fmt(f)
    }
}

impl<'a, R: Runtime> PageOptimistic<'a, R> {
    #[cfg(any(test, feature = "testing"))]
    pub fn pid(&self) -> Pid {
        Pid(self.o.pid())
    }

    /// Returns what `f` computes from a copy of the page taken through atomic loads.
    ///
    /// The copy may be torn by a concurrent writer: `f` bounds-checks what it reads, and its
    /// result is used only once [`PageOptimistic::validate`] returns true.
    pub fn read<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        self.o.copied(0, self.pager.page_size, f)
    }

    pub fn validate(&self) -> bool {
        self.o.validate()
    }

    /// Take the exclusive latch if no exclusive latch was taken since this read began.
    #[expect(
        clippy::disallowed_methods,
        reason = "permanent: the optimistic upgrade clippy.toml admits"
    )]
    #[cfg(any(test, feature = "testing"))]
    pub fn upgrade(self) -> Option<PageExclusive<'a, R>> {
        let pager = self.pager;
        let g = self.o.upgrade()?;
        pager.check_bound();
        Some(Held { pager, g }.latched())
    }
}

impl<R: Runtime> Pager<R> {
    pub(super) fn assemble(
        rt: R,
        o: &Options,
        header: FileHeader,
        copy: u64,
        data: FileId,
    ) -> Self {
        let alloc = Allocation::new(header.bitmap_groups);
        let marks = Marks::new(header.quarantine);
        let evidence = Evidence::new(header.damage_list);
        let unmapped = evidence.unmapped_flag();
        let log = Log::new(header.log_block as usize, header.epoch, header.file_id);
        let segments = Segments::new(
            o.log_dir(),
            header.log_block as usize,
            header.segment_size,
            header.file_id,
        );
        Pager(std::sync::Arc::new(Shared {
            name: o.name.clone(),
            page_size: header.page_size as usize,
            pool_frames: o.pool_frames,
            stall_after: o.stall_after,
            data,
            mem: PageMemory::reserve(header.page_size as usize, o.max_pages),
            drivers: Lock::new(BTreeMap::new()),
            forcing: Lock::new(Forcing(())),
            header: Header::new(header, copy),
            log: Lock::new(Appended { log, map: None }),
            log_holder: AtomicU64::new(NO_HOLDER),
            meta_holder: AtomicU64::new(NO_HOLDER),
            segments: Lock::new(segments),
            meta: Lock::new(Meta {
                table: DirtyTable::new(o.atomic_page_writes),
                evidence,
                resident: Vec::new(),
                hand: 0,
            }),
            alloc,
            marks,
            unmapped,
            opening: std::sync::Arc::default(),
            archive: Lock::new(None),
            recycling: Lock::new(()),
            checkpointer: Lock::new(super::checkpoint::Worker::new(o.checkpoint_every)),
            rt,
        }))
    }

    /// Returns the pager of a new database whose id is `file_id` in the empty file `data` and
    /// the log directory of `options`, with its first checkpoint.
    pub(super) fn format_new(
        rt: R,
        options: &Options,
        data: FileId,
        file_id: [u8; 16],
    ) -> Result<Pager<R>, OpenError> {
        let header = FileHeader {
            seq: 0,
            file_id,
            page_size: options.page_size,
            log_block: options.log_block,
            segment_size: options.segment_size,
            lowest_format: 1,
            highest_format: 1,
            upgrade_state: 0,
            atomic_page_writes: options.atomic_page_writes,
            checkpoint: Lsn(0),
            catalog_root: Pid(0),
            quarantine: Pid(0),
            epoch: options.after.map_or(1, |at| {
                at.epoch + rt.next_u64() % super::backup::EPOCH_STEP + 1
            }),
            encryption: 0,
            bitmap_groups: 0,
            history: options
                .after
                .map(|at| (at.epoch, at.lsn))
                .into_iter()
                .collect(),
            log_start: Lsn(0),
            damage_list: Pid(0),
            log_damage: Vec::new(),
            log_damage_full: false,
        };
        let pager = Pager::assemble(rt, options, header, 1, data);
        pager.evidence().arm_created();
        let claimed = {
            let mut segments = pager.segments.lock(&pager.rt);
            pager.io(|d| segments.claim(d))
        };
        if !claimed {
            pager.close_files();
            return Err(OpenError::LogPresent);
        }
        let made = pager.reserve_marks().and_then(|(new, marks)| {
            pager.format_marks(new, marks)?;
            pager.create_damage_list()
        });
        if let Err(e) = made {
            pager.close_files();
            return Err(OpenError::Unformatted(e));
        }
        pager.checkpoint();
        pager.write_header(None);
        Ok(pager)
    }

    pub fn page_size(&self) -> usize {
        self.page_size
    }

    /// Returns the most threads the pool serves: those that may latch pages at once.
    #[cfg(any(test, feature = "testing"))]
    pub fn max_threads(&self) -> usize {
        self.pool_frames / LATCHES_PER_THREAD
    }

    pub(crate) fn rt(&self) -> &R {
        &self.rt
    }

    pub(super) fn data(&self) -> FileId {
        self.data
    }

    pub(super) fn segments(&self) -> &Lock<Segments> {
        &self.segments
    }

    pub(super) fn forcing(&self) -> &Lock<Forcing> {
        &self.forcing
    }

    pub(super) fn archive_hook(&self) -> &Lock<Option<ArchiveHook>> {
        &self.archive
    }

    pub(super) fn recycling(&self) -> &Lock<()> {
        &self.recycling
    }

    pub(super) fn checkpointer(&self) -> &Lock<super::checkpoint::Worker> {
        &self.checkpointer
    }

    pub(super) fn allocation(&self) -> &Allocation {
        &self.alloc
    }

    pub(super) fn opening_flag(&self) -> &std::sync::Arc<std::sync::atomic::AtomicBool> {
        &self.opening
    }

    pub(super) fn marks(&self) -> &Marks {
        &self.marks
    }

    pub(super) fn header(&self) -> &Header {
        &self.header
    }

    fn own_meta(&self) -> Holding<'_, Meta> {
        let me = self.rt.current();
        assert_ne!(
            self.log_holder.load(Ordering::Relaxed),
            me.0,
            "the meta lock is taken under the log lock only by an append"
        );
        self.take_meta(me)
    }

    fn take_meta(&self, me: ThreadId) -> Holding<'_, Meta> {
        assert_ne!(
            self.meta_holder.load(Ordering::Relaxed),
            me.0,
            "a thread holding the meta lock does not take it again"
        );
        Holding::take(&self.meta, &self.meta_holder, &self.rt, me)
    }

    /// Take the log lock.
    ///
    /// Panic if the calling thread holds the log lock or the meta lock.
    fn take_log(&self, me: ThreadId) -> Holding<'_, Appended> {
        assert_ne!(
            self.log_holder.load(Ordering::Relaxed),
            me.0,
            "a thread holding the log lock does not take it again"
        );
        assert_ne!(
            self.meta_holder.load(Ordering::Relaxed),
            me.0,
            "the log lock is never taken under the meta lock"
        );
        Holding::take(&self.log, &self.log_holder, &self.rt, me)
    }

    pub(super) fn dirty_table(&self) -> Locked<'_> {
        Locked(self.own_meta())
    }

    pub(super) fn evidence(&self) -> Evidential<'_> {
        Evidential(self.own_meta())
    }

    /// Returns whether a damage record may be unmapped, without the meta lock.
    pub(super) fn any_unmapped(&self) -> bool {
        self.unmapped.any()
    }

    /// Returns whether the frame of `pid` holds changes the file lacks.
    pub(super) fn is_unwritten(&self, pid: Pid) -> bool {
        self.own_meta().table.is_unwritten(pid)
    }

    pub(super) fn log(&self) -> LogGuard<'_> {
        LogGuard(self.take_log(self.rt.current()))
    }

    /// Take the log lock, then the meta lock.
    pub(super) fn append(&self) -> Append<'_, R> {
        let me = self.rt.current();
        let log = self.take_log(me);
        Append::new(self, log, self.take_meta(me))
    }

    /// Returns the page LSN map's root, `None` while the file has no map.
    pub(super) fn map_root(&self) -> Option<MapRoot> {
        self.take_log(self.rt.current()).map
    }

    /// Make `map` the page LSN map's root, as recovery found it.
    pub(super) fn recover_map_root(&self, _: &Replaying, map: Option<MapRoot>) {
        self.take_log(self.rt.current()).map = map;
    }

    /// Run `f` on the calling thread's I/O queue.
    pub(crate) fn io<T>(&self, f: impl FnOnce(&mut Driver<R>) -> T) -> T {
        let me = self.rt.current();
        let taken = self.drivers.lock(&self.rt).remove(&me);
        let mut drv = taken.unwrap_or_else(|| Driver::new(self.rt.clone()));
        let out = f(&mut drv);
        self.drivers.lock(&self.rt).insert(me, drv);
        out
    }

    fn refused(&self, pid: Pid) -> Result<(), PageError> {
        match self.refusal(pid) {
            Some(damage) => Err(PageError { pid, damage }),
            None => Ok(()),
        }
    }

    /// Returns a shared latch on page `pid`, reading and verifying it if it is not resident.
    ///
    /// A page never written reads as zeros with LSN 0.
    pub fn shared(&self, pid: Pid) -> Result<PageShared<'_>, PageError> {
        loop {
            self.refused(pid)?;
            if let Some(g) = self.mem.lock_shared(pid.0, &self.rt) {
                self.check_bound();
                return Ok(PageShared { g });
            }
            if let Some(held) = self.resident(pid)? {
                return Ok(PageShared { g: held.g.share() });
            }
        }
    }

    /// Returns a shared latch on page `pid` if it is resident, reading nothing.
    pub(crate) fn shared_resident(&self, pid: Pid) -> Option<PageShared<'_>> {
        let g = self.mem.lock_shared(pid.0, &self.rt)?;
        Some(PageShared { g })
    }

    /// Returns the exclusive latch on page `pid` without reading it in.
    #[expect(
        clippy::disallowed_methods,
        reason = "permanent: the exclusive latch clippy.toml admits, which only pager.rs writes through"
    )]
    pub(crate) fn hold(&self, pid: Pid) -> Held<'_, R> {
        Held {
            pager: self,
            g: self.mem.lock_exclusive(pid.0, &self.rt),
        }
    }

    /// Returns the exclusive latch on page `pid`, reading and verifying it if it is not resident.
    pub fn exclusive(&self, pid: Pid) -> Result<PageExclusive<'_, R>, PageError> {
        assert!(pid >= Pid::FIRST_DATA, "a change writes a file header page");
        loop {
            if let Some(held) = self.resident(pid)? {
                return Ok(held.latched());
            }
        }
    }

    /// Start an optimistic read of page `pid`, reading it in first if it is not resident.
    pub fn optimistic(&self, pid: Pid) -> Result<PageOptimistic<'_, R>, PageError> {
        loop {
            self.refused(pid)?;
            if let Some(o) = self.mem.optimistic(pid.0) {
                return Ok(PageOptimistic { pager: self, o });
            }
            match self.mem.state(pid.0).0 {
                PageState::Evicted => drop(self.resident(pid)?),
                _ => self.rt.yield_now(),
            }
        }
    }

    /// Returns the page `pid` under a shared latch; see [`Pager::shared`].
    pub fn read(&self, pid: Pid) -> Result<PageShared<'_>, PageError> {
        self.shared(pid)
    }

    /// Returns the page `pid` under a shared latch, as [`Pager::read`] does, refusing it when every
    /// byte of it is zero.
    ///
    /// Preconditions: a structure in the database names `pid`, so the batch that named it formatted
    /// it, and the caller holds no latch on `pid`.
    pub fn read_formatted(&self, pid: Pid) -> Result<PageShared<'_>, PageError> {
        let page = self.read(pid)?;
        if page.iter().all(|&b| b == 0) {
            drop(page);
            return Err(self.damage(pid, Damage::Checksum));
        }
        Ok(page)
    }

    /// Returns the exclusive latch on `pid`, as [`Pager::exclusive`] does, refusing the page when
    /// every byte of it is zero.
    ///
    /// Preconditions: a structure in the database names `pid`, so the batch that named it formatted
    /// it, and the caller holds no latch on `pid`.
    pub fn exclusive_formatted(&self, pid: Pid) -> Result<PageExclusive<'_, R>, PageError> {
        let page = self.exclusive(pid)?;
        if page.iter().all(|&b| b == 0) {
            drop(page);
            return Err(self.damage(pid, Damage::Checksum));
        }
        Ok(page)
    }

    /// Panic, in debug builds and when `latch-checks` is on, if the calling thread latches more
    /// than [`LATCHES_PER_THREAD`] pages.
    pub(super) fn check_bound(&self) {
        let held = self.mem.latched_here();
        assert!(
            held <= LATCHES_PER_THREAD,
            "this thread latches {held} pages, more than {LATCHES_PER_THREAD}"
        );
    }

    /// Returns the exclusive latch on `pid` with the page resident, or `None` if it was found
    /// damaged by another thread meanwhile and the caller should look again. A copy below its
    /// expected LSN of a page the allocation bitmap calls free is read in unrefused.
    fn resident(&self, pid: Pid) -> Result<Option<Held<'_, R>>, PageError> {
        self.refused(pid)?;
        let mut g = self.hold(pid);
        self.check_bound();
        if g.resident() {
            return Ok(Some(g));
        }
        if self.refusal(pid).is_some() {
            return Ok(None);
        }
        self.make_room(pid);
        let read = self.read_disk(pid);
        let refused = match self.judge(pid, file_copy(&read), Against::Expected) {
            Judged::Behind(_) if self.bitmapped(pid) == Bitmapped::Free => None,
            judged => judged.damage(),
        };
        if let Some(damage) = refused {
            return Err(self.refuse_latched(g, damage));
        }
        let (buf, _) = read.unwrap();
        g.read_in(&buf);
        Ok(Some(g))
    }

    /// Refuse `pid` from now on, dropping its memory. Memory holding changes the file lacks is
    /// written first, so the next force of the data file drops its dirty entry; during recovery
    /// or on a file quarantined whole it is dropped unwritten and its dirty entry stays until a
    /// free of the page is durable, so recovery replays them. Record the damage unless it is
    /// [`Damage::Unreadable`], durable on return once recovery has ended and unless the file is
    /// quarantined whole.
    ///
    /// Preconditions: the caller holds no latch on `pid`, and neither the log lock nor the meta
    /// lock.
    pub(crate) fn damage(&self, pid: Pid, damage: Damage) -> PageError {
        self.refuse_latched(self.hold(pid), damage)
    }

    /// Refuse the page under `latch` as [`Pager::damage`] does, for a caller that judged it
    /// damaged under that latch.
    ///
    /// Preconditions: the caller holds neither the log lock nor the meta lock.
    pub(super) fn refuse_latched(&self, latch: Held<'_, R>, damage: Damage) -> PageError {
        let pid = latch.pid();
        if latch.resident() && self.writes_refused(pid) {
            self.write_out(pid, &latch);
        }
        let logged = self.append().refuse(&latch, damage);
        if latch.resident() {
            latch.g.evict();
        } else {
            drop(latch);
        }
        if let Some(end) = logged {
            self.force_below(end);
        }
        self.unreserve(pid);
        PageError { pid, damage }
    }

    /// Returns whether the resident copy of `pid` holds changes the file lacks and may be written
    /// before it is refused: recovery has ended and the file is not quarantined whole.
    fn writes_refused(&self, pid: Pid) -> bool {
        if self.is_quarantined(Quarantine::Database) {
            return false;
        }
        let m = self.own_meta();
        m.table.is_unwritten(pid) && m.evidence.armed()
    }

    /// Stop counting `pid` as resident.
    pub(super) fn unreserve(&self, pid: Pid) {
        let mut m = self.own_meta();
        if let Some(i) = m.resident.iter().position(|p| *p == pid) {
            m.resident.swap_remove(i);
        }
    }

    /// Returns the LSN the file's copy of `pid` is below: the log's next LSN once this open has
    /// written the page, and the end of the log recovery kept before.
    ///
    /// Preconditions: the caller holds a latch on `pid` that no write-out holds with, or judges
    /// again under one.
    pub(super) fn file_end(&self, pid: Pid) -> Lsn {
        let end = {
            let m = self.own_meta();
            m.table.file_end(pid, m.evidence.recovered())
        };
        end.unwrap_or_else(|| self.log().next_lsn())
    }

    /// Returns the page `pid` as the file holds it, verified.
    pub(super) fn read_disk(&self, pid: Pid) -> Result<(AlignedBuf, Verdict), Damage> {
        let size = self.page_size;
        let buf = self
            .io(|d| d.read(self.data, pid.0 * size as u64, size))
            .map_err(|_| Damage::Unreadable)?;
        let verdict = page::verify(pid, &buf);
        Ok((buf, verdict))
    }

    /// Count `pid` as resident, first evicting pages by the clock while the pool is full.
    ///
    /// Panics when every frame stays latched for longer than the pool's stall bound.
    #[expect(
        clippy::disallowed_methods,
        reason = "permanent: eviction's try-lock clippy.toml admits"
    )]
    pub(super) fn make_room(&self, pid: Pid) {
        let mut since = None;
        loop {
            let victim = {
                let mut m = self.own_meta();
                if m.resident.len() < self.pool_frames {
                    m.resident.push(pid);
                    return;
                }
                let mut found = None;
                for _ in 0..2 * m.resident.len() {
                    let i = m.hand % m.resident.len();
                    let p = m.resident[i];
                    if self.mem.mark(p.0)
                        && let Some(g) = self.mem.try_lock_exclusive(p.0)
                    {
                        m.resident.swap_remove(i);
                        found = Some(g);
                        break;
                    }
                    m.hand = i + 1;
                }
                found
            };
            match victim {
                Some(g) => self.evict(g),
                None => {
                    let now = self.rt.now();
                    let start = *since.get_or_insert(now);
                    assert!(
                        now.saturating_since(start) <= self.stall_after,
                        "the pool is stalled: all {} frames have stayed latched for {:?}",
                        self.pool_frames,
                        self.stall_after
                    );
                    self.rt.yield_now();
                }
            }
        }
    }

    fn evict(&self, g: pages::Exclusive<'_>) {
        let pid = Pid(g.pid());
        if self.is_unwritten(pid) {
            self.write_out(pid, &g);
        }
        g.evict();
    }

    /// Write `bytes`, page `pid` under a latch the caller holds, to the file.
    pub(super) fn write_out(&self, pid: Pid, bytes: &[u8]) {
        let staged = self.staged(pid, bytes);
        let offset = pid.0 * self.page_size as u64;
        self.io(|d| d.write_all(vec![(self.data, offset, staged)]));
        let mut m = self.own_meta();
        let wrote = Wrote(pid);
        m.evidence.written_out(&wrote);
        m.table.written_out(wrote);
    }

    /// Returns `bytes` sealed for the device, once the log holds everything the page depends on.
    pub(super) fn staged(&self, pid: Pid, bytes: &[u8]) -> AlignedBuf {
        self.wait_durable(page::lsn(bytes));
        let mut staged = AlignedBuf::from_slice(bytes);
        page::seal(pid, &mut staged);
        staged
    }

    /// Apply `edits[i]` to the page under `guards[i]` and log them, one record per page with
    /// edits, then `tail` if given, all in one batch. A `tail` naming a new map root makes it the
    /// map before any later record can be logged.
    ///
    /// Returns the LSN of `tail`, or else of the last page record.
    pub(crate) fn apply(
        &self,
        guards: &mut [&mut PageExclusive<'_, R>],
        edits: Vec<Vec<Edit>>,
        tail: Option<Record>,
    ) -> Lsn {
        self.apply_freeing(guards, edits, None, tail)
    }

    /// Apply as [`Pager::apply`] does, for the allocation `new` when given: each page it
    /// allocates is no longer blank and its record starts a chain, and the damage recorded of
    /// each page it frees is dropped, logged in the batch before `tail`.
    pub(super) fn apply_freeing(
        &self,
        guards: &mut [&mut PageExclusive<'_, R>],
        edits: Vec<Vec<Edit>>,
        new: Option<&Reservation<'_, R>>,
        tail: Option<Record>,
    ) -> Lsn {
        let allocated: Vec<Pid> = new.map(|n| n.allocated().collect()).unwrap_or_default();
        let freed = new.map_or(&[][..], Reservation::freed);
        let size = self.page_size;
        for e in edits.iter().flatten() {
            let at = e.offset as usize;
            assert!(
                at >= FIRST_EDITABLE && at + e.bytes.len() <= size,
                "edit of {} bytes at {at} is outside the page's contents",
                e.bytes.len()
            );
        }
        let mut append = self.append();
        let mut last = append.log.log.next_lsn();
        for (g, edits) in guards.iter_mut().zip(edits) {
            let start = matches!(g.kind, Kind::Overwriting) || allocated.contains(&g.pid());
            if edits.is_empty() && !start {
                continue;
            }
            last = append.change_page(g, edits, start);
        }
        if !freed.is_empty() {
            last = append.free(freed).unwrap_or(last);
        }
        match tail {
            Some(Record::MapRoot { root, height }) => {
                let lsn = guards
                    .iter()
                    .find(|g| g.pid() == root)
                    .map(|g| page::lsn(g));
                append.map_root(
                    root,
                    lsn.expect("a map root is logged with its page"),
                    height,
                )
            }
            Some(record) => append.record(&record),
            None => last,
        }
    }

    /// Run `f` with exclusive latches on `pids`, taken in page order, passing them in the order
    /// given.
    pub(crate) fn with_latched<'s, T>(
        &'s self,
        pids: &[Pid],
        f: impl FnOnce(&'s Self, &mut [&mut PageExclusive<'s, R>]) -> T,
    ) -> Result<T, PageError> {
        self.with_latched_by(pids, |pid| self.exclusive(pid), f)
    }

    /// Run `f` as [`Pager::with_latched`] does, taking each latch through `latch`.
    pub(crate) fn with_latched_by<'s, T>(
        &'s self,
        pids: &[Pid],
        latch: impl Fn(Pid) -> Result<PageExclusive<'s, R>, PageError>,
        f: impl FnOnce(&'s Self, &mut [&mut PageExclusive<'s, R>]) -> T,
    ) -> Result<T, PageError> {
        let distinct: BTreeSet<Pid> = pids.iter().copied().collect();
        assert_eq!(
            distinct.len(),
            pids.len(),
            "a page appears twice in one change"
        );
        assert!(
            pids.len() <= self.pool_frames,
            "a change needs more frames than the pool has"
        );
        let mut held: BTreeMap<Pid, PageExclusive<'s, R>> = BTreeMap::new();
        for &pid in &distinct {
            held.insert(pid, latch(pid)?);
        }
        let mut ordered: Vec<PageExclusive<'s, R>> =
            pids.iter().map(|p| held.remove(p).unwrap()).collect();
        let mut refs: Vec<&mut PageExclusive<'s, R>> = ordered.iter_mut().collect();
        Ok(f(self, &mut refs))
    }

    /// Write every changed page, take a checkpoint, and close the data file and every log segment.
    ///
    /// Preconditions: every other call on the pager has returned, and none follows.
    pub fn close(&self) {
        self.stop_checkpointer();
        self.flush_log();
        self.flush_pages(usize::MAX);
        self.checkpoint();
        self.close_files();
    }

    pub(super) fn close_files(&self) {
        self.io(|d| {
            self.segments.lock(&self.rt).close(d);
            d.close(self.data);
        });
    }

    /// Returns the wall-clock time of the runtime the pager runs on.
    pub fn wall(&self) -> SystemTime {
        self.rt.wall()
    }
}
