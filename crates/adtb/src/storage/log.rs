use std::collections::BTreeMap;

use crate::runtime::{AlignedBuf, FileId, Runtime};

use super::Lsn;
use super::block::{LogImage, LogSource, Slot, data_lsn, frame};
use super::driver::Driver;
use super::record::Record;

/// The redo log's appender: the batch being filled, behind the log lock.
#[derive(Debug)]
pub(crate) struct Log {
    pub(crate) block: usize,
    pub(crate) epoch: u32,
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
    files: BTreeMap<u64, FileId>,
}

fn segment_name(base: u64) -> String {
    format!("{base:016x}")
}

fn parse_segment(path: &str) -> Option<u64> {
    let name = path.rsplit_once('/').map_or(path, |(_, n)| n);
    (name.len() == 16)
        .then(|| u64::from_str_radix(name, 16).ok())
        .flatten()
}

impl Log {
    pub(crate) fn new(block: usize, epoch: u32) -> Log {
        Log {
            block,
            epoch,
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
        let blocks = frame(self.start, self.seq, self.epoch, &self.pending, self.block);
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
    pub(crate) fn new(dir: String, block: usize, segment: u64) -> Segments {
        Segments {
            dir,
            block,
            segment,
            files: BTreeMap::new(),
        }
    }

    fn path(&self, base: u64) -> String {
        format!("{}/{}", self.dir, segment_name(base))
    }

    /// Open the segment starting at `base`, creating and preallocating it if absent.
    fn segment_file<R: Runtime>(&mut self, drv: &mut Driver<R>, base: u64) -> FileId {
        if let Some(&f) = self.files.get(&base) {
            return f;
        }
        let path = self.path(base);
        let (file, len) = drv
            .open(&path, true)
            .unwrap_or_else(|e| panic!("fatal I/O error: create {path}: {e:?}"));
        if len < self.segment {
            drv.allocate(file, self.segment);
            drv.sync_all(&[file]);
        }
        drv.sync_dir(&self.dir);
        self.files.insert(base, file);
        file
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
        let mut bases: Vec<u64> = drv
            .list(&self.dir)
            .iter()
            .filter_map(|p| parse_segment(p))
            .filter(|b| b % self.segment == 0)
            .collect();
        bases.sort_unstable();
        let mut files = BTreeMap::new();
        for base in bases {
            let file = match self.files.get(&base) {
                Some(&f) => Some(f),
                None => drv.open(&self.path(base), false).ok().map(|(f, _)| f),
            };
            if let Some(file) = file {
                self.files.insert(base, file);
                files.insert(base, file);
            }
        }
        LogImage {
            block: self.block,
            segment: self.segment,
            bases: files.keys().copied().collect(),
            source: FileSource {
                drv: Driver::new(drv.rt.clone()),
                files,
                segment: self.segment,
                chunk: None,
            },
        }
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
            self.files.remove(b);
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
