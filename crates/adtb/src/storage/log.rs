use std::collections::BTreeMap;

use crate::runtime::{AlignedBuf, Done, FileId, IoError, Op, Runtime};

use super::Lsn;
use super::block::{LogImage, LogSource, Slot, data_lsn, frame};
use super::driver::{Driver, fatal};
use super::pager::parent;
use super::record::Record;

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

/// The log's segment files, behind the flush lock, which one flusher holds at a time.
#[derive(Debug)]
pub(crate) struct Segments {
    pub(crate) dir: String,
    block: usize,
    segment: u64,
    file_id: [u8; 16],
    files: BTreeMap<u64, FileId>,
    /// The directory exists and its creation is forced.
    made: bool,
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
    /// Preconditions: the caller holds the flush lock, and forces the batch before taking another.
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

    /// Record that `batch` is forced.
    pub(crate) fn forced(&mut self, batch: &Batch) {
        assert_eq!(self.durable, batch.start, "batches are forced in order");
        self.durable = batch.end();
        self.batches += 1;
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
