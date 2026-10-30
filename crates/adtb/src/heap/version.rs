use std::collections::BTreeSet;

use crate::key::Tid;
use crate::runtime::Runtime;
use crate::storage::{PageError, Pager, Pid};

use super::page::{self, Slot};

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

/// Returns the slot on `page`, page `pid` of its heap, holding the heap-only successor the version
/// in `slot` links to, if any.
pub(super) fn link(page: &[u8], pid: u64, slot: usize) -> Option<usize> {
    let Slot::Normal { flags, at } = page::get(page, slot) else {
        return None;
    };
    if !flags.contains(Flags::HOT_UPDATED) {
        return None;
    }
    let next = Header::read(&page[at]).next.filter(|n| n.page() == pid)?;
    let to = usize::from(next.slot());
    matches!(page::get(page, to), Slot::Normal { flags, .. } if flags.contains(Flags::HEAP_ONLY))
        .then_some(to)
}

/// Returns the slots of the chain from `start`, following `link` until it ends or a slot repeats.
pub(super) fn chain(start: usize, link: impl Fn(usize) -> Option<usize>) -> Vec<usize> {
    let mut members = vec![start];
    while let Some(to) = link(*members.last().unwrap()) {
        if members.contains(&to) {
            break;
        }
        members.push(to);
    }
    members
}

/// Returns what is wrong with the links of `page`, page `pid` of its heap: each hot-updated slot
/// that links to no version its deleter wrote, and each version two slots link to.
pub(super) fn link_faults(page: &[u8], pid: u64) -> Vec<String> {
    let header = |s| match page::get(page, s) {
        Slot::Normal { at, .. } => Some(Header::read(&page[at])),
        _ => None,
    };
    let mut faults = Vec::new();
    let mut successors = BTreeSet::new();
    let mut shared = BTreeSet::new();
    for slot in 0..page::slot_count(page) {
        if !matches!(page::get(page, slot), Slot::Normal { flags, .. } if flags.contains(Flags::HOT_UPDATED))
        {
            continue;
        }
        let written = link(page, pid, slot).filter(
            |&to| matches!((header(slot), header(to)), (Some(h), Some(s)) if s.xmin == h.xmax),
        );
        match written {
            Some(to) if !successors.insert(to) => {
                shared.insert(to);
            }
            Some(_) => {}
            None => faults.push(format!(
                "slot {slot} is hot-updated but names no heap-only successor"
            )),
        }
    }
    faults.extend(
        shared
            .into_iter()
            .map(|to| format!("slot {to} is the heap-only successor of more than one slot")),
    );
    faults
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

/// Returns the header of the version at `tid`, or `None` when its page is not a heap page or
/// its slot holds no version.
pub fn header_at<R: Runtime>(pager: &Pager<R>, tid: Tid) -> Result<Option<Header>, PageError> {
    let page = pager.read(Pid(tid.page()))?;
    if !page::is_heap(&page) || !page::check(&page).is_empty() {
        return Ok(None);
    }
    Ok(match page::get(&page, usize::from(tid.slot())) {
        Slot::Normal { at, .. } => Some(Header::read(&page[at])),
        _ => None,
    })
}
