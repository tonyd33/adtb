use std::collections::BTreeSet;

use adtb_sys::lock::Lock;

use crate::runtime::{AlignedBuf, Runtime};

use super::block::{LogFinding, LogProblem};
use super::crc::crc32c_skipping;
use super::page::{MAX_PAGE_SIZE, MIN_PAGE_SIZE, valid_page_size};
use super::pager::Pager;
use super::{Lsn, Pid};

/// The bytes of a header copy at the start of its page; the rest of the page is unused.
pub const REGION: usize = 4096;

const MAGIC: [u8; 8] = *b"adtbfile";
const LAYOUT: u32 = 3;
const CHECKSUM: usize = 8;
const ATOMIC_PAGE_WRITES: u16 = 1;
const BITMAP_GROUPS: usize = REGION - 8;
const HISTORY: usize = 2048;
/// Zero in a header written before the field existed.
const LOG_START: usize = HISTORY - 8;
const HISTORY_ENTRY: usize = 16;
/// The end of the encryption parameters.
const DAMAGE_LIST: usize = 1024;
const LOG_DAMAGE: usize = DAMAGE_LIST + 8;
const LOG_DAMAGE_ENTRY: usize = 16;
const LOG_DAMAGE_ENTRIES: usize = LOG_DAMAGE + 8;
/// The log findings a header holds.
pub const LOG_DAMAGE_CAPACITY: usize = (LOG_START - LOG_DAMAGE_ENTRIES) / LOG_DAMAGE_ENTRY;
const LOG_STALE: u64 = 1;
/// The bit of the log findings' count set when more were found than a header holds.
const LOG_DAMAGE_FULL: u64 = 1 << 63;
/// The ended epochs a header holds; a restore past it forgets the oldest.
pub const HISTORY_CAPACITY: usize = (BITMAP_GROUPS - HISTORY - 4) / HISTORY_ENTRY;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileHeader {
    pub seq: u64,
    pub file_id: [u8; 16],
    pub page_size: u32,
    pub log_block: u32,
    pub segment_size: u64,
    pub lowest_format: u16,
    pub highest_format: u16,
    pub upgrade_state: u16,
    pub atomic_page_writes: bool,
    pub checkpoint: Lsn,
    pub catalog_root: Pid,
    /// The page holding the quarantine marks, 0 before it exists.
    pub quarantine: Pid,
    pub epoch: u64,
    /// The encryption scheme; 0 is a plaintext file, the only one written so far.
    pub encryption: u32,
    /// The allocation bitmap groups started by the time of the checkpoint this header names.
    pub bitmap_groups: u64,
    /// Each epoch of the file's history before the current one, oldest first, with the log's end
    /// where it ended.
    pub history: Vec<(u64, Lsn)>,
    /// Where this file's log starts in its epoch: 0, or a restored backup's end.
    pub log_start: Lsn,
    /// The page listing the damage recorded, 0 before it exists.
    pub damage_list: Pid,
    /// Stale blocks recovery found past the end of the log and the damage list does not hold yet,
    /// at most [`LOG_DAMAGE_CAPACITY`].
    pub log_damage: Vec<LogFinding>,
    /// Whether recovery found more stale blocks than [`LOG_DAMAGE_CAPACITY`], which damages the
    /// whole file.
    pub log_damage_full: bool,
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn take<const N: usize>(&mut self) -> [u8; N] {
        let out = self.bytes[self.at..self.at + N].try_into().unwrap();
        self.at += N;
        out
    }
    fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.take())
    }
    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take())
    }
    fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.take())
    }
}

impl FileHeader {
    /// Returns header copy `copy` (0 or 1) as the first [`REGION`] bytes of its page.
    pub fn encode(&self, copy: u64) -> Vec<u8> {
        let mut out = Vec::with_capacity(REGION);
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&LAYOUT.to_le_bytes());
        out.extend_from_slice(&self.seq.to_le_bytes());
        out.extend_from_slice(&self.file_id);
        out.extend_from_slice(&self.page_size.to_le_bytes());
        out.extend_from_slice(&self.log_block.to_le_bytes());
        out.extend_from_slice(&self.segment_size.to_le_bytes());
        out.extend_from_slice(&self.lowest_format.to_le_bytes());
        out.extend_from_slice(&self.highest_format.to_le_bytes());
        out.extend_from_slice(&self.upgrade_state.to_le_bytes());
        let flags = if self.atomic_page_writes {
            ATOMIC_PAGE_WRITES
        } else {
            0
        };
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&self.checkpoint.0.to_le_bytes());
        out.extend_from_slice(&self.catalog_root.0.to_le_bytes());
        out.extend_from_slice(&self.quarantine.0.to_le_bytes());
        out.extend_from_slice(&self.epoch.to_le_bytes());
        out.extend_from_slice(&self.encryption.to_le_bytes());
        out.resize(DAMAGE_LIST, 0);
        out.extend_from_slice(&self.damage_list.0.to_le_bytes());
        assert!(
            self.log_damage.len() <= LOG_DAMAGE_CAPACITY,
            "a header holds at most {LOG_DAMAGE_CAPACITY} log findings"
        );
        let full = if self.log_damage_full {
            LOG_DAMAGE_FULL
        } else {
            0
        };
        out.extend_from_slice(&(self.log_damage.len() as u64 | full).to_le_bytes());
        for f in &self.log_damage {
            assert_eq!(
                f.problem,
                LogProblem::Stale,
                "a header holds only stale log blocks"
            );
            out.extend_from_slice(&f.lsn.0.to_le_bytes());
            out.extend_from_slice(&LOG_STALE.to_le_bytes());
        }
        out.resize(LOG_START, 0);
        out.extend_from_slice(&self.log_start.0.to_le_bytes());
        out.extend_from_slice(&(self.history.len() as u32).to_le_bytes());
        for (epoch, end) in &self.history {
            out.extend_from_slice(&epoch.to_le_bytes());
            out.extend_from_slice(&end.0.to_le_bytes());
        }
        out.resize(BITMAP_GROUPS, 0);
        out.extend_from_slice(&self.bitmap_groups.to_le_bytes());
        let sum = crc32c_skipping(Some(copy), &out, CHECKSUM);
        out[CHECKSUM..CHECKSUM + 4].copy_from_slice(&sum.to_le_bytes());
        out
    }

    /// Returns the header copy `copy` that `bytes` holds, or `None` if it fails verification.
    pub fn decode(copy: u64, bytes: &[u8]) -> Option<FileHeader> {
        let bytes = whole_copy(copy, bytes)?;
        let mut c = Cursor { bytes, at: 12 };
        if c.u32() != LAYOUT {
            return None;
        }
        let header = FileHeader {
            seq: c.u64(),
            file_id: c.take(),
            page_size: c.u32(),
            log_block: c.u32(),
            segment_size: c.u64(),
            lowest_format: c.u16(),
            highest_format: c.u16(),
            upgrade_state: c.u16(),
            atomic_page_writes: c.u16() & ATOMIC_PAGE_WRITES != 0,
            checkpoint: Lsn(c.u64()),
            catalog_root: Pid(c.u64()),
            quarantine: Pid(c.u64()),
            epoch: c.u64(),
            encryption: c.u32(),
            bitmap_groups: Cursor {
                bytes,
                at: BITMAP_GROUPS,
            }
            .u64(),
            history: history(bytes)?,
            log_start: Lsn(Cursor {
                bytes,
                at: LOG_START,
            }
            .u64()),
            damage_list: Pid(Cursor {
                bytes,
                at: DAMAGE_LIST,
            }
            .u64()),
            log_damage: log_damage(bytes)?,
            log_damage_full: Cursor {
                bytes,
                at: LOG_DAMAGE,
            }
            .u64()
                & LOG_DAMAGE_FULL
                != 0,
        };
        valid_page_size(header.page_size).then_some(header)
    }
}

fn history(bytes: &[u8]) -> Option<Vec<(u64, Lsn)>> {
    let mut c = Cursor { bytes, at: HISTORY };
    let n = c.u32() as usize;
    (n <= HISTORY_CAPACITY).then(|| (0..n).map(|_| (c.u64(), Lsn(c.u64()))).collect())
}

fn log_damage(bytes: &[u8]) -> Option<Vec<LogFinding>> {
    let mut c = Cursor {
        bytes,
        at: LOG_DAMAGE,
    };
    let n = usize::try_from(c.u64() & !LOG_DAMAGE_FULL).ok()?;
    if n > LOG_DAMAGE_CAPACITY {
        return None;
    }
    (0..n)
        .map(|_| {
            let lsn = Lsn(c.u64());
            (c.u64() == LOG_STALE).then_some(LogFinding {
                lsn,
                problem: LogProblem::Stale,
            })
        })
        .collect()
}

/// Returns the byte offsets where copy 1 may start: one per page size a file may have.
pub fn second_copy_offsets() -> impl Iterator<Item = u64> {
    (MIN_PAGE_SIZE.trailing_zeros()..=MAX_PAGE_SIZE.trailing_zeros()).map(|s| 1u64 << s)
}

/// Returns the region of header copy `copy` when its magic and checksum hold.
fn whole_copy(copy: u64, bytes: &[u8]) -> Option<&[u8]> {
    if bytes.len() < REGION || bytes[..8] != MAGIC {
        return None;
    }
    let bytes = &bytes[..REGION];
    let stored = u32::from_le_bytes(bytes[CHECKSUM..CHECKSUM + 4].try_into().unwrap());
    (stored == crc32c_skipping(Some(copy), bytes, CHECKSUM)).then_some(bytes)
}

/// Returns the layout of a whole header copy, when it is one this engine does not read.
pub fn other_layout(copy: u64, bytes: &[u8]) -> Option<u32> {
    let bytes = whole_copy(copy, bytes)?;
    let layout = Cursor { bytes, at: 12 }.u32();
    (layout != LAYOUT).then_some(layout)
}

/// Returns the copy to use, and its index: the valid copy with the higher sequence number.
pub fn choose(first: Option<FileHeader>, second: Option<FileHeader>) -> Option<(u64, FileHeader)> {
    match (first, second) {
        (Some(a), Some(b)) if b.seq > a.seq => Some((1, b)),
        (Some(a), _) => Some((0, a)),
        (None, Some(b)) => Some((1, b)),
        (None, None) => None,
    }
}

/// The file header as last written and the copy it was written to, under the header lock.
///
/// Invariants:
///
/// - Each write takes the other copy and the next sequence number.
/// - The checkpoint and the bitmap group count only rise.
/// - A copy written, or the header read, names the marks page and the damage list page their
///   owners name at the time; the header keeps no value of its own for either past the open.
#[derive(Debug)]
pub(super) struct Header(Lock<(FileHeader, u64)>);

impl Header {
    /// Returns the header of a file opened from `header`, read from copy `copy`.
    pub(super) fn new(header: FileHeader, copy: u64) -> Header {
        Header(Lock::new((header, copy)))
    }

    /// Returns what `f` reads of the header as last written, naming `marks` as the marks page
    /// and `list` as the damage list page.
    pub(super) fn read<R: Runtime, T>(
        &self,
        rt: &R,
        marks: Pid,
        list: Pid,
        f: impl FnOnce(&FileHeader) -> T,
    ) -> T {
        let mut h = self.0.lock(rt);
        h.0.quarantine = marks;
        h.0.damage_list = list;
        f(&h.0)
    }

    /// Name `root` as the catalog's root from the next write.
    pub(super) fn set_catalog_root<R: Runtime>(&self, rt: &R, root: Pid) {
        self.0.lock(rt).0.catalog_root = root;
    }

    /// Add `stale`, stale log blocks recovery found, to those the header holds, the lowest that
    /// fit, marking the header full when some do not. Returns whether that changed the header.
    pub(super) fn keep_log_findings<'a, R: Runtime>(
        &self,
        rt: &R,
        stale: impl Iterator<Item = &'a LogFinding>,
    ) -> bool {
        let mut h = self.0.lock(rt);
        let mut kept: BTreeSet<LogFinding> = h.0.log_damage.iter().copied().collect();
        kept.extend(stale);
        let full = h.0.log_damage_full || kept.len() > LOG_DAMAGE_CAPACITY;
        let kept: Vec<LogFinding> = kept.into_iter().take(LOG_DAMAGE_CAPACITY).collect();
        let changed = kept != h.0.log_damage || full != h.0.log_damage_full;
        h.0.log_damage = kept;
        h.0.log_damage_full = full;
        changed
    }

    /// Drop the log findings from the next write.
    ///
    /// Preconditions: a fold of the damage records that holds them is logged and forced.
    pub(super) fn clear_log_findings<R: Runtime>(&self, rt: &R) {
        let mut h = self.0.lock(rt);
        h.0.log_damage.clear();
        h.0.log_damage_full = false;
    }

    /// Pass `write` the next copy and its bytes, naming `marks` as the marks page, `list` as the
    /// damage list page and the checkpoint of `checkpoint` unless the header names a later one,
    /// under the header lock. Write nothing when it names a later one. The bitmap group count
    /// rises to the checkpoint's in either case.
    pub(super) fn write_next<R: Runtime>(
        &self,
        rt: &R,
        checkpoint: Option<(Lsn, u64)>,
        marks: Pid,
        list: Pid,
        write: impl FnOnce(u64, &[u8]),
    ) {
        let mut h = self.0.lock(rt);
        let (header, copy) = &mut *h;
        if let Some((at, groups)) = checkpoint {
            header.bitmap_groups = header.bitmap_groups.max(groups);
            if at < header.checkpoint {
                return;
            }
            header.checkpoint = at;
        }
        header.quarantine = marks;
        header.damage_list = list;
        header.seq += 1;
        *copy = 1 - *copy;
        write(*copy, &header.encode(*copy));
    }
}

impl<R: Runtime> Pager<R> {
    /// Write the next header copy, pointing at the checkpoint of `checkpoint` unless the header
    /// already names a later one. The header's bitmap group count only rises.
    pub(super) fn write_header(&self, checkpoint: Option<(Lsn, u64)>) {
        let (marks, list) = (self.quarantine_page(), self.damage_list_page());
        self.header()
            .write_next(self.rt(), checkpoint, marks, list, |copy, bytes| {
                let offset = copy * self.page_size() as u64;
                self.io(|d| {
                    d.write_all(vec![(self.data(), offset, AlignedBuf::from_slice(bytes))]);
                    d.sync_all(&[self.data()]);
                });
            });
    }

    /// Returns what `f` reads of the header as a copy written now would hold it.
    pub(super) fn read_header<T>(&self, f: impl FnOnce(&FileHeader) -> T) -> T {
        let (marks, list) = (self.quarantine_page(), self.damage_list_page());
        self.header().read(self.rt(), marks, list, f)
    }

    /// Returns the page the file header names as the catalog's root, `Pid(0)` before one is set.
    pub fn catalog_root(&self) -> Pid {
        self.read_header(|h| h.catalog_root)
    }

    /// Force the log, then write and force a header copy naming `root` as the catalog's root.
    pub fn set_catalog_root(&self, root: Pid) {
        self.flush_log();
        self.header().set_catalog_root(self.rt(), root);
        self.write_header(None);
    }

    /// Returns the LSN of the checkpoint record the file header names.
    pub(crate) fn checkpoint_lsn(&self) -> Lsn {
        self.read_header(|h| h.checkpoint)
    }
}
