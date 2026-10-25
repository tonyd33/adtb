use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use crate::runtime::{AlignedBuf, FileId, IoError, Runtime};

use super::block::LogFinding;
use super::driver::Driver;
use super::header::{self, FileHeader, REGION};
use super::log::Log;
use super::page::{self, FIRST_EDITABLE, Verdict, valid_page_size};
use super::record::{Edit, Record};
use super::{Lsn, Pid};

#[derive(Clone, Debug)]
pub struct Options {
    /// The data file's path; the log's segments live in the directory `{name}-log`.
    pub name: String,
    pub page_size: u32,
    /// The device's logical block size, which it writes atomically; the log's block size.
    pub log_block: u32,
    pub segment_size: u64,
    pub pool_frames: usize,
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

#[derive(Debug)]
pub(crate) struct Frame {
    pub(crate) page: AlignedBuf,
    /// Changed since it was last written.
    pub(crate) dirty: bool,
    used: u64,
}

/// The data file, its buffer pool and its redo log.
///
/// Invariants:
///
/// - A frame is written only once the log is durable past its page LSN.
/// - A page is in the dirty page table from the change that dirties its clean frame, which logs a
///   full-page image, until a data file force covers a write of it with no change since.
pub struct Pager<R: Runtime> {
    pub(crate) drv: Driver<R>,
    pub(crate) name: String,
    pub(crate) header: FileHeader,
    /// The header copy holding the highest sequence number.
    copy: u64,
    pub(crate) data: FileId,
    pub(crate) page_size: usize,
    pool_frames: usize,
    pub(crate) frames: BTreeMap<Pid, Frame>,
    tick: u64,
    pub(crate) dirty: BTreeMap<Pid, Lsn>,
    /// The LSN of every page whose frame was dropped, checked when it is read back.
    evicted: BTreeMap<Pid, Lsn>,
    pub(crate) damaged: BTreeMap<Pid, Damage>,
    pub(crate) log: Log,
    /// No bitmap group below this one has a free page.
    pub(crate) alloc_group: u64,
}

impl<R: Runtime> std::fmt::Debug for Pager<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pager")
            .field("header", &self.header)
            .field("frames", &self.frames.len())
            .field("dirty", &self.dirty.len())
            .field("damaged", &self.damaged)
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
    assert!(o.pool_frames > 0, "the pool holds at least one frame");
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
            drv: Driver::new(rt),
            name: o.name.clone(),
            page_size: header.page_size as usize,
            header,
            copy,
            data,
            pool_frames: o.pool_frames,
            frames: BTreeMap::new(),
            tick: 0,
            dirty: BTreeMap::new(),
            evicted: BTreeMap::new(),
            damaged: BTreeMap::new(),
            log,
            alloc_group: 0,
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
            clog_bound: 0,
            epoch: 1,
            encryption: 0,
        };
        let mut pager = Pager::assemble(rt, options, header, 1, data);
        pager.drv = drv;
        pager.checkpoint();
        pager.write_header();
        Ok(pager)
    }

    /// Open an existing database and recover it from its log.
    pub fn open(rt: R, options: &Options) -> Result<(Pager<R>, super::Recovery), OpenError> {
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
        let (copy, header) = header::choose(first, second).ok_or(OpenError::NoHeader)?;
        let mut pager = Pager::assemble(rt, options, header, copy, data);
        pager.drv = drv;
        pager.header.atomic_page_writes = options.atomic_page_writes;
        let recovery = pager.recover()?;
        Ok((pager, recovery))
    }

    pub fn page_size(&self) -> usize {
        self.page_size
    }

    /// Returns the pages refused so far, each with what is wrong with it.
    pub fn damaged(&self) -> &BTreeMap<Pid, Damage> {
        &self.damaged
    }

    /// Returns the page `pid`, reading and verifying it if it is not in the pool.
    ///
    /// A page never written reads as zeros with LSN 0.
    pub fn read(&mut self, pid: Pid) -> Result<&[u8], PageError> {
        self.fetch(pid, &BTreeSet::new())?;
        Ok(&self.frames[&pid].page)
    }

    /// Apply `changes` to their pages and log them, every page or none.
    ///
    /// Returns the LSN of the last record logged. Fails, changing nothing, if a page is damaged.
    ///
    /// Panics if a page appears twice, if the pool cannot hold every page at once, or if an edit
    /// writes the page's LSN or checksum or past its end.
    pub fn change(&mut self, changes: &[(Pid, Vec<Edit>)]) -> Result<Lsn, PageError> {
        let pinned: BTreeSet<Pid> = changes.iter().map(|(p, _)| *p).collect();
        assert_eq!(
            pinned.len(),
            changes.len(),
            "a page appears twice in one change"
        );
        assert!(
            pinned.len() <= self.pool_frames,
            "a change needs more frames than the pool has"
        );
        assert!(
            pinned.iter().all(|p| *p >= Pid::FIRST_DATA),
            "a change writes a file header page"
        );
        for &pid in &pinned {
            self.fetch(pid, &pinned)?;
        }
        let mut last = self.log.next_lsn();
        for (pid, edits) in changes {
            last = self.apply(*pid, edits);
        }
        Ok(last)
    }

    fn apply(&mut self, pid: Pid, edits: &[Edit]) -> Lsn {
        let size = self.page_size;
        let frame = self.frames.get_mut(&pid).expect("a changed page is pinned");
        for e in edits {
            let at = e.offset as usize;
            assert!(
                at >= FIRST_EDITABLE && at + e.bytes.len() <= size,
                "edit of {} bytes at {at} is outside the page's contents",
                e.bytes.len()
            );
            frame.page[at..at + e.bytes.len()].copy_from_slice(&e.bytes);
        }
        let prev = page::lsn(&frame.page);
        let lsn = self.log.next_lsn();
        page::set_lsn(&mut frame.page, lsn);
        let record = if self.dirty.contains_key(&pid) || self.header.atomic_page_writes {
            Record::Change {
                pid,
                prev,
                edits: edits.to_vec(),
            }
        } else {
            Record::Image {
                pid,
                prev,
                page: frame.page.to_vec(),
            }
        };
        frame.dirty = true;
        let at = self.log.append(&record);
        debug_assert_eq!(lsn, at);
        self.dirty.entry(pid).or_insert(lsn);
        lsn
    }

    /// Log the commit of transaction `xid`; it is durable once the log is flushed past it.
    pub fn commit(&mut self, xid: u64) -> Lsn {
        let wall = self.drv.rt.wall();
        let wall_nanos = wall
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
        self.log.append(&Record::Commit { xid, wall_nanos })
    }

    pub fn abort(&mut self, xid: u64) -> Lsn {
        self.log.append(&Record::Abort { xid })
    }

    /// Log that transaction ids below `next` are reserved.
    pub fn reserve_xids(&mut self, next: u64) -> Lsn {
        self.log.append(&Record::XidAllocation { next })
    }

    /// Returns the LSN below which every record is durable.
    pub fn durable(&self) -> Lsn {
        self.log.durable()
    }

    /// Write and force every record appended so far as one batch.
    pub fn flush_log(&mut self) {
        self.log.flush(&mut self.drv);
    }

    /// Write up to `max` dirty frames, oldest first by the LSN that dirtied them, and force the
    /// data file. Returns the number written.
    pub fn flush_pages(&mut self, max: usize) -> usize {
        let mut oldest: Vec<(Lsn, Pid)> = self
            .dirty
            .iter()
            .filter(|(p, _)| self.frames.get(p).is_some_and(|f| f.dirty))
            .map(|(p, l)| (*l, *p))
            .collect();
        oldest.sort_unstable();
        let pids: Vec<Pid> = oldest.into_iter().take(max).map(|(_, p)| p).collect();
        self.write_pages(&pids);
        self.sync_data();
        pids.len()
    }

    /// Force the data file and drop from the dirty page table every page with no change since its
    /// last write.
    pub(crate) fn sync_data(&mut self) {
        if self.dirty.is_empty() {
            return;
        }
        self.drv.sync_all(&[self.data]);
        let frames = &self.frames;
        self.dirty
            .retain(|p, _| frames.get(p).is_some_and(|f| f.dirty));
    }

    /// Log a fuzzy checkpoint of the dirty page table, force it, and point the file header at it.
    pub fn checkpoint(&mut self) {
        let lsn = self.log.next_lsn();
        let redo = self.dirty.values().min().copied().unwrap_or(lsn);
        let dirty = self.dirty.iter().map(|(p, l)| (*p, *l)).collect();
        let at = self.log.append(&Record::Checkpoint { redo, dirty });
        debug_assert_eq!(lsn, at);
        self.flush_log();
        self.header.checkpoint = at;
        self.write_header();
    }

    pub(crate) fn write_header(&mut self) {
        self.header.seq += 1;
        let copy = 1 - self.copy;
        let bytes = self.header.encode(copy);
        let offset = copy * self.page_size as u64;
        self.drv
            .write_all(vec![(self.data, offset, AlignedBuf::from_slice(&bytes))]);
        self.drv.sync_all(&[self.data]);
        self.copy = copy;
    }

    pub(crate) fn write_pages(&mut self, pids: &[Pid]) {
        let Some(newest) = pids.iter().map(|p| page::lsn(&self.frames[p].page)).max() else {
            return;
        };
        if newest >= self.log.durable() {
            self.flush_log();
        }
        let size = self.page_size as u64;
        let writes = pids
            .iter()
            .map(|&pid| {
                let mut staged = AlignedBuf::from_slice(&self.frames[&pid].page);
                page::seal(pid, &mut staged);
                (self.data, pid.0 * size, staged)
            })
            .collect();
        self.drv.write_all(writes);
        for pid in pids {
            self.frames
                .get_mut(pid)
                .expect("a written page is in the pool")
                .dirty = false;
        }
    }

    pub(crate) fn damage(&mut self, pid: Pid, damage: Damage) -> PageError {
        self.damaged.insert(pid, damage);
        self.frames.remove(&pid);
        PageError { pid, damage }
    }

    /// Returns the page `pid` as the file holds it, verified.
    pub(crate) fn read_disk(&mut self, pid: Pid) -> Result<(AlignedBuf, Verdict), Damage> {
        let size = self.page_size;
        let buf = self
            .drv
            .read(self.data, pid.0 * size as u64, size)
            .map_err(|_| Damage::Unreadable)?;
        let verdict = page::verify(pid, &buf);
        Ok((buf, verdict))
    }

    pub(crate) fn fetch(&mut self, pid: Pid, pinned: &BTreeSet<Pid>) -> Result<(), PageError> {
        if let Some(&damage) = self.damaged.get(&pid) {
            return Err(PageError { pid, damage });
        }
        self.tick += 1;
        if let Some(f) = self.frames.get_mut(&pid) {
            f.used = self.tick;
            return Ok(());
        }
        self.make_room(pinned);
        let (buf, verdict) = match self.read_disk(pid) {
            Ok(r) => r,
            Err(d) => return Err(self.damage(pid, d)),
        };
        if verdict == Verdict::Corrupt {
            return Err(self.damage(pid, Damage::Checksum));
        }
        let found = page::lsn(&buf);
        if let Some(&expected) = self.evicted.get(&pid)
            && found < expected
        {
            return Err(self.damage(pid, Damage::Stale { expected, found }));
        }
        self.install(pid, buf, false);
        Ok(())
    }

    pub(crate) fn install(&mut self, pid: Pid, page: AlignedBuf, dirty: bool) {
        let used = self.tick;
        self.frames.insert(pid, Frame { page, dirty, used });
    }

    /// Drop frames until one is free, writing a dirty victim first.
    pub(crate) fn make_room(&mut self, pinned: &BTreeSet<Pid>) {
        while self.frames.len() >= self.pool_frames {
            let victim = self
                .frames
                .iter()
                .filter(|(p, _)| !pinned.contains(p))
                .min_by_key(|(_, f)| (f.dirty, f.used))
                .map(|(p, _)| *p)
                .expect("every frame is pinned");
            if self.frames[&victim].dirty {
                self.write_pages(&[victim]);
            }
            let frame = self.frames.remove(&victim).unwrap();
            let lsn = page::lsn(&frame.page);
            if lsn > Lsn(0) {
                self.evicted.insert(victim, lsn);
            }
        }
    }
}
