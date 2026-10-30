use super::crc::crc32c_skipping;
use super::{Lsn, Pid};

pub const MIN_PAGE_SIZE: u32 = 4096;
pub const MAX_PAGE_SIZE: u32 = 65536;

/// The length of the plaintext page header; kind-specific contents start here.
pub const HEADER_LEN: usize = 24;

/// The first byte a page change may write: the LSN and checksum are the pager's.
pub const FIRST_EDITABLE: usize = 12;

const LSN: usize = 0;
const CHECKSUM: usize = 8;
const FORMAT: usize = 12;
const KIND: usize = 14;
const OWNER: usize = 16;
const FLAGS: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum PageKind {
    HeapFixed = 1,
    HeapSlotted = 2,
    IndexLeaf = 3,
    IndexInner = 4,
    HashBucket = 5,
    FreeSpaceMap = 6,
    VisibilityMap = 7,
    AllocationBitmap = 8,
    CommitLog = 9,
    Catalog = 10,
    Quarantine = 11,
}

impl PageKind {
    pub fn from_u16(v: u16) -> Option<PageKind> {
        use PageKind::*;
        Some(match v {
            1 => HeapFixed,
            2 => HeapSlotted,
            3 => IndexLeaf,
            4 => IndexInner,
            5 => HashBucket,
            6 => FreeSpaceMap,
            7 => VisibilityMap,
            8 => AllocationBitmap,
            9 => CommitLog,
            10 => Catalog,
            11 => Quarantine,
            _ => return None,
        })
    }
}

/// Returns whether `size` is a page size a file may be created with.
pub fn valid_page_size(size: u32) -> bool {
    size.is_power_of_two() && (MIN_PAGE_SIZE..=MAX_PAGE_SIZE).contains(&size)
}

fn u16_at(page: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([page[at], page[at + 1]])
}

pub fn lsn(page: &[u8]) -> Lsn {
    Lsn(u64::from_le_bytes(page[LSN..LSN + 8].try_into().unwrap()))
}

pub fn set_lsn(page: &mut [u8], lsn: Lsn) {
    page[LSN..LSN + 8].copy_from_slice(&lsn.0.to_le_bytes());
}

/// Returns the offset and bytes of the LSN field holding `lsn`.
pub fn lsn_field(lsn: Lsn) -> (usize, [u8; 8]) {
    (LSN, lsn.0.to_le_bytes())
}

/// Returns a copy of `page` with its LSN set to `lsn`.
pub fn with_lsn(page: &[u8], lsn: Lsn) -> Vec<u8> {
    let mut copy = page.to_vec();
    set_lsn(&mut copy, lsn);
    copy
}

pub fn format(page: &[u8]) -> u16 {
    u16_at(page, FORMAT)
}

pub fn kind(page: &[u8]) -> u16 {
    u16_at(page, KIND)
}

pub fn owner(page: &[u8]) -> u32 {
    u32::from_le_bytes(page[OWNER..OWNER + 4].try_into().unwrap())
}

/// Flip a bit of the page's owner, which a second flip undoes.
pub fn flip_owner(page: &mut [u8]) {
    page[OWNER] ^= 1;
}

pub fn flags(page: &[u8]) -> u16 {
    u16_at(page, FLAGS)
}

/// Returns the bytes that set a page's format, kind and owner, to be written at [`FIRST_EDITABLE`].
pub fn header_edit(format: u16, kind: PageKind, owner: u32, flags: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN - FIRST_EDITABLE);
    out.extend_from_slice(&format.to_le_bytes());
    out.extend_from_slice(&(kind as u16).to_le_bytes());
    out.extend_from_slice(&owner.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&[0; 2]);
    out
}

fn checksum(pid: Pid, page: &[u8]) -> u32 {
    crc32c_skipping(Some(pid.0), page, CHECKSUM)
}

/// Write the checksum of `page` as page `pid` into its header.
pub fn seal(pid: Pid, page: &mut [u8]) {
    let sum = checksum(pid, page);
    page[CHECKSUM..CHECKSUM + 4].copy_from_slice(&sum.to_le_bytes());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Valid,
    /// Every byte is zero: the page was never written.
    Blank,
    Corrupt,
}

pub fn verify(pid: Pid, page: &[u8]) -> Verdict {
    let stored = u32::from_le_bytes(page[CHECKSUM..CHECKSUM + 4].try_into().unwrap());
    if stored == checksum(pid, page) {
        Verdict::Valid
    } else if page.iter().all(|&b| b == 0) {
        Verdict::Blank
    } else {
        Verdict::Corrupt
    }
}
