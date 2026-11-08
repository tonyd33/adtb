use std::collections::BTreeMap;
use std::time::SystemTime;

use crate::runtime::{AlignedBuf, Done, FileId, IoError, Op, Runtime, fatal};

use super::alloc::edits_by;
use super::block::{LogImage, LogSource, Slot, data_lsn, frame};
use super::driver::Driver;
use super::pager::{ArchiveHook, PageError, Pager, parent};
use super::record::Record;
use super::{Lsn, Pid, Position};

/// The redo log's appender: the batch being filled, behind the log lock.
#[derive(Debug)]
pub(crate) struct Log {
    pub(crate) block: usize,
    pub(crate) epoch: u64,
    file_id: [u8; 16],
    /// The first LSN of the batch being filled.
    start: Lsn,
    seq: u64,
    pending: Vec<u8>,
    /// Every record below it is forced.
    durable: Lsn,
    batches: u64,
    /// The redo LSN of the last checkpoint the file header durably names.
    redo: Lsn,
    /// The LSN of the last checkpoint record the file header durably names.
    checkpoint: Lsn,
    /// The log forced past `checkpoint` that makes a checkpoint due, `None` while none falls due.
    every: Option<u64>,
    /// Whether a forced batch has found a checkpoint due since the last one.
    due: bool,
    /// Where this open's log starts, the end of the log recovery kept, until the open's readers of
    /// the log have run; then `None`.
    session: Option<Lsn>,
    /// The LSNs readers of the log hold it from, each with its number of holders.
    pins: BTreeMap<Lsn, usize>,
}

/// A batch taken from the appender, to be written and forced by the one flusher.
#[derive(Debug)]
pub(crate) struct Batch {
    pub(crate) start: Lsn,
    pub(crate) blocks: Vec<u8>,
}

impl Batch {
    pub(crate) fn end(&self) -> Lsn {
        Lsn(self.start.0 + self.blocks.len() as u64)
    }
}

/// The log's segment files, behind the segments lock, which one flusher holds at a time.
#[derive(Debug)]
pub(crate) struct Segments {
    pub(crate) dir: String,
    block: usize,
    segment: u64,
    file_id: [u8; 16],
    files: BTreeMap<u64, FileId>,
    /// The directory exists and its creation is forced.
    made: bool,
    /// The first LSN of the oldest segment present, as last listed or recycled.
    oldest: u64,
}

pub(super) fn segment_name(base: u64) -> String {
    format!("{base:016x}")
}

pub(super) fn parse_segment(path: &str) -> Option<u64> {
    let name = path.rsplit_once('/').map_or(path, |(_, n)| n);
    (name.len() == 16)
        .then(|| u64::from_str_radix(name, 16).ok())
        .flatten()
}

impl Log {
    pub(crate) fn new(block: usize, epoch: u64, file_id: [u8; 16]) -> Log {
        Log {
            block,
            epoch,
            file_id,
            start: Lsn(0),
            seq: 1,
            pending: Vec::new(),
            durable: Lsn(0),
            batches: 0,
            redo: Lsn(0),
            checkpoint: Lsn(0),
            every: None,
            due: false,
            session: Some(Lsn(0)),
            pins: BTreeMap::new(),
        }
    }

    /// Returns the LSN the next appended record gets.
    pub(crate) fn next_lsn(&self) -> Lsn {
        data_lsn(self.start, self.pending.len(), self.block)
    }

    /// Returns the LSN below which every record is forced.
    pub(crate) fn durable(&self) -> Lsn {
        self.durable
    }

    /// Returns the number of batches forced since open.
    pub(crate) fn batches(&self) -> u64 {
        self.batches
    }

    /// Continue the log at `end` with batch number `seq`, everything before it forced.
    pub(crate) fn resume(&mut self, end: Lsn, seq: u64) {
        assert!(self.pending.is_empty());
        self.start = end;
        self.durable = end;
        self.session = Some(end);
        self.seq = seq;
    }

    pub(crate) fn append(&mut self, record: &Record) -> Lsn {
        let lsn = self.next_lsn();
        record.encode(&mut self.pending);
        lsn
    }

    /// Take every record appended so far as the next batch, framed in blocks, and start the batch
    /// after it.
    ///
    /// Preconditions: the caller holds the segments lock, and forces the batch before taking
    /// another.
    pub(crate) fn take(&mut self) -> Option<Batch> {
        if self.pending.is_empty() {
            return None;
        }
        let blocks = frame(
            self.start,
            self.seq,
            self.epoch,
            &self.file_id,
            &self.pending,
            self.block,
        );
        let batch = Batch {
            start: self.start,
            blocks,
        };
        self.start = batch.end();
        self.seq += 1;
        self.pending.clear();
        Some(batch)
    }

    /// Record that `batch` is forced. Returns true when it is the first batch since the last
    /// checkpoint to make the next one due.
    pub(crate) fn forced(&mut self, batch: &Batch) -> bool {
        assert_eq!(self.durable, batch.start, "batches are forced in order");
        self.durable = batch.end();
        self.batches += 1;
        let newly = !self.due && self.checkpoint_due();
        self.due |= newly;
        newly
    }

    /// Returns whether the log forced past the last checkpoint has reached the threshold.
    pub(crate) fn checkpoint_due(&self) -> bool {
        self.every
            .is_some_and(|every| self.durable.0.saturating_sub(self.checkpoint.0) >= every)
    }

    /// Returns the LSN of the last checkpoint record the file header durably names.
    pub(crate) fn last_checkpoint(&self) -> Lsn {
        self.checkpoint
    }

    /// Make a checkpoint due once `every` bytes of log are forced past the last one.
    pub(crate) fn checkpoint_every(&mut self, every: u64) {
        self.every = Some(every);
    }

    /// Record that the file header durably names the checkpoint logged at `at`, whose redo LSN is
    /// `redo`.
    pub(crate) fn checkpointed(&mut self, at: Lsn, redo: Lsn) {
        self.checkpoint = self.checkpoint.max(at);
        self.redo = self.redo.max(redo);
        self.due = false;
    }

    /// Hold the log from `at` until the returned pin is given to a [`LogPin`] and that drops.
    pub(super) fn pin(&mut self, at: Lsn) -> Pinned {
        *self.pins.entry(at).or_default() += 1;
        Pinned(at)
    }

    fn unpin(&mut self, pin: Pinned) {
        let Pinned(at) = pin;
        if let Some(n) = self.pins.get_mut(&at) {
            *n -= 1;
            if *n == 0 {
                self.pins.remove(&at);
            }
        }
    }

    /// Returns the least LSN still needed: by recovery, the durable checkpoint's redo LSN; by a
    /// reader, a pin; until [`Log::release_session`], the start of this open's log.
    pub(crate) fn needed(&self) -> Lsn {
        let kept = self.session.map_or(self.redo, |s| s.min(self.redo));
        self.pins.keys().next().map_or(kept, |&p| p.min(kept))
    }

    /// Stop holding the log from the start of this open, once every reader of it the open found
    /// has run.
    pub(crate) fn release_session(&mut self) {
        self.session = None;
    }

    /// Returns whether the log may be recycled past the start of this open.
    pub(crate) fn session_released(&self) -> bool {
        self.session.is_none()
    }
}

impl Segments {
    pub(crate) fn new(dir: String, block: usize, segment: u64, file_id: [u8; 16]) -> Segments {
        Segments {
            dir,
            block,
            segment,
            file_id,
            files: BTreeMap::new(),
            made: false,
            oldest: 0,
        }
    }

    fn path(&self, base: u64) -> String {
        format!("{}/{}", self.dir, segment_name(base))
    }

    /// Take the log directory for a new file by making it and a first segment in it, so that a
    /// rename of another directory onto it is refused.
    ///
    /// Postconditions:
    /// - Returns true when the directory holds only that segment, never written.
    /// - Returns false when it holds anything else or a restore took it back meanwhile, with what
    ///   it held left as it was.
    pub(crate) fn claim<R: Runtime>(&mut self, drv: &mut Driver<R>) -> bool {
        drv.create_dir(&self.dir);
        drv.sync_dir(parent(&self.dir));
        self.made = true;
        let first = self.path(0);
        let created = match drv.create_new(&first) {
            Ok((file, _)) => {
                drv.close(file);
                true
            }
            Err(IoError::Exists) => false,
            Err(IoError::NotFound) => return false,
            Err(e) => fatal(format!("create {first}"), e),
        };
        let dir = self.dir.clone();
        let entries = match drv.attempt(Op::List { dir }).result {
            Ok(Done::Listed(paths)) => paths,
            Err(IoError::NotFound) => return false,
            Err(e) => fatal(format!("list {}", self.dir), e),
            Ok(other) => unreachable!("list completed as {other:?}"),
        };
        if entries != [first.clone()] {
            if created {
                drv.remove(&first);
                self.force_dir_if_present(drv);
            }
            return false;
        }
        self.force_dir_if_present(drv) && (created || self.never_written(drv, &first))
    }

    /// Force the directory's names. Returns whether it was there.
    fn force_dir_if_present<R: Runtime>(&self, drv: &mut Driver<R>) -> bool {
        let dir = self.dir.clone();
        match drv.attempt(Op::SyncDir { dir }).result {
            Ok(_) => true,
            Err(IoError::NotFound) => false,
            Err(e) => fatal(format!("force {}", self.dir), e),
        }
    }

    /// Returns whether the first block of the segment at `path` is zero or past its end.
    fn never_written<R: Runtime>(&self, drv: &mut Driver<R>, path: &str) -> bool {
        let Ok((file, len)) = drv.open(path, false) else {
            return false;
        };
        let blank = len == 0
            || drv
                .read(file, 0, self.block)
                .is_ok_and(|head| head.iter().all(|&b| b == 0));
        drv.close(file);
        blank
    }

    /// Open the segment starting at `base`, creating and preallocating it if absent.
    fn segment_file<R: Runtime>(&mut self, drv: &mut Driver<R>, base: u64) -> FileId {
        if let Some(&f) = self.files.get(&base) {
            return f;
        }
        if !self.made {
            drv.create_dir(&self.dir);
            drv.sync_dir(parent(&self.dir));
            self.made = true;
        }
        let path = self.path(base);
        let (file, len) = drv
            .open(&path, true)
            .unwrap_or_else(|e| fatal(format!("create {path}"), e));
        if len < self.segment {
            drv.allocate(file, self.segment);
            drv.sync_all(&[file]);
        }
        drv.sync_dir(&self.dir);
        self.files.insert(base, file);
        file
    }

    /// Close every segment file open in this process.
    pub(crate) fn close<R: Runtime>(&mut self, drv: &mut Driver<R>) {
        for file in std::mem::take(&mut self.files).into_values() {
            drv.close(file);
        }
    }

    /// Remove, oldest first, every segment of `bases` that ends at or before `floor`, forcing the
    /// directory after each, so a crash leaves the segments present a run with no hole. Returns
    /// the number removed.
    ///
    /// Precondition: `bases` is ascending.
    pub(crate) fn recycle<R: Runtime>(
        &mut self,
        drv: &mut Driver<R>,
        bases: &[u64],
        floor: u64,
    ) -> usize {
        let mut removed = 0;
        for &base in bases.iter().take_while(|&&b| b + self.segment <= floor) {
            if let Some(file) = self.files.remove(&base) {
                drv.close(file);
            }
            drv.remove(&self.path(base));
            drv.sync_dir(&self.dir);
            self.oldest = base + self.segment;
            removed += 1;
        }
        removed
    }

    /// Returns whether a segment ends at or before `floor`, without listing the directory.
    pub(crate) fn may_recycle(&self, floor: u64) -> bool {
        self.oldest + self.segment <= floor
    }

    /// Write and force the blocks of `batch`.
    pub(crate) fn write<R: Runtime>(&mut self, drv: &mut Driver<R>, batch: &Batch) {
        let bytes = &batch.blocks;
        let mut writes = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let lsn = batch.start.0 + at as u64;
            let base = lsn - lsn % self.segment;
            let room = (base + self.segment - lsn) as usize;
            let n = room.min(bytes.len() - at);
            let file = self.segment_file(drv, base);
            writes.push((file, lsn - base, AlignedBuf::from_slice(&bytes[at..at + n])));
            at += n;
        }
        let files: Vec<FileId> = writes.iter().map(|w| w.0).collect();
        drv.write_all(writes);
        drv.sync_all(&files);
    }

    /// Returns the log as its segment files hold it now, read block by block on its own I/O
    /// queue.
    pub(crate) fn load<R: Runtime>(&mut self, drv: &mut Driver<R>) -> LogImage<FileSource<R>> {
        self.try_load(drv)
            .unwrap_or_else(|(what, e)| fatal(what, e))
    }

    /// Returns what [`load`](Self::load) does, or the path and failure of what it cannot read. A
    /// segment gone between its listing and its opening is not in the log.
    pub(crate) fn try_load<R: Runtime>(
        &mut self,
        drv: &mut Driver<R>,
    ) -> Result<LogImage<FileSource<R>>, (String, IoError)> {
        let listed = drv.attempt(Op::List {
            dir: self.dir.clone(),
        });
        let paths = match listed.result {
            Ok(Done::Listed(paths)) => paths,
            Ok(other) => unreachable!("list completed as {other:?}"),
            Err(IoError::NotFound) => Vec::new(),
            Err(e) => return Err((self.dir.clone(), e)),
        };
        let mut bases: Vec<u64> = paths
            .iter()
            .filter_map(|p| parse_segment(p))
            .filter(|b| b % self.segment == 0)
            .collect();
        bases.sort_unstable();
        if let Some(&first) = bases.first() {
            self.oldest = first;
        }
        let mut files = BTreeMap::new();
        for base in bases {
            let file = match self.files.get(&base) {
                Some(&f) => f,
                None => match drv.open(&self.path(base), false) {
                    Ok((f, _)) => f,
                    Err(IoError::NotFound) => continue,
                    Err(e) => return Err((self.path(base), e)),
                },
            };
            self.files.insert(base, file);
            files.insert(base, file);
        }
        Ok(LogImage {
            block: self.block,
            segment: self.segment,
            bases: files.keys().copied().collect(),
            file_id: self.file_id,
            source: FileSource {
                drv: Driver::new(drv.rt.clone()),
                files,
                segment: self.segment,
                chunk: None,
            },
        })
    }

    /// Zero every nonzero block of the log past `end` and remove later segments, then force both.
    pub(crate) fn clear_tail<R: Runtime, S: LogSource>(
        &mut self,
        drv: &mut Driver<R>,
        img: &mut LogImage<S>,
        end: Lsn,
    ) {
        let base = end.0 - end.0 % self.segment;
        let present = img.bases.contains(&base);
        if present {
            let mut last = None;
            let mut at = end.0;
            while at < base + self.segment {
                if let Slot::Bytes(b) = img.slot(Lsn(at))
                    && b.iter().any(|&x| x != 0)
                {
                    last = Some(at);
                }
                at += self.block as u64;
            }
            let file = self.segment_file(drv, base);
            if let Some(last) = last {
                let from = end.0 - base;
                let to = last + self.block as u64 - base;
                drv.write_all(vec![(file, from, AlignedBuf::zeroed((to - from) as usize))]);
            }
            drv.allocate(file, self.segment);
            drv.sync_all(&[file]);
        }
        let later: Vec<u64> = img.bases.iter().copied().filter(|&b| b > base).collect();
        for b in &later {
            drv.remove(&self.path(*b));
            if let Some(file) = self.files.remove(b) {
                drv.close(file);
            }
        }
        if !later.is_empty() {
            drv.sync_dir(&self.dir);
        }
    }
}

/// The bytes of a log read from its segment files, one chunk at a time.
pub(crate) const CHUNK: usize = 64 * 1024;

/// Reads a log's blocks from its segment files, holding one chunk of one segment at a time.
pub(crate) struct FileSource<R: Runtime> {
    drv: Driver<R>,
    files: BTreeMap<u64, FileId>,
    segment: u64,
    /// The LSN the cached chunk starts at, and its bytes; `None` bytes where the read failed.
    chunk: Option<(u64, Option<AlignedBuf>)>,
}

impl<R: Runtime> FileSource<R> {
    /// Returns the log of the open segment `files`, keyed by their first LSN. The caller closes
    /// them after the image's last read.
    pub(crate) fn image(
        rt: R,
        block: usize,
        segment: u64,
        file_id: [u8; 16],
        files: BTreeMap<u64, FileId>,
    ) -> LogImage<FileSource<R>> {
        LogImage {
            block,
            segment,
            bases: files.keys().copied().collect(),
            file_id,
            source: FileSource {
                drv: Driver::new(rt),
                files,
                segment,
                chunk: None,
            },
        }
    }
}

impl<R: Runtime> std::fmt::Debug for FileSource<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileSource")
            .field("files", &self.files)
            .finish_non_exhaustive()
    }
}

impl<R: Runtime> LogSource for FileSource<R> {
    fn block(&mut self, base: u64, at: Lsn, block: usize) -> Slot {
        let Some(&file) = self.files.get(&base) else {
            return Slot::Missing;
        };
        let chunk = (CHUNK as u64).min(self.segment);
        let start = base + (at.0 - base) / chunk * chunk;
        if self.chunk.as_ref().is_none_or(|(s, _)| *s != start) {
            let len = chunk.min(base + self.segment - start) as usize;
            let bytes = self.drv.read(file, start - base, len).ok();
            self.chunk = Some((start, bytes));
        }
        match &self.chunk {
            Some((_, Some(bytes))) => {
                let off = (at.0 - start) as usize;
                Slot::Bytes(bytes[off..off + block].to_vec())
            }
            _ => Slot::Unreadable,
        }
    }
}

/// A count of a hold on the log, made only by [`Log::pin`] and given back only by the drop of
/// the [`LogPin`] holding it.
#[derive(Debug)]
#[must_use]
pub(super) struct Pinned(Lsn);

/// A hold on the log from an LSN: no recycle removes the segment holding the start of its batch,
/// or any later one, while it lives.
pub(crate) struct LogPin<'p, R: Runtime> {
    pager: &'p Pager<R>,
    /// Taken only by the drop.
    pinned: Option<Pinned>,
}

type PinnedLog<'p, R> = (LogPin<'p, R>, LogImage<FileSource<R>>);

impl<'p, R: Runtime> LogPin<'p, R> {
    /// Returns the hold `pinned` counts on `pager`'s log, given back when it drops.
    pub(super) fn new(pager: &'p Pager<R>, pinned: Pinned) -> Self {
        LogPin {
            pager,
            pinned: Some(pinned),
        }
    }

    /// Returns the LSN the log is held from.
    pub(super) fn at(&self) -> Lsn {
        self.pinned.as_ref().expect("held until the drop").0
    }
}

impl<R: Runtime> Drop for LogPin<'_, R> {
    fn drop(&mut self) {
        if let Some(pinned) = self.pinned.take() {
            self.pager.log().unpin(pinned);
        }
    }
}

impl<R: Runtime> Pager<R> {
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
    pub(crate) fn appended(&self) -> Lsn {
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
    /// Panic if no record was appended at `lsn`.
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
    pub(super) fn force_below(&self, lsn: Lsn) {
        loop {
            if self.durable() >= lsn {
                return;
            }
            let Some(mut segments) = self.segments().try_lock() else {
                self.rt().yield_now();
                continue;
            };
            if self.durable() >= lsn {
                return;
            }
            let Some(batch) = self.log().take() else {
                return;
            };
            self.io(|d| segments.write(d, &batch));
            let due = self.log().forced(&batch);
            drop(segments);
            if due {
                self.wake_checkpointer();
            }
        }
    }

    /// Bound what [`Pager::recycle_log`] removes by what `hook` says the archive holds.
    pub fn set_archive(&self, hook: ArchiveHook) {
        *self.archive_hook().lock(self.rt()) = Some(hook);
    }

    /// Returns the log as its segments hold it now, pinned from its first segment while the pin
    /// lives, or the path and failure of what it cannot read.
    pub(crate) fn load_log(&self) -> Result<PinnedLog<'_, R>, (String, IoError)> {
        let mut segments = self.segments().lock(self.rt());
        let img = self.io(|d| segments.try_load(d))?;
        let at = img.first().unwrap_or(Lsn(0));
        let pinned = self.log().pin(at);
        Ok((LogPin::new(self, pinned), img))
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

    pub(super) fn commit_record(&self, xid: u64) -> Record {
        let wall_nanos = self
            .rt()
            .wall()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
        Record::Commit { xid, wall_nanos }
    }
}

#[cfg(test)]
mod tests {
    use super::Log;
    use crate::storage::Lsn;

    #[test]
    fn each_pin_holds_the_log_from_its_lsn_until_it_is_given_back() {
        let mut log = Log::new(512, 1, [0; 16]);
        log.checkpointed(Lsn(100), Lsn(100));
        log.release_session();
        assert_eq!(Lsn(100), log.needed());
        let first = log.pin(Lsn(40));
        let second = log.pin(Lsn(40));
        let later = log.pin(Lsn(60));
        assert_eq!(Lsn(40), log.needed());
        log.unpin(first);
        assert_eq!(Lsn(40), log.needed());
        log.unpin(second);
        assert_eq!(Lsn(60), log.needed());
        log.unpin(later);
        assert_eq!(Lsn(100), log.needed());
    }
}
