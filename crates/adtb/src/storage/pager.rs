use std::collections::{BTreeMap, BTreeSet};
use std::ops::Deref;
use std::sync::MutexGuard;
use std::time::SystemTime;

use adtb_sys::pages::{self, PageMemory, PageState};

use crate::runtime::{AlignedBuf, FileId, IoError, Runtime, ThreadId};

use super::alloc::diff;
use super::block::LogFinding;
use super::driver::Driver;
use super::header::{self, FileHeader, REGION};
use super::log::Log;
use super::page::{self, FIRST_EDITABLE, Verdict, valid_page_size};
use super::record::{Edit, Record};
use super::{Lsn, Pid};
use adtb_sys::lock::Lock;

#[derive(Clone, Debug)]
pub struct Options {
    /// The data file's path; the log's segments live in the directory `{name}-log`.
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
}

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
        }
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
}

/// What the pool knows of its pages beyond their memory and state words.
#[derive(Debug, Default)]
pub(crate) struct Meta {
    /// The dirty page table: each page's `rec_lsn`.
    pub(crate) dirty: BTreeMap<Pid, Lsn>,
    /// Pages changed since they were last written.
    pub(crate) changed: BTreeSet<Pid>,
    /// Pages written since the last force of the data file.
    pub(crate) written: BTreeSet<Pid>,
    /// The LSN of every page the pool dropped, checked when it is read back.
    pub(crate) evicted: BTreeMap<Pid, Lsn>,
    pub(crate) damaged: BTreeMap<Pid, Damage>,
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
/// Lock order: page latches, then the log lock, then the meta lock. The header, allocation and
/// driver locks are taken alone.
pub struct Pager<R: Runtime> {
    pub(crate) rt: R,
    pub(crate) name: String,
    pub(crate) page_size: usize,
    pool_frames: usize,
    pub(crate) data: FileId,
    pub(crate) atomic_page_writes: bool,
    pub(crate) mem: PageMemory,
    drivers: Lock<BTreeMap<ThreadId, Driver<R>>>,
    header: Lock<(FileHeader, u64)>,
    pub(crate) log: Lock<Log>,
    pub(crate) meta: Lock<Meta>,
    /// No bitmap group below this one has a free page; held across an allocation.
    pub(crate) alloc_group: Lock<u64>,
}

impl<R: Runtime> std::fmt::Debug for Pager<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pager")
            .field("name", &self.name)
            .field("page_size", &self.page_size)
            .finish_non_exhaustive()
    }
}

fn parent(path: &str) -> &str {
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
    assert!(o.pool_frames >= 2, "the pool holds at least two frames");
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

impl<R: Runtime> PageExclusive<'_, R> {
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
        let mut copy = vec![0; self.pager.page_size];
        self.o.read(0, &mut copy);
        f(&copy)
    }

    pub fn validate(&self) -> bool {
        self.o.validate()
    }

    /// Take the exclusive latch if no exclusive latch was taken since this read began.
    pub fn upgrade(self) -> Option<PageExclusive<'a, R>> {
        let pager = self.pager;
        self.o.upgrade().map(|g| PageExclusive { pager, g })
    }
}

impl<R: Runtime> Pager<R> {
    fn assemble(rt: R, o: &Options, header: FileHeader, copy: u64, data: FileId) -> Self {
        let log = Log::new(
            format!("{}-log", o.name),
            header.log_block as usize,
            header.segment_size,
            header.epoch,
        );
        Pager {
            name: o.name.clone(),
            page_size: header.page_size as usize,
            pool_frames: o.pool_frames,
            data,
            atomic_page_writes: o.atomic_page_writes,
            mem: PageMemory::reserve(header.page_size as usize, o.max_pages),
            drivers: Lock::new(BTreeMap::new()),
            header: Lock::new((header, copy)),
            log: Lock::new(log),
            meta: Lock::new(Meta::default()),
            alloc_group: Lock::new(0),
            rt,
        }
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
            epoch: 1,
            encryption: 0,
        };
        let pager = Pager::assemble(rt, options, header, 1, data);
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
            IoError::Failed => OpenError::NoHeader,
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
        let (copy, mut header) = header::choose(first, second).ok_or(OpenError::NoHeader)?;
        header.atomic_page_writes = options.atomic_page_writes;
        let pager = Pager::assemble(rt, options, header, copy, data);
        let recovery = pager.recover()?;
        Ok((pager, recovery))
    }

    pub fn page_size(&self) -> usize {
        self.page_size
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

    /// Returns the exclusive latch on `pid` with the page resident, or `None` if it was found
    /// damaged by another thread meanwhile and the caller should look again.
    fn resident(&self, pid: Pid) -> Result<Option<pages::Exclusive<'_>>, PageError> {
        self.refused(pid)?;
        let mut g = self.mem.lock_exclusive(pid.0, &self.rt);
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
                    .get(&pid)
                    .filter(|&&expected| found < expected)
                    .map(|&expected| Damage::Stale { expected, found })
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
    pub(crate) fn damage(&self, pid: Pid, damage: Damage) -> PageError {
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
    pub(crate) fn make_room(&self, pid: Pid) {
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
                None => self.rt.yield_now(),
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
        let lsn = page::lsn(bytes);
        if lsn >= self.durable() {
            self.flush_log();
        }
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
        let mut copies = Vec::new();
        for (g, edits) in guards.iter().zip(&edits) {
            let mut copy = g.to_vec();
            for e in edits {
                let at = e.offset as usize;
                assert!(
                    at >= FIRST_EDITABLE && at + e.bytes.len() <= size,
                    "edit of {} bytes at {at} is outside the page's contents",
                    e.bytes.len()
                );
                copy[at..at + e.bytes.len()].copy_from_slice(&e.bytes);
            }
            copies.push(copy);
        }
        let mut log = self.log();
        let mut last = log.next_lsn();
        let mut m = self.meta();
        for ((g, edits), copy) in guards.iter_mut().zip(edits).zip(&mut copies) {
            if edits.is_empty() {
                continue;
            }
            let pid = Pid(g.pid());
            let prev = page::lsn(copy);
            let lsn = log.next_lsn();
            page::set_lsn(copy, lsn);
            let record = if m.dirty.contains_key(&pid) || self.atomic_page_writes {
                Record::Change { pid, prev, edits }
            } else {
                Record::Image {
                    pid,
                    prev,
                    page: copy.clone(),
                }
            };
            log.append(&record);
            m.dirty.entry(pid).or_insert(lsn);
            m.changed.insert(pid);
            g.write(0, copy);
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
    fn with_latched<T>(
        &self,
        pids: &[Pid],
        f: impl FnOnce(&Self, &mut [&mut pages::Exclusive<'_>]) -> T,
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
        let mut held: BTreeMap<Pid, pages::Exclusive<'_>> = BTreeMap::new();
        for &pid in &distinct {
            held.insert(pid, self.exclusive(pid)?.g);
        }
        let mut ordered: Vec<pages::Exclusive<'_>> =
            pids.iter().map(|p| held.remove(p).unwrap()).collect();
        let mut refs: Vec<&mut pages::Exclusive<'_>> = ordered.iter_mut().collect();
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

    /// Log that transaction ids below `next` are reserved.
    pub fn reserve_xids(&self, next: u64) -> Lsn {
        self.log().append(&Record::XidAllocation { next })
    }

    /// Returns the LSN below which every record is durable.
    pub fn durable(&self) -> Lsn {
        self.log().durable()
    }

    /// Write and force every record appended so far, as one batch unless another thread's flush
    /// took some of them first.
    pub fn flush_log(&self) {
        let mut log = self.log();
        self.io(|d| log.flush(d));
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
    let mut views: Vec<&mut [u8]> = copies.iter_mut().map(|c| c.as_mut_slice()).collect();
    f(&mut views);
    originals
        .iter()
        .zip(&copies)
        .map(|(old, new)| diff(old, new))
        .collect()
}
