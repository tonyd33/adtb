use std::collections::{BTreeMap, BTreeSet};
use std::ops::Deref;
use std::sync::MutexGuard;
use std::time::{Duration, SystemTime};

use adtb_sys::pages::{self, PageMemory, PageState};

use crate::runtime::{AlignedBuf, FileId, IoError, Runtime, ThreadId};

use super::alloc::diff;
use super::block::LogFinding;
use super::driver::Driver;
use super::evicted::EvictedLsns;
use super::header::{self, FileHeader, REGION};
use super::log::{Log, Segments};
use super::page::{self, FIRST_EDITABLE, Verdict, valid_page_size};
use super::record::{Edit, Record};
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
    /// The page is older than the log says it must be: a lost write.
    Stale { expected: Lsn, found: Lsn },
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
    /// Neither copy of the file header verifies.
    NoHeader,
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
    /// The dirty page table: each page's `rec_lsn`.
    pub(crate) dirty: BTreeMap<Pid, Lsn>,
    /// Pages changed since they were last written.
    pub(crate) changed: BTreeSet<Pid>,
    /// Pages written since the last force of the data file.
    pub(crate) written: BTreeSet<Pid>,
    /// The LSN of pages the pool dropped, checked when they are read back.
    pub(crate) evicted: EvictedLsns,
    pub(crate) damaged: BTreeMap<Pid, Damage>,
    /// Damaged pages latched blank, whose next change logs an unchained full-page image.
    pub(crate) overwritten: BTreeSet<Pid>,
    /// Resident pages and pages being read in, in clock order.
    resident: Vec<Pid>,
    hand: usize,
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
/// Lock order: page latches, then the flush lock, then the log lock, then the meta lock. The
/// header, allocation and driver locks are taken alone.
///
/// Clones are handles on one pager.
pub struct Pager<R: Runtime>(std::sync::Arc<Shared<R>>);

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
    pub(crate) atomic_page_writes: bool,
    pub(crate) mem: PageMemory,
    drivers: Lock<BTreeMap<ThreadId, Driver<R>>>,
    header: Lock<(FileHeader, u64)>,
    pub(crate) log: Lock<Log>,
    /// Held by the one thread writing and forcing a batch: the group commit leader.
    pub(crate) segments: Lock<Segments>,
    pub(crate) meta: Lock<Meta>,
    /// Held across a search for a free page and across marking pages allocated.
    pub(crate) alloc: Lock<super::alloc::AllocState>,
    pub(crate) marks: Lock<super::quarantine::Marks>,
    /// Whether `marks` holds any mark. Set before a mark shows in `marks`.
    pub(crate) any_mark: std::sync::atomic::AtomicBool,
}

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

/// Slots of the evicted-page LSN table per pool frame.
const EVICTED_PER_FRAME: usize = 4;

pub(super) fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

fn check_options(o: &Options) {
    let block = o.log_block as usize;
    assert!(
        valid_page_size(o.page_size),
        "page size {} is not allowed",
        o.page_size
    );
    assert!(
        block >= 64 && block.is_power_of_two() && AlignedBuf::ALIGN.is_multiple_of(block),
        "log block {block} must be a power of two from 64 that divides {}",
        AlignedBuf::ALIGN
    );
    assert!(
        o.segment_size > 0 && o.segment_size.is_multiple_of(block as u64),
        "segment size must be a positive multiple of the log block"
    );
    assert!(o.threads >= 1, "at least one thread uses the pool");
    let serves = o.pool_frames / LATCHES_PER_THREAD;
    assert!(
        serves >= o.threads,
        "a pool of {} frames serves at most {serves} threads, not {}",
        o.pool_frames,
        o.threads
    );
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
    pager: &'a Pager<R>,
    g: pages::Exclusive<'a>,
}

impl<R: Runtime> std::fmt::Debug for PageExclusive<'_, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.g.fmt(f)
    }
}

impl<R: Runtime> Deref for PageExclusive<'_, R> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.g
    }
}

impl<'a, R: Runtime> PageExclusive<'a, R> {
    pub(crate) fn of(pager: &'a Pager<R>, g: pages::Exclusive<'a>) -> Self {
        PageExclusive { pager, g }
    }

    pub(crate) fn inner(&mut self) -> &mut pages::Exclusive<'a> {
        &mut self.g
    }

    pub fn pid(&self) -> Pid {
        Pid(self.g.pid())
    }

    /// Change the page through `f`, which sees a copy, and log the bytes it changed.
    ///
    /// Returns the LSN of the record, or the LSN the next record gets if nothing changed.
    pub fn update(&mut self, f: impl FnOnce(&mut [u8])) -> Lsn {
        let mut copy = self.g.to_vec();
        f(&mut copy);
        let edits = diff(&self.g, &copy);
        self.change(edits)
    }

    /// Apply `edits` to the page and log them.
    ///
    /// Panics if an edit writes the page's LSN or checksum or past its end.
    pub fn change(&mut self, edits: Vec<Edit>) -> Lsn {
        let pager = self.pager;
        pager.apply(&mut [&mut self.g], vec![edits], None)
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
    pub fn upgrade(self) -> Option<PageExclusive<'a, R>> {
        let pager = self.pager;
        let g = self.o.upgrade()?;
        pager.check_bound();
        Some(PageExclusive { pager, g })
    }
}

impl<R: Runtime> Pager<R> {
    fn assemble(rt: R, o: &Options, header: FileHeader, copy: u64, data: FileId) -> Self {
        let log = Log::new(header.log_block as usize, header.epoch);
        let segments = Segments::new(o.log_dir(), header.log_block as usize, header.segment_size);
        Pager(std::sync::Arc::new(Shared {
            name: o.name.clone(),
            page_size: header.page_size as usize,
            pool_frames: o.pool_frames,
            stall_after: o.stall_after,
            data,
            atomic_page_writes: o.atomic_page_writes,
            mem: PageMemory::reserve(header.page_size as usize, o.max_pages),
            drivers: Lock::new(BTreeMap::new()),
            header: Lock::new((header, copy)),
            log: Lock::new(log),
            segments: Lock::new(segments),
            meta: Lock::new(Meta {
                dirty: BTreeMap::new(),
                changed: BTreeSet::new(),
                written: BTreeSet::new(),
                evicted: EvictedLsns::new(EVICTED_PER_FRAME * o.pool_frames),
                damaged: BTreeMap::new(),
                overwritten: BTreeSet::new(),
                resident: Vec::new(),
                hand: 0,
            }),
            alloc: Lock::new(Default::default()),
            marks: Lock::new(Default::default()),
            any_mark: std::sync::atomic::AtomicBool::new(false),
            rt,
        }))
    }

    /// Create an empty database: its data file, its first log segment and a first checkpoint.
    ///
    /// Panics if `options` breaks a limit of the file format.
    pub fn create(rt: R, options: &Options) -> Result<Pager<R>, OpenError> {
        check_options(options);
        let mut drv = Driver::new(rt.clone());
        let (data, len) = drv
            .open(&options.name, true)
            .map_err(|_| OpenError::NotFound)?;
        if len > 0 {
            drv.close(data);
            return Err(OpenError::Exists);
        }
        drv.sync_dir(parent(&options.name));
        let mut file_id = [0; 16];
        rt.fill(&mut file_id);
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
            epoch: 1,
            encryption: 0,
        };
        let pager = Pager::assemble(rt, options, header, 1, data);
        let made = pager.reserve_marks().and_then(|(new, marks)| {
            pager.format_marks(new, marks)?;
            pager.header.lock(&pager.rt).0.quarantine = marks;
            Ok(())
        });
        if let Err(e) = made {
            pager.close_files();
            return Err(e.into());
        }
        pager.checkpoint();
        pager.write_header(None);
        Ok(pager)
    }

    /// Open an existing database and recover it from its log.
    pub fn open(rt: R, options: &Options) -> Result<(Pager<R>, super::Recovery), OpenError> {
        check_options(options);
        let mut drv = Driver::new(rt.clone());
        let (data, _) = drv.open(&options.name, false).map_err(|e| match e {
            IoError::NotFound => OpenError::NotFound,
            IoError::Full | IoError::Failed => OpenError::NoHeader,
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
            drv.close(data);
            return Err(OpenError::NoHeader);
        };
        header.atomic_page_writes = options.atomic_page_writes;
        let pager = Pager::assemble(rt, options, header, copy, data);
        match pager.recover_and_mark() {
            Ok(recovery) => Ok((pager, recovery)),
            Err(e) => {
                pager.close_files();
                Err(e)
            }
        }
    }

    fn recover_and_mark(&self) -> Result<super::Recovery, OpenError> {
        let recovery = self.recover()?;
        let marks = self.header.lock(&self.rt).0.quarantine;
        if marks == Pid(0) {
            let (new, marks) = self.reserve_marks()?;
            self.format_marks(new, marks)?;
            self.header.lock(&self.rt).0.quarantine = marks;
            self.checkpoint();
        } else {
            self.load_marks(marks)?;
        }
        Ok(recovery)
    }

    pub fn page_size(&self) -> usize {
        self.page_size
    }

    /// Returns the most threads the pool serves: those that may latch pages at once.
    pub fn max_threads(&self) -> usize {
        self.pool_frames / LATCHES_PER_THREAD
    }

    /// Returns the pages refused so far, each with what is wrong with it.
    pub fn damaged(&self) -> BTreeMap<Pid, Damage> {
        self.meta().damaged.clone()
    }

    pub(crate) fn meta(&self) -> MutexGuard<'_, Meta> {
        self.meta.lock(&self.rt)
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
        match self.meta().damaged.get(&pid) {
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
            if let Some(g) = self.resident(pid)? {
                return Ok(PageShared { g: g.share() });
            }
        }
    }

    /// Returns the exclusive latch on page `pid`, reading and verifying it if it is not resident.
    pub fn exclusive(&self, pid: Pid) -> Result<PageExclusive<'_, R>, PageError> {
        assert!(pid >= Pid::FIRST_DATA, "a change writes a file header page");
        loop {
            if let Some(g) = self.resident(pid)? {
                return Ok(PageExclusive { pager: self, g });
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

    /// Panic, in debug builds and when `latch-checks` is on, if the calling thread latches more
    /// than [`LATCHES_PER_THREAD`] pages.
    fn check_bound(&self) {
        let held = self.mem.latched_here();
        assert!(
            held <= LATCHES_PER_THREAD,
            "this thread latches {held} pages, more than {LATCHES_PER_THREAD}"
        );
    }

    /// Returns the exclusive latch on `pid` with the page resident, or `None` if it was found
    /// damaged by another thread meanwhile and the caller should look again.
    fn resident(&self, pid: Pid) -> Result<Option<pages::Exclusive<'_>>, PageError> {
        self.refused(pid)?;
        let mut g = self.mem.lock_exclusive(pid.0, &self.rt);
        self.check_bound();
        if g.resident() {
            return Ok(Some(g));
        }
        if self.meta().damaged.contains_key(&pid) {
            return Ok(None);
        }
        self.make_room(pid);
        let read = self.read_disk(pid);
        let damage = match &read {
            Err(d) => Some(*d),
            Ok((_, Verdict::Corrupt)) => Some(Damage::Checksum),
            Ok((buf, _)) => {
                let found = page::lsn(buf);
                self.meta()
                    .evicted
                    .get(pid)
                    .filter(|&expected| found < expected)
                    .map(|expected| Damage::Stale { expected, found })
            }
        };
        if let Some(damage) = damage {
            drop(g);
            return Err(self.damage(pid, damage));
        }
        let (buf, _) = read.unwrap();
        g.write(0, &buf);
        Ok(Some(g))
    }

    /// Refuse `pid` from now on, dropping its memory unwritten.
    ///
    /// Preconditions: the caller holds no latch on `pid`.
    #[doc(hidden)]
    pub fn damage(&self, pid: Pid, damage: Damage) -> PageError {
        let g = self.mem.lock_exclusive(pid.0, &self.rt);
        if g.resident() {
            g.evict();
        } else {
            drop(g);
        }
        let mut m = self.meta();
        m.damaged.insert(pid, damage);
        m.changed.remove(&pid);
        m.dirty.remove(&pid);
        drop(m);
        self.unreserve(pid);
        PageError { pid, damage }
    }

    /// Stop counting `pid` as resident.
    pub(crate) fn unreserve(&self, pid: Pid) {
        let mut m = self.meta();
        if let Some(i) = m.resident.iter().position(|p| *p == pid) {
            m.resident.swap_remove(i);
        }
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
    pub(crate) fn make_room(&self, pid: Pid) {
        let mut since = None;
        loop {
            let victim = {
                let mut m = self.meta();
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
        if self.meta().changed.contains(&pid) {
            self.write_out(pid, &g);
        }
        let lsn = page::lsn(&g);
        if lsn > Lsn(0) {
            self.meta().evicted.insert(pid, lsn);
        }
        g.evict();
    }

    /// Write `bytes`, page `pid` under a latch the caller holds, to the file.
    pub(crate) fn write_out(&self, pid: Pid, bytes: &[u8]) {
        self.wait_durable(page::lsn(bytes));
        let mut staged = AlignedBuf::from_slice(bytes);
        page::seal(pid, &mut staged);
        let offset = pid.0 * self.page_size as u64;
        self.io(|d| d.write_all(vec![(self.data, offset, staged)]));
        let mut m = self.meta();
        m.changed.remove(&pid);
        m.written.insert(pid);
    }

    /// Apply `edits[i]` to the page under `guards[i]` and log them, one record per page with
    /// edits, then `tail` if given, all in one batch.
    ///
    /// Returns the LSN of `tail`, or else of the last page record.
    pub(crate) fn apply(
        &self,
        guards: &mut [&mut pages::Exclusive<'_>],
        edits: Vec<Vec<Edit>>,
        tail: Option<Record>,
    ) -> Lsn {
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
        let mut m = self.meta();
        for (g, edits) in guards.iter_mut().zip(edits) {
            if edits.is_empty() {
                continue;
            }
            let pid = Pid(g.pid());
            let prev = page::lsn(g);
            let lsn = log.next_lsn();
            for e in &edits {
                g.write(e.offset as usize, &e.bytes);
            }
            let record = if m.overwritten.remove(&pid) {
                Record::Image {
                    pid,
                    prev: Lsn::UNCHAINED,
                    page: page::with_lsn(g, lsn),
                }
            } else if m.dirty.contains_key(&pid) || self.atomic_page_writes {
                Record::Change { pid, prev, edits }
            } else {
                Record::Image {
                    pid,
                    prev,
                    page: page::with_lsn(g, lsn),
                }
            };
            let (at, field) = page::lsn_field(lsn);
            g.write(at, &field);
            log.append(&record);
            m.dirty.entry(pid).or_insert(lsn);
            m.changed.insert(pid);
            last = lsn;
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
        let mut inner: Vec<&mut pages::Exclusive<'_>> =
            guards.iter_mut().map(|g| &mut g.g).collect();
        self.apply(&mut inner, edits, None)
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
        let mut inner: Vec<&mut pages::Exclusive<'_>> =
            guards.iter_mut().map(|g| &mut g.g).collect();
        self.apply(&mut inner, edits, Some(self.commit_record(xid)))
    }

    fn commit_record(&self, xid: u64) -> Record {
        let wall_nanos = self
            .rt
            .wall()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
        Record::Commit { xid, wall_nanos }
    }

    /// Run `f` with exclusive latches on `pids`, taken in page order, passing them in the order
    /// given.
    pub(crate) fn with_latched<'s, T>(
        &'s self,
        pids: &[Pid],
        f: impl FnOnce(&'s Self, &mut [&mut pages::Exclusive<'s>]) -> T,
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
        let mut held: BTreeMap<Pid, pages::Exclusive<'s>> = BTreeMap::new();
        for &pid in &distinct {
            held.insert(pid, self.exclusive(pid)?.g);
        }
        let mut ordered: Vec<pages::Exclusive<'s>> =
            pids.iter().map(|p| held.remove(p).unwrap()).collect();
        let mut refs: Vec<&mut pages::Exclusive<'s>> = ordered.iter_mut().collect();
        Ok(f(self, &mut refs))
    }

    /// Change `pids` through `f` as [`Pager::update`] does, and log the commit record of
    /// transaction `xid` after the page records, in the same batch.
    ///
    /// Returns the LSN of the commit record; the commit is durable once the log is flushed past
    /// it. Fails, logging nothing, if a page is damaged.
    pub fn commit<F>(&self, xid: u64, pids: &[Pid], f: F) -> Result<Lsn, PageError>
    where
        F: FnOnce(&mut [&mut [u8]]),
    {
        self.resolve(self.commit_record(xid), pids, f)
    }

    /// Commit as [`Pager::commit`] does, the commit record recording `wall_nanos`.
    pub fn commit_at<F>(
        &self,
        xid: u64,
        wall_nanos: u64,
        pids: &[Pid],
        f: F,
    ) -> Result<Lsn, PageError>
    where
        F: FnOnce(&mut [&mut [u8]]),
    {
        self.resolve(Record::Commit { xid, wall_nanos }, pids, f)
    }

    /// Change `pids` through `f` and log the abort record of transaction `xid`, as
    /// [`Pager::commit`] does.
    pub fn abort<F>(&self, xid: u64, pids: &[Pid], f: F) -> Result<Lsn, PageError>
    where
        F: FnOnce(&mut [&mut [u8]]),
    {
        self.resolve(Record::Abort { xid }, pids, f)
    }

    fn resolve<F>(&self, record: Record, pids: &[Pid], f: F) -> Result<Lsn, PageError>
    where
        F: FnOnce(&mut [&mut [u8]]),
    {
        self.with_latched(pids, |pager, guards| {
            let edits = edits_by(guards.iter().map(|g| &***g), f);
            pager.apply(guards, edits, Some(record))
        })
    }

    /// Log a safe snapshot marker: `snapshot`, as the transactions layer encodes it, covering
    /// every commit at or before `at`. Returns the marker's LSN; the marker is durable once the
    /// log is flushed past it.
    pub fn log_safe_snapshot(&self, at: Position, snapshot: &[u8]) -> Lsn {
        self.log().append(&Record::SafeSnapshot {
            at,
            snapshot: snapshot.to_vec(),
        })
    }

    /// Log that transaction ids below `next` are reserved.
    pub fn reserve_xids(&self, next: u64) -> Lsn {
        self.log().append(&Record::XidAllocation { next })
    }

    /// Returns the LSN below which every record is durable.
    pub fn durable(&self) -> Lsn {
        self.log().durable()
    }

    /// Write and force every record appended so far.
    ///
    /// Postcondition: every record appended before the call is durable.
    pub fn flush_log(&self) {
        let lsn = self.log().next_lsn();
        self.force_below(lsn);
    }

    /// Returns the LSN the next record appended takes.
    pub fn appended(&self) -> Lsn {
        self.log().next_lsn()
    }

    /// Write and force every record below `end`.
    pub fn flush_below(&self, end: Lsn) {
        self.force_below(end);
    }

    /// Wait until the record at `lsn` is durable, and return its commit position.
    ///
    /// The first waiter that finds no batch being forced leads: it takes every record appended
    /// so far as one batch, writes and forces it. Waiters whose record that batch holds return
    /// with it; the rest wait for the next leader.
    ///
    /// Panics if no record was appended at `lsn`.
    pub fn wait_durable(&self, lsn: Lsn) -> Position {
        assert!(
            lsn < self.log().next_lsn(),
            "no record was appended at {lsn:?}"
        );
        self.force_below(Lsn(lsn.0 + 1));
        self.position(lsn)
    }

    /// Returns the commit position of the record at `lsn`.
    pub fn position(&self, lsn: Lsn) -> Position {
        Position {
            epoch: self.log().epoch,
            lsn,
        }
    }

    /// Returns the number of log batches forced since the pager opened.
    pub fn batches(&self) -> u64 {
        self.log().batches()
    }

    /// Return once every record below `lsn` is durable, leading a flush when no other thread
    /// is. A waiter rechecks durability between its attempts to lead.
    fn force_below(&self, lsn: Lsn) {
        loop {
            if self.durable() >= lsn {
                return;
            }
            let Some(mut segments) = self.segments.try_lock() else {
                self.rt.yield_now();
                continue;
            };
            if self.durable() >= lsn {
                return;
            }
            let Some(batch) = self.log().take() else {
                return;
            };
            self.io(|d| segments.write(d, &batch));
            self.log().forced(&batch);
        }
    }

    /// Write up to `max` changed pages, oldest first by the LSN that dirtied them, and force the
    /// data file. Returns the number written.
    pub fn flush_pages(&self, max: usize) -> usize {
        let oldest: Vec<Pid> = {
            let m = self.meta();
            let mut by_lsn: Vec<(Lsn, Pid)> = m
                .dirty
                .iter()
                .filter(|(p, _)| m.changed.contains(p))
                .map(|(p, l)| (*l, *p))
                .collect();
            by_lsn.sort_unstable();
            by_lsn.into_iter().take(max).map(|(_, p)| p).collect()
        };
        let mut n = 0;
        for pid in oldest {
            let Some(g) = self.mem.lock_shared(pid.0, &self.rt) else {
                continue;
            };
            if self.meta().changed.contains(&pid) {
                self.write_out(pid, &g);
                n += 1;
            }
        }
        self.sync_data();
        n
    }

    /// Force the data file and drop from the dirty page table every page written before the force
    /// with no change since.
    pub(crate) fn sync_data(&self) {
        let covered = std::mem::take(&mut self.meta().written);
        self.io(|d| d.sync_all(&[self.data]));
        let mut m = self.meta();
        for pid in covered {
            if !m.changed.contains(&pid) {
                m.dirty.remove(&pid);
            }
        }
    }

    /// Log a fuzzy checkpoint of the dirty page table, force it, and point the file header at it.
    pub fn checkpoint(&self) {
        let at = {
            let mut log = self.log();
            let lsn = log.next_lsn();
            let dirty: Vec<(Pid, Lsn)> = self.meta().dirty.iter().map(|(p, l)| (*p, *l)).collect();
            let redo = dirty.iter().map(|(_, l)| *l).min().unwrap_or(lsn);
            log.append(&Record::Checkpoint { redo, dirty })
        };
        self.flush_log();
        self.write_header(Some(at));
    }

    /// Write the next header copy, pointing at the checkpoint at `checkpoint` unless the header
    /// already names a later one.
    fn write_header(&self, checkpoint: Option<Lsn>) {
        let mut h = self.header.lock(&self.rt);
        let (header, copy) = &mut *h;
        if let Some(at) = checkpoint {
            if at < header.checkpoint {
                return;
            }
            header.checkpoint = at;
        }
        header.seq += 1;
        *copy = 1 - *copy;
        let bytes = header.encode(*copy);
        let offset = *copy * self.page_size as u64;
        self.io(|d| {
            d.write_all(vec![(self.data, offset, AlignedBuf::from_slice(&bytes))]);
            d.sync_all(&[self.data]);
        });
    }

    /// Write every changed page, take a checkpoint, and close the data file and every log segment.
    ///
    /// Preconditions: every other call on the pager has returned, and none follows.
    pub fn close(&self) {
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

    /// Returns the page the file header names as the catalog's root, `Pid(0)` before one is set.
    pub fn catalog_root(&self) -> Pid {
        self.header.lock(&self.rt).0.catalog_root
    }

    /// Force the log, then write and force a header copy naming `root` as the catalog's root.
    pub fn set_catalog_root(&self, root: Pid) {
        self.flush_log();
        self.header.lock(&self.rt).0.catalog_root = root;
        self.write_header(None);
    }

    /// Returns the wall-clock time of the runtime the pager runs on.
    pub fn wall(&self) -> SystemTime {
        self.rt.wall()
    }

    pub(crate) fn checkpoint_lsn(&self) -> Lsn {
        self.header.lock(&self.rt).0.checkpoint
    }
}

/// Returns the edits `f` makes to copies of `pages`.
fn edits_by<'p, F>(pages: impl Iterator<Item = &'p [u8]>, f: F) -> Vec<Vec<Edit>>
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
