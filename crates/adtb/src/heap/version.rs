use crate::key::Tid;

/// The length of the header every version starts with.
pub const HEADER_LEN: usize = 24;

/// The header of a row version: its writers and its successor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub xmin: u64,
    /// Zero when no transaction has deleted or superseded the version.
    pub xmax: u64,
    pub next: Option<Tid>,
}

impl Header {
    /// Returns a header for a version `xmin` inserts.
    pub fn new(xmin: u64) -> Header {
        Header {
            xmin,
            xmax: 0,
            next: None,
        }
    }

    /// Returns the header at the start of `version`.
    pub fn read(version: &[u8]) -> Header {
        let word = |i: usize| u64::from_le_bytes(version[i..i + 8].try_into().unwrap());
        Header {
            xmin: word(0),
            xmax: word(8),
            next: tid_from_bits(word(16)),
        }
    }

    /// Write the header over the first 24 bytes of `version`.
    pub fn write(&self, version: &mut [u8]) {
        version[0..8].copy_from_slice(&self.xmin.to_le_bytes());
        version[8..16].copy_from_slice(&self.xmax.to_le_bytes());
        version[16..24].copy_from_slice(&self.next.map_or(0, tid_bits).to_le_bytes());
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
