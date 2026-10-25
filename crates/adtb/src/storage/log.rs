use std::collections::BTreeMap;

use crate::runtime::{AlignedBuf, FileId, Runtime};

use super::Lsn;
use super::block::{LogImage, data_lsn, frame};
use super::driver::Driver;
use super::record::Record;

/// The redo log's writer: the segment files and the batch being filled.
#[derive(Debug)]
pub(crate) struct Log {
    pub(crate) dir: String,
    pub(crate) block: usize,
    pub(crate) segment: u64,
    pub(crate) epoch: u32,
    files: BTreeMap<u64, FileId>,
    /// The first LSN of the batch being filled; every batch before it is forced.
    start: Lsn,
    seq: u64,
    pending: Vec<u8>,
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
    pub(crate) fn new(dir: String, block: usize, segment: u64, epoch: u32) -> Log {
        Log {
            dir,
            block,
            segment,
            epoch,
            files: BTreeMap::new(),
            start: Lsn(0),
            seq: 1,
            pending: Vec::new(),
        }
    }

    fn path(&self, base: u64) -> String {
        format!("{}/{}", self.dir, segment_name(base))
    }

    /// Returns the LSN the next appended record gets.
    pub(crate) fn next_lsn(&self) -> Lsn {
        data_lsn(self.start, self.pending.len(), self.block)
    }

    /// Returns the LSN up to which every record is forced.
    pub(crate) fn durable(&self) -> Lsn {
        self.start
    }

    /// Continue the log at `end` with batch number `seq`.
    pub(crate) fn resume(&mut self, end: Lsn, seq: u64) {
        assert!(self.pending.is_empty());
        self.start = end;
        self.seq = seq;
    }

    pub(crate) fn append(&mut self, record: &Record) -> Lsn {
        let lsn = self.next_lsn();
        record.encode(&mut self.pending);
        lsn
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

    /// Write and force the batch being filled; on return every appended record is durable.
    pub(crate) fn flush<R: Runtime>(&mut self, drv: &mut Driver<R>) {
        if self.pending.is_empty() {
            return;
        }
        let bytes = frame(self.start, self.seq, self.epoch, &self.pending, self.block);
        let mut writes = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let lsn = self.start.0 + at as u64;
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
        self.start = Lsn(self.start.0 + bytes.len() as u64);
        self.seq += 1;
        self.pending.clear();
    }

    /// Returns every segment of the log directory as read now.
    pub(crate) fn load<R: Runtime>(&mut self, drv: &mut Driver<R>) -> LogImage {
        let mut bases: Vec<u64> = drv
            .list(&self.dir)
            .iter()
            .filter_map(|p| parse_segment(p))
            .filter(|b| b % self.segment == 0)
            .collect();
        bases.sort_unstable();
        let mut segments = BTreeMap::new();
        for base in bases {
            let path = self.path(base);
            let file = match self.files.get(&base) {
                Some(&f) => Some(f),
                None => drv.open(&path, false).ok().map(|(f, _)| f),
            };
            let Some(file) = file else { continue };
            self.files.insert(base, file);
            let bytes = drv
                .read(file, 0, self.segment as usize)
                .ok()
                .map(|b| b.to_vec());
            segments.insert(base, bytes);
        }
        LogImage {
            block: self.block,
            segment: self.segment,
            segments,
        }
    }

    /// Zero every nonzero byte of the log past `end` and remove later segments, then force both.
    pub(crate) fn clear_tail<R: Runtime>(&mut self, drv: &mut Driver<R>, img: &LogImage, end: Lsn) {
        let base = end.0 - end.0 % self.segment;
        if let Some(Some(bytes)) = img.segments.get(&base) {
            let from = (end.0 - base) as usize;
            if let Some(last) = bytes[from..].iter().rposition(|&b| b != 0) {
                let to = (from + last + 1).next_multiple_of(self.block);
                let file = self.segment_file(drv, base);
                drv.write_all(vec![(file, from as u64, AlignedBuf::zeroed(to - from))]);
                drv.sync_all(&[file]);
            }
        }
        if img.segments.contains_key(&base) {
            let file = self.segment_file(drv, base);
            drv.allocate(file, self.segment);
            drv.sync_all(&[file]);
        }
        let later: Vec<u64> = img.segments.keys().copied().filter(|&b| b > base).collect();
        for b in &later {
            drv.remove(&self.path(*b));
            self.files.remove(b);
        }
        if !later.is_empty() {
            drv.sync_dir(&self.dir);
        }
    }
}
