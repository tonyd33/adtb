use std::collections::{BTreeMap, BTreeSet};
use std::ops::Deref;
use std::sync::MutexGuard;
use std::time::{Duration, SystemTime};

use adtb_sys::pages::{self, PageMemory, PageState};

use crate::runtime::{AlignedBuf, FileId, IoError, Runtime, ThreadId};

use super::alloc::{Reservation, diff};
use super::block::LogFinding;
use super::create::{Created, check_options, create};
use super::damage_log::Met;
use super::damage_log::Refusal;
use super::driver::Driver;
use super::header::{self, FileHeader, REGION};
use super::log::{Log, Segments};
use super::lsnmap::{Expected, Gate, retained};
use super::page::{self, FIRST_EDITABLE, HEADER_LEN, Verdict};
use super::pagetable::{DirtyTable, Logged};
use super::quarantine::Quarantine;
use super::record::{Edit, MapRoot, Record};
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

impl Options {
    pub fn new(name: &str) -> Options {
        Options {
            name: name.to_owned(),
            page_size: 8192,
            log_block: 512,
            segment_size: 16 << 20,
            pool_frames: 1024,
            max_pages: 1 << 24,
            atomic_page_writes: false,
            threads: 1,
            stall_after: Duration::from_secs(10),
            after: None,
            checkpoint_every: 64 << 20,
        }
    }

    /// Returns the path of the directory holding the log's segments.
    pub fn log_dir(&self) -> String {
        format!("{}-log", self.name)
    }
}

/// Why a page cannot be handed out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Damage {
    /// The page fails its checksum.
    Checksum,
    /// The device refused to read it.
    Unreadable,
    /// The page's LSN is not the one the log says it must be: a lost page write when `found` is
    /// before `expected`, a lost log record when past it.
    Stale { expected: Lsn, found: Lsn },
    /// The page holds ids in progress that the log, damaged at `at`, cannot settle.
    LogUnproven { at: Lsn },
    /// A commit log status page lost the write to `expected` and was settled from the log at
    /// `found`. The page serves again.
    Settled { expected: Lsn, found: Lsn },
}

impl Damage {
    /// Returns the damage of a page the file holds at LSN `found`, other than a blank one, when the
    /// log holds no record at or past `end`.
    pub(crate) fn past_log_end(found: Lsn, end: Lsn) -> Option<Damage> {
        (found >= end && found != Lsn(0)).then_some(Damage::Stale {
            expected: end,
            found,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageError {
    pub pid: Pid,
    pub damage: Damage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenError {
    NotFound,
    /// `create` found a nonempty file.
    Exists,
    /// `create` found the log directory holding what the new file did not write.
    LogPresent,
    /// `create` found its reserved directory holding what no create left.
    Reserved,
    /// Another open holds the lock on a file `create` would remove or rename.
    Locked,
    /// `create` took the log directory, then failed on a page of the new file.
    Unformatted(PageError),
    /// The options' segment size is not a positive multiple of their log block.
    SegmentSize(u64),
    /// Neither copy of the file header verifies.
    NoHeader,
    /// Opening the data file failed.
    Io(IoError),
    /// A whole copy of the file header is written in this layout, which this engine does not read.
    OtherLayout(u32),
    /// The log is damaged where recovery needs it.
    Log(LogFinding),
    /// A page opening needs, an allocation bitmap page or the quarantine marks', is damaged.
    Page(PageError),
}

impl From<PageError> for OpenError {
    fn from(e: PageError) -> OpenError {
        OpenError::Page(e)
    }
}

/// What the pool knows of its pages beyond their memory and state words.
#[derive(Debug)]
pub(crate) struct Meta {
    table: DirtyTable,
    /// The LSN each page must have at least when read in: its last record in the retained log, or
    /// else the page LSN map's entry, as of the end of recovery, raised by every page record
    /// logged since.
    pub(crate) gate: Gate,
    pub(crate) damaged: BTreeMap<Pid, Damage>,
    pub(crate) records: super::damage_log::Records,
    /// The bitmap groups a batch has started: their bitmap pages are never blank. A batch counts
    /// here only once it is in the log and while its bitmap page is latched, so a checkpoint that
    /// captures a lower count finds that page dirty and its redo point covers the batch.
    pub(crate) groups: u64,
    /// The free pages reservations hold, each until its reservation and any blank latch of it
    /// end.
    pub(crate) reserved: BTreeSet<Pid>,
    /// The reserved pages latched blank whose allocation has not committed.
    pub(crate) blanked: BTreeSet<Pid>,
    /// The end of the log recovery kept when the pager opened, or zero for a pager created empty.
    pub(crate) recovered: Lsn,
    /// The page LSN map's root, `None` while the file has no map.
    pub(crate) map: Option<MapRoot>,
    /// Resident pages and pages being read in, in clock order.
    resident: Vec<Pid>,
    hand: usize,
}

impl Meta {
    pub(super) fn refused(&mut self, refusal: &Refusal) {
        self.table.refused(refusal);
    }
}

/// The dirty page table, under the meta lock.
pub(super) struct Locked<'a>(MutexGuard<'a, Meta>);

impl Deref for Locked<'_> {
    type Target = DirtyTable;

    fn deref(&self) -> &DirtyTable {
        &self.0.table
    }
}

impl std::ops::DerefMut for Locked<'_> {
    fn deref_mut(&mut self) -> &mut DirtyTable {
        &mut self.0.table
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
///   full-page image, until a data file force covers a write of it with no change since.
/// - A page record and its dirty page table entry are made under the log lock, so a checkpoint,
///   which snapshots the table under the same lock, never misses a page whose image precedes it.
///
/// Lock order: the recycle lock, then page latches, then the flush lock, then the log lock, then
/// the meta lock. The header, allocation, archive and driver locks are taken alone. The
/// checkpointer lock is taken last, under any of these, and nothing is taken under it.
///
/// Clones are handles on one pager.
pub struct Pager<R: Runtime>(pub(super) std::sync::Arc<Shared<R>>);

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
pub struct Shared<R: Runtime> {
    pub(crate) rt: R,
    pub(crate) name: String,
    pub(crate) page_size: usize,
    pool_frames: usize,
    stall_after: Duration,
    pub(crate) data: FileId,
    mem: PageMemory,
    drivers: Lock<BTreeMap<ThreadId, Driver<R>>>,
    /// Held by a force of the data file from taking the pages written to dropping pages from the
    /// dirty page table.
    pub(super) forcing: Lock<Forcing>,
    pub(super) header: Lock<(FileHeader, u64)>,
    pub(crate) log: Lock<Log>,
    /// Held by the one thread writing and forcing a batch: the group commit leader.
    pub(crate) segments: Lock<Segments>,
    pub(crate) meta: Lock<Meta>,
    /// Held across a search for a free page and across marking pages allocated.
    pub(crate) alloc: Lock<super::alloc::AllocState>,
    pub(crate) marks: Lock<super::quarantine::Marks>,
    /// Whether `marks` holds any mark. Set before a mark shows in `marks`.
    pub(crate) any_mark: std::sync::atomic::AtomicBool,
    /// Whether a damage record may be unmapped. Set under the meta lock with the record.
    pub(crate) any_unmapped: std::sync::atomic::AtomicBool,
    /// How many damaged pages this open has walked the objects for.
    pub(crate) walked: std::sync::atomic::AtomicU64,
    pub(super) archive: Lock<Option<ArchiveHook>>,
    /// Held across a recycle of the log, taken before any other lock or latch.
    pub(super) recycling: Lock<()>,
    pub(super) checkpointer: Lock<super::checkpoint::Worker>,
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
        let mut m = self.pager.own_meta();
        if !m.blanked.remove(&pid) {
            return;
        }
        if let Some(damage) = prior {
            m.damaged.insert(pid, damage);
        }
        m.reserved.remove(&pid);
        drop(m);
        let pager = self.pager;
        self.g.evict();
        pager.unreserve(pid);
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
            let mut m = self.own_meta();
            let refusal = m.damaged.remove(&pid);
            m.blanked.insert(pid);
            refusal
        };
        held.blank(header, refusal)
    }

    /// Returns the exclusive latch on `pid`, damaged, over a blank frame unless resident, and no
    /// longer refused. Its next change logs an unchained image; its drop before one refuses the
    /// page again.
    pub(super) fn latch_overwritten(&self, pid: Pid) -> PageExclusive<'_, R> {
        let g = self.hold(pid).overwrite();
        self.own_meta().damaged.remove(&pid);
        g
    }

    /// Returns the exclusive latch on `pid` holding `copy`, the file's copy of a page refused as
    /// stale, and no longer refused.
    pub(super) fn latch_restored(&self, pid: Pid, copy: &[u8]) -> PageExclusive<'_, R> {
        let g = self.hold(pid).restore_copy(copy);
        self.own_meta().damaged.remove(&pid);
        g
    }

    /// Read `pid` in for replay from the file's copy. Returns why the copy cannot be read in,
    /// leaving the page not resident.
    ///
    /// Preconditions: `pid` is not resident.
    pub(super) fn replay_page_in(&self, pid: Pid) -> Option<Damage> {
        let mut g = self.hold(pid);
        self.make_room(pid);
        let failed = match self.read_disk(pid) {
            Ok((buf, Verdict::Valid | Verdict::Blank)) => {
                let past = Damage::past_log_end(page::lsn(&buf), self.file_end(pid));
                if past.is_none() {
                    g.read_in(&buf);
                }
                past
            }
            Ok((_, Verdict::Corrupt)) => Some(Damage::Checksum),
            Err(d) => Some(d),
        };
        if failed.is_some() {
            self.unreserve(pid);
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
    pub fn upgrade(self) -> Option<PageExclusive<'a, R>> {
        let pager = self.pager;
        let g = self.o.upgrade()?;
        pager.check_bound();
        Some(Held { pager, g }.latched())
    }
}

impl<R: Runtime> Pager<R> {
    fn assemble(rt: R, o: &Options, header: FileHeader, copy: u64, data: FileId) -> Self {
        let bitmap_groups = header.bitmap_groups;
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
            header: Lock::new((header, copy)),
            log: Lock::new(log),
            segments: Lock::new(segments),
            meta: Lock::new(Meta {
                table: DirtyTable::new(o.atomic_page_writes),
                gate: Gate::Recovering,
                damaged: BTreeMap::new(),
                records: Default::default(),
                groups: bitmap_groups,
                reserved: BTreeSet::new(),
                blanked: BTreeSet::new(),
                recovered: Lsn(0),
                map: None,
                resident: Vec::new(),
                hand: 0,
            }),
            alloc: Lock::new(Default::default()),
            marks: Lock::new(Default::default()),
            any_mark: std::sync::atomic::AtomicBool::new(false),
            any_unmapped: std::sync::atomic::AtomicBool::new(false),
            walked: std::sync::atomic::AtomicU64::new(0),
            archive: Lock::new(None),
            recycling: Lock::new(()),
            checkpointer: Lock::new(super::checkpoint::Worker::new(o.checkpoint_every)),
            rt,
        }))
    }

    /// Create an empty database: its data file, its first log segment and a first checkpoint,
    /// built as [`create`] does and published.
    ///
    /// Panics if `options` breaks a limit of the file format.
    pub fn create(rt: R, options: &Options) -> Result<Pager<R>, OpenError> {
        let (pager, new) = match create(rt.clone(), options, &mut |_| Ok(()))? {
            Created::Finished(pager) => return Ok(pager),
            Created::Built(pager, new) => (pager, new),
        };
        if let Err(e) = new.publish(&pager, &mut |_| Ok(())) {
            pager.close();
            new.discard(rt, &mut |_| Ok(()))?;
            return Err(e);
        }
        Ok(pager)
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
        pager.own_meta().gate = Gate::Armed(Expected::default());
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
            pager.header.lock(&pager.rt).0.quarantine = marks;
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

    /// Open an existing database and recover it from its log.
    pub fn open(rt: R, options: &Options) -> Result<(Pager<R>, super::Recovery), OpenError> {
        check_options(options)?;
        let mut drv = Driver::new(rt.clone());
        let (data, _) = drv.open(&options.name, false).map_err(|e| match e {
            IoError::NotFound => OpenError::NotFound,
            e => OpenError::Io(e),
        })?;
        let read = |drv: &mut Driver<R>, at: u64| drv.read(data, at, REGION).ok();
        let first = read(&mut drv, 0).and_then(|b| FileHeader::decode(0, &b));
        let mut second: Option<FileHeader> = None;
        for at in header::second_copy_offsets() {
            let found = read(&mut drv, at)
                .and_then(|b| FileHeader::decode(1, &b))
                .filter(|h| u64::from(h.page_size) == at);
            if let Some(h) = found
                && second.as_ref().is_none_or(|s| h.seq > s.seq)
            {
                second = Some(h);
            }
        }
        let Some((copy, mut header)) = header::choose(first, second) else {
            let layout = read(&mut drv, 0)
                .and_then(|b| header::other_layout(0, &b))
                .or_else(|| {
                    header::second_copy_offsets()
                        .find_map(|at| read(&mut drv, at).and_then(|b| header::other_layout(1, &b)))
                });
            drv.close(data);
            return Err(layout.map_or(OpenError::NoHeader, OpenError::OtherLayout));
        };
        header.atomic_page_writes = options.atomic_page_writes;
        let pager = Pager::assemble(rt, options, header, copy, data);
        match pager.recover_and_mark() {
            Ok(recovery) => {
                pager.start_checkpoints();
                Ok((pager, recovery))
            }
            Err(e) => {
                pager.close_files();
                Err(e)
            }
        }
    }

    fn recover_and_mark(&self) -> Result<super::Recovery, OpenError> {
        let (recovery, pages, armed) = self.recover()?;
        self.refuse_cut_pages(pages);
        let marks = self.header.lock(&self.rt).0.quarantine;
        if marks == Pid(0) {
            let (new, marks) = self.reserve_marks()?;
            self.format_marks(new, marks)?;
            self.header.lock(&self.rt).0.quarantine = marks;
        } else {
            self.load_marks(marks)?;
        }
        self.load_damage_list()?;
        self.start_recording();
        self.checkpoint();
        self.check_map();
        if let Err(map) = armed {
            if map.is_some_and(|e| e.damage != Damage::Unreadable) {
                self.quarantine(Quarantine::Database).ok();
            }
            self.hold_quarantine(Quarantine::Database);
        }
        Ok(recovery)
    }

    /// End recovery: set each page's expected LSN from the page LSN map, then the retained log.
    /// Turn the gate off, setting none, when the log cannot prove a record before the end
    /// recovery kept, or fail with the map page refused.
    pub(super) fn arm(&self) -> super::recovery::Armed {
        let armed = self.map_entries().map_err(Some).and_then(|folded| {
            let end = self.own_meta().recovered;
            let (_pin, mut img) = self.load_log().map_err(|_| None)?;
            let last = retained(&mut img, end).map_err(|_| None)?;
            let mut expected = Expected::new(folded);
            for (pid, lsn) in last {
                expected.raise(pid, lsn);
            }
            Ok(expected)
        });
        let mut m = self.own_meta();
        match armed {
            Ok(expected) => {
                m.gate = Gate::Armed(expected);
                Ok(())
            }
            Err(e) => {
                m.gate = Gate::Off;
                Err(e)
            }
        }
    }

    pub fn page_size(&self) -> usize {
        self.page_size
    }

    /// Returns the most threads the pool serves: those that may latch pages at once.
    pub fn max_threads(&self) -> usize {
        self.pool_frames / LATCHES_PER_THREAD
    }

    /// Returns the pages refused so far, each with what is wrong with it.
    pub(crate) fn damaged(&self) -> BTreeMap<Pid, Damage> {
        self.own_meta().damaged.clone()
    }

    pub(crate) fn meta(&self) -> MutexGuard<'_, Meta> {
        self.own_meta()
    }

    fn own_meta(&self) -> MutexGuard<'_, Meta> {
        self.meta.lock(&self.rt)
    }

    pub(super) fn dirty_table(&self) -> Locked<'_> {
        Locked(self.own_meta())
    }

    /// Returns whether the frame of `pid` holds changes the file lacks.
    pub(super) fn is_unwritten(&self, pid: Pid) -> bool {
        self.own_meta().table.is_unwritten(pid)
    }

    pub(crate) fn log(&self) -> MutexGuard<'_, Log> {
        self.log.lock(&self.rt)
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
        match self.own_meta().damaged.get(&pid) {
            Some(&damage) => Err(PageError { pid, damage }),
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
    /// damaged by another thread meanwhile and the caller should look again.
    fn resident(&self, pid: Pid) -> Result<Option<Held<'_, R>>, PageError> {
        self.refused(pid)?;
        let mut g = self.hold(pid);
        self.check_bound();
        if g.resident() {
            return Ok(Some(g));
        }
        if self.own_meta().damaged.contains_key(&pid) {
            return Ok(None);
        }
        self.make_room(pid);
        let read = self.read_disk(pid);
        let damage = match &read {
            Err(d) => Some(*d),
            Ok((_, Verdict::Corrupt)) => Some(Damage::Checksum),
            Ok((buf, _)) => {
                let found = page::lsn(buf);
                Damage::past_log_end(found, self.file_end(pid))
                    .or_else(|| self.lost_write(pid, found, None))
            }
        };
        if let Some(damage) = damage {
            return Err(self.refuse_latched(g, damage));
        }
        let (buf, _) = read.unwrap();
        g.read_in(&buf);
        Ok(Some(g))
    }

    /// Returns `Stale` when `pid` must have an LSN past `found`, the LSN of the file's copy: the LSN
    /// of `image`, its resident copy, changed or not, or else its expected LSN.
    ///
    /// Preconditions: the caller holds a latch on `pid` and `image` is the resident copy under it,
    /// or the caller judges again under one.
    pub(crate) fn lost_write(&self, pid: Pid, found: Lsn, image: Option<&[u8]>) -> Option<Damage> {
        let expected = match image {
            Some(image) => page::lsn(image),
            None => self.own_meta().gate.expected(pid)?,
        };
        (found < expected).then_some(Damage::Stale { expected, found })
    }

    /// Refuse `pid` from now on, dropping its memory. Memory holding changes the file lacks is
    /// written first, so the next force of the data file drops its dirty entry; during recovery
    /// or on a file quarantined whole it is dropped unwritten and its dirty entry stays, so
    /// recovery replays them. Record the damage unless it is [`Damage::Unreadable`], durable on
    /// return once recovery has ended and unless the file is quarantined whole.
    ///
    /// Preconditions: the caller holds no latch on `pid`, and neither the log lock nor the meta
    /// lock.
    #[doc(hidden)]
    pub fn damage(&self, pid: Pid, damage: Damage) -> PageError {
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
        let logged = self.publish_refusal(&latch, damage);
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
        m.table.is_unwritten(pid) && matches!(m.gate, Gate::Armed(_))
    }

    /// Stop counting `pid` as resident.
    pub(crate) fn unreserve(&self, pid: Pid) {
        let mut m = self.own_meta();
        if let Some(i) = m.resident.iter().position(|p| *p == pid) {
            m.resident.swap_remove(i);
        }
    }

    /// Returns whether the file's copy `buf` of `pid` verifies and is behind [`Pager::file_end`].
    pub(crate) fn sound(&self, pid: Pid, buf: &[u8]) -> bool {
        !matches!(page::verify(pid, buf), Verdict::Corrupt)
            && Damage::past_log_end(page::lsn(buf), self.file_end(pid)).is_none()
    }

    /// Returns the LSN the file's copy of `pid` is below: the log's next LSN once this open has
    /// written the page, and the end of the log recovery kept before.
    ///
    /// Preconditions: the caller holds a latch on `pid` that no write-out holds with, or judges
    /// again under one.
    pub(crate) fn file_end(&self, pid: Pid) -> Lsn {
        let end = {
            let m = self.own_meta();
            m.table.file_end(pid, m.recovered)
        };
        end.unwrap_or_else(|| self.log().next_lsn())
    }

    /// Returns the page `pid` as the file holds it, verified.
    pub(crate) fn read_disk(&self, pid: Pid) -> Result<(AlignedBuf, Verdict), Damage> {
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
    pub(crate) fn make_room(&self, pid: Pid) {
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
    pub(crate) fn write_out(&self, pid: Pid, bytes: &[u8]) {
        let staged = self.staged(pid, bytes);
        let offset = pid.0 * self.page_size as u64;
        self.io(|d| d.write_all(vec![(self.data, offset, staged)]));
        let mut m = self.own_meta();
        m.table.written_out(Wrote(pid));
        m.records.replaced(pid);
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
        let mut log = self.log();
        let mut last = log.next_lsn();
        let mut m = self.own_meta();
        for pid in &allocated {
            m.blanked.remove(pid);
        }
        for (g, edits) in guards.iter_mut().zip(edits) {
            let pid = g.pid();
            let start = matches!(g.kind, Kind::Overwriting) || allocated.contains(&pid);
            if edits.is_empty() && !start {
                continue;
            }
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
            } else if m.table.record_kind(pid, log.last_checkpoint()) == Logged::Change {
                Record::Change { pid, prev, edits }
            } else {
                Record::Image {
                    pid,
                    prev,
                    page: g.to_vec(),
                }
            };
            let kind = if let Record::Image { prev, .. } = record {
                let met = if prev == Lsn::UNCHAINED {
                    Met::Start
                } else {
                    Met::Image
                };
                m.records.meet(pid, met);
                Logged::Image
            } else {
                Logged::Change
            };
            log.append(&record);
            m.table.logged(pid, lsn, kind);
            m.gate.raise(pid, lsn);
            if let Some(map) = m.map.as_mut()
                && map.root == pid
            {
                map.lsn = lsn;
            }
            last = lsn;
        }
        if !freed.is_empty() {
            let whole = self.is_quarantined(Quarantine::Database);
            for &pid in freed {
                last = m.records.free(&mut log, pid, whole).unwrap_or(last);
            }
        }
        if let Some(Record::MapRoot { root, height }) = &tail {
            let lsn = guards
                .iter()
                .find(|g| g.pid() == *root)
                .map(|g| page::lsn(g));
            m.map = Some(MapRoot {
                root: *root,
                lsn: lsn.expect("a map root is logged with its page"),
                height: *height,
            });
        }
        tail.map_or(last, |r| log.append(&r))
    }

    /// Apply `changes` to their pages and log them, every page or none.
    ///
    /// Returns the LSN of the last record logged. Fails, changing nothing, if a page is damaged.
    ///
    /// Panics if a page appears twice, if the pool cannot hold every page at once, or if an edit
    /// writes the page's LSN or checksum or past its end.
    pub fn change(&self, changes: &[(Pid, Vec<Edit>)]) -> Result<Lsn, PageError> {
        let pids: Vec<Pid> = changes.iter().map(|(p, _)| *p).collect();
        let edits: Vec<Vec<Edit>> = changes.iter().map(|(_, e)| e.clone()).collect();
        self.with_latched(&pids, |pager, guards| pager.apply(guards, edits, None))
    }

    /// Modify `pids` together through `f`, which sees a copy of every page at once, and log what
    /// changed.
    ///
    /// Returns the LSN of the last record logged. Fails, calling nothing, if a page is damaged.
    ///
    /// Panics if `f` changes a page's LSN or checksum, or as [`Pager::change`] does.
    pub fn update<F>(&self, pids: &[Pid], f: F) -> Result<Lsn, PageError>
    where
        F: FnOnce(&mut [&mut [u8]]),
    {
        self.with_latched(pids, |pager, guards| {
            let edits = edits_by(guards.iter().map(|g| &***g), f);
            pager.apply(guards, edits, None)
        })
    }

    /// Change several pages under their exclusive latches through `f`, which sees a copy of
    /// each, logging every record in one batch.
    pub fn update_all<F>(&self, guards: &mut [&mut PageExclusive<'_, R>], f: F) -> Lsn
    where
        F: FnOnce(&mut [&mut [u8]]),
    {
        let edits = edits_by(guards.iter().map(|g| &***g), f);
        self.apply(guards, edits, None)
    }

    /// Change several pages under their exclusive latches through `f`, as
    /// [`Pager::update_all`] does, and log the commit record of transaction `xid` after the page
    /// records, in the same batch.
    ///
    /// Returns the LSN of the commit record.
    pub fn commit_all<F>(&self, xid: u64, guards: &mut [&mut PageExclusive<'_, R>], f: F) -> Lsn
    where
        F: FnOnce(&mut [&mut [u8]]),
    {
        let edits = edits_by(guards.iter().map(|g| &***g), f);
        self.apply(guards, edits, Some(self.commit_record(xid)))
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

    fn close_files(&self) {
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

/// Returns the edits `f` makes to copies of `pages`.
pub(super) fn edits_by<'p, F>(pages: impl Iterator<Item = &'p [u8]>, f: F) -> Vec<Vec<Edit>>
where
    F: FnOnce(&mut [&mut [u8]]),
{
    let originals: Vec<&[u8]> = pages.collect();
    let mut copies: Vec<Vec<u8>> = originals.iter().map(|p| p.to_vec()).collect();
    let mut views: Vec<&mut [u8]> = copies.iter_mut().map(Vec::as_mut_slice).collect();
    f(&mut views);
    originals
        .iter()
        .zip(&copies)
        .map(|(old, new)| diff(old, new))
        .collect()
}
