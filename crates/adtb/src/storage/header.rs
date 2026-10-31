use super::crc::crc32c_skipping;
use super::page::{MAX_PAGE_SIZE, MIN_PAGE_SIZE, valid_page_size};
use super::{Lsn, Pid};

/// The bytes of a header copy at the start of its page; the rest of the page is unused.
pub const REGION: usize = 4096;

const MAGIC: [u8; 8] = *b"adtbfile";
const LAYOUT: u32 = 3;
const CHECKSUM: usize = 8;
const ATOMIC_PAGE_WRITES: u16 = 1;
const BITMAP_GROUPS: usize = REGION - 8;
const HISTORY: usize = 2048;
const HISTORY_ENTRY: usize = 16;
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
        out.resize(HISTORY, 0);
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
        };
        valid_page_size(header.page_size).then_some(header)
    }
}

fn history(bytes: &[u8]) -> Option<Vec<(u64, Lsn)>> {
    let mut c = Cursor { bytes, at: HISTORY };
    let n = c.u32() as usize;
    (n <= HISTORY_CAPACITY).then(|| (0..n).map(|_| (c.u64(), Lsn(c.u64()))).collect())
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
