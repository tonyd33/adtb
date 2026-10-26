use std::ops::Range;

use crate::storage::{self, HEADER_LEN, PageKind};

use super::version::Flags;
use super::{fixed, slotted};

pub(super) const CONTENTS: usize = HEADER_LEN;
const POSITION: usize = CONTENTS;
const PAGE_FLAGS: usize = 20;
const ALL_VISIBLE: u16 = 1;

pub(super) fn align8(n: usize) -> usize {
    n.div_ceil(8) * 8
}

pub(super) fn u16_at(page: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([page[at], page[at + 1]])
}

pub(super) fn u32_at(page: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(page[at..at + 4].try_into().unwrap())
}

pub(super) fn u64_at(page: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(page[at..at + 8].try_into().unwrap())
}

/// What a slot of a heap page holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Slot {
    Unused,
    /// A version, at the byte range `at` of the page.
    Normal {
        flags: Flags,
        at: Range<usize>,
    },
    /// Pruned, and still named by index entries.
    Dead,
    /// Pruned; the next live version of its heap-only chain is the slot named, on this page.
    Redirect(u16),
}

/// How a heap page arranges its versions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrangement {
    /// Every slot is `width` bytes at a fixed offset.
    Fixed {
        width: usize,
    },
    Slotted,
}

impl Arrangement {
    pub fn kind(self) -> PageKind {
        match self {
            Arrangement::Fixed { .. } => PageKind::HeapFixed,
            Arrangement::Slotted => PageKind::HeapSlotted,
        }
    }

    /// Returns the free bytes a page needs to take a version of `len` bytes.
    pub fn need(self, len: usize) -> usize {
        match self {
            Arrangement::Fixed { width } => width,
            Arrangement::Slotted => align8(len),
        }
    }

    /// Format the contents of `page` as an empty page at `position` of its heap's directory.
    pub fn init(self, page: &mut [u8], position: u32) {
        match self {
            Arrangement::Fixed { width } => fixed::init(page, width),
            Arrangement::Slotted => slotted::init(page),
        }
        page[POSITION..POSITION + 4].copy_from_slice(&position.to_le_bytes());
    }
}

/// Returns whether `page` is a heap page of either arrangement.
pub fn is_heap(page: &[u8]) -> bool {
    matches!(
        PageKind::from_u16(storage::kind(page)),
        Some(PageKind::HeapFixed | PageKind::HeapSlotted)
    )
}

/// Returns what is wrong with the slots of heap page `page`.
pub fn check(page: &[u8]) -> Vec<String> {
    match arrangement(page) {
        Arrangement::Fixed { .. } => fixed::check(page),
        Arrangement::Slotted => slotted::check(page),
    }
}

/// Returns the arrangement of a heap page.
///
/// Panics if `page` is not a heap page.
pub fn arrangement(page: &[u8]) -> Arrangement {
    match PageKind::from_u16(storage::kind(page)) {
        Some(PageKind::HeapFixed) => Arrangement::Fixed {
            width: fixed::width(page),
        },
        Some(PageKind::HeapSlotted) => Arrangement::Slotted,
        k => panic!("page of kind {k:?} is not a heap page"),
    }
}

/// Returns the page's index in its heap's directory.
pub fn position(page: &[u8]) -> u32 {
    u32_at(page, POSITION)
}

/// Returns whether the page header's all-visible flag is set.
pub fn all_visible(page: &[u8]) -> bool {
    storage::flags(page) & ALL_VISIBLE != 0
}

pub fn set_all_visible(page: &mut [u8], on: bool) {
    let flags = storage::flags(page);
    let flags = if on {
        flags | ALL_VISIBLE
    } else {
        flags & !ALL_VISIBLE
    };
    page[PAGE_FLAGS..PAGE_FLAGS + 2].copy_from_slice(&flags.to_le_bytes());
}

/// Returns whether no slot of the page holds anything.
pub fn is_empty(page: &[u8]) -> bool {
    (0..slot_count(page)).all(|s| get(page, s) == Slot::Unused)
}

pub fn slot_count(page: &[u8]) -> usize {
    match arrangement(page) {
        Arrangement::Fixed { .. } => fixed::count(page),
        Arrangement::Slotted => slotted::count(page),
    }
}

pub fn get(page: &[u8], slot: usize) -> Slot {
    match arrangement(page) {
        Arrangement::Fixed { .. } => fixed::get(page, slot),
        Arrangement::Slotted => slotted::get(page, slot),
    }
}

pub fn set_flags(page: &mut [u8], slot: usize, flags: Flags) {
    match arrangement(page) {
        Arrangement::Fixed { .. } => fixed::set_flags(page, slot, flags),
        Arrangement::Slotted => slotted::set_flags(page, slot, flags),
    }
}

/// Returns the bytes free for new versions.
pub fn free(page: &[u8]) -> usize {
    match arrangement(page) {
        Arrangement::Fixed { .. } => fixed::free(page),
        Arrangement::Slotted => slotted::free(page),
    }
}

/// Write `version` into a free slot. Returns the slot, or `None` if the page lacks the room.
pub fn insert(page: &mut [u8], version: &[u8], flags: Flags) -> Option<usize> {
    match arrangement(page) {
        Arrangement::Fixed { .. } => fixed::insert(page, version, flags),
        Arrangement::Slotted => slotted::insert(page, version, flags),
    }
}

/// Move a slotted page's tuples together at its end. A fixed-slot page is left as it is.
pub fn compact(page: &mut [u8]) {
    if arrangement(page) == Arrangement::Slotted {
        slotted::compact(page);
    }
}

/// Empty slot `slot`, leaving it unused, dead or a redirect as `content` says.
///
/// Panics if `content` is a version, or a dead slot or redirect on a fixed-slot page.
pub fn release(page: &mut [u8], slot: usize, content: &Slot) {
    match (arrangement(page), content) {
        (Arrangement::Fixed { .. }, Slot::Unused) => fixed::release(page, slot),
        (Arrangement::Slotted, Slot::Unused) => slotted::release(page, slot, None),
        (Arrangement::Slotted, Slot::Dead) => {
            slotted::release(page, slot, Some(slotted::Content::Dead))
        }
        (Arrangement::Slotted, Slot::Redirect(to)) => {
            slotted::release(page, slot, Some(slotted::Content::Redirect(*to)))
        }
        (a, c) => panic!("a {a:?} page cannot release a slot to {c:?}"),
    }
}

/// Fill the unused slot `slot` with what `content` holds. Returns `false`, changing nothing, when
/// the page cannot.
pub fn put(page: &mut [u8], slot: usize, content: &Slot, version: &[u8]) -> bool {
    match (arrangement(page), content) {
        (Arrangement::Fixed { .. }, Slot::Normal { flags, .. }) => {
            fixed::put(page, slot, version, *flags)
        }
        (Arrangement::Fixed { .. }, Slot::Unused) => true,
        (Arrangement::Fixed { .. }, Slot::Dead | Slot::Redirect(_)) => false,
        (Arrangement::Slotted, Slot::Normal { flags, .. }) => {
            slotted::put(page, slot, slotted::Content::Version(version, *flags))
        }
        (Arrangement::Slotted, Slot::Dead) => slotted::put(page, slot, slotted::Content::Dead),
        (Arrangement::Slotted, Slot::Redirect(to)) => {
            slotted::put(page, slot, slotted::Content::Redirect(*to))
        }
        (Arrangement::Slotted, Slot::Unused) => true,
    }
}
