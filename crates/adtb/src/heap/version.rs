use crate::key::Tid;
use crate::mvcc::{Writers, Xid};

/// The length of the header every version starts with.
pub const HEADER_LEN: usize = 24;

/// The header of a row version: its writers and its successor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub writers: Writers,
    pub next: Option<Tid>,
}

impl Header {
    /// Returns a header for a version `x` inserts.
    pub fn new(x: Xid) -> Header {
        Header {
            writers: Writers::inserted(x),
            next: None,
        }
    }

    /// Returns the header at the start of `version`.
    pub fn read(version: &[u8]) -> Header {
        let (writers, next) = version[..HEADER_LEN].split_at(Writers::LEN);
        Header {
            writers: Writers::decode(writers.try_into().expect("writers bytes")),
            next: tid_from_bits(u64::from_le_bytes(next.try_into().expect("next bytes"))),
        }
    }

    /// Write the header over the first 24 bytes of `version`.
    pub fn write(&self, version: &mut [u8]) {
        version[..Writers::LEN].copy_from_slice(&self.writers.encode());
        version[Writers::LEN..HEADER_LEN]
            .copy_from_slice(&self.next.map_or(0, tid_bits).to_le_bytes());
    }
}

/// Returns `tid` as one word: the page id in the upper 48 bits, the slot in the lower 16.
pub fn tid_bits(tid: Tid) -> u64 {
    tid.page() << 16 | u64::from(tid.slot())
}

/// Returns the TID [`tid_bits`] packed into `bits`, or `None` for zero, which names no version
/// since page 0 is a file header copy.
pub fn tid_from_bits(bits: u64) -> Option<Tid> {
    (bits != 0).then(|| Tid::new(bits >> 16, bits as u16).expect("48-bit page id"))
}

/// The flags of a version, kept in its slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Flags(pub u16);

impl Flags {
    pub const NONE: Flags = Flags(0);
    /// The successor `next` names is in another heap of the sum.
    pub const MOVED: Flags = Flags(1);
    /// A retag inserted this version.
    pub const RETAGGED_IN: Flags = Flags(2);
    /// The successor is a heap-only version on this page.
    pub const HOT_UPDATED: Flags = Flags(4);
    /// No index entry names this version.
    pub const HEAP_ONLY: Flags = Flags(8);
    /// A copy of this version is in its constructor's history heap.
    pub const KEPT: Flags = Flags(16);

    pub fn contains(self, other: Flags) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn with(self, other: Flags) -> Flags {
        Flags(self.0 | other.0)
    }

    pub fn without(self, other: Flags) -> Flags {
        Flags(self.0 & !other.0)
    }
}
