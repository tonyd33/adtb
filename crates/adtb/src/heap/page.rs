use std::ops::{Deref, DerefMut, Range};

use crate::runtime::Runtime;
use crate::storage::{self, HEADER_LEN, PageExclusive, PageKind, PageShared, Pid, Reservation};

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

/// A page whose header names its owner: a latched page or a copy of one.
pub(super) trait Owned: Deref<Target = [u8]> {
    fn owner(&self) -> u32;
}

impl Owned for PageShared<'_> {
    fn owner(&self) -> u32 {
        PageShared::owner(self)
    }
}

impl<R: Runtime> Owned for PageExclusive<'_, R> {
    fn owner(&self) -> u32 {
        PageExclusive::owner(self)
    }
}

/// A copy of a page with the owner the page's header named when it was copied.
///
/// Invariant: no change to the copy rewrites the owner its header names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PageCopy {
    owner: u32,
    bytes: Vec<u8>,
}

impl PageCopy {
    pub fn of(page: &impl Owned) -> PageCopy {
        PageCopy::over(page, page.to_vec())
    }

    /// Returns `bytes` as the new contents of `page`.
    ///
    /// Precondition: `bytes` names the owner `page` names.
    pub fn over(page: &impl Owned, bytes: Vec<u8>) -> PageCopy {
        PageCopy {
            owner: page.owner(),
            bytes,
        }
    }

    /// Returns `bytes` as the contents of `pid`, which `new` reserves.
    ///
    /// Precondition: `bytes` names the owner `new` formats `pid` with.
    pub fn reserved<R: Runtime>(new: &Reservation<'_, R>, pid: Pid, bytes: Vec<u8>) -> PageCopy {
        PageCopy {
            owner: new.owner(pid),
            bytes,
        }
    }

    /// Replace the copy's contents by `bytes`.
    ///
    /// Precondition: `bytes` names the owner the copy names.
    pub fn replace(&mut self, bytes: Vec<u8>) {
        self.bytes = bytes;
    }

    #[cfg(test)]
    pub fn owned_by(owner: u32, bytes: Vec<u8>) -> PageCopy {
        PageCopy { owner, bytes }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl Deref for PageCopy {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.bytes
    }
}

impl DerefMut for PageCopy {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
}

impl Owned for PageCopy {
    fn owner(&self) -> u32 {
        self.owner
    }
}

/// A page of a heap kind, whatever its slot table holds.
#[derive(Clone, Copy, Debug)]
pub(super) struct Kinded<'g> {
    bytes: &'g [u8],
    owner: u32,
}

impl<'g> Kinded<'g> {
    pub fn of(page: &'g impl Owned) -> Option<Kinded<'g>> {
        has_heap_kind(page).then(|| Kinded {
            bytes: page,
            owner: page.owner(),
        })
    }

    pub fn owner(self) -> u32 {
        self.owner
    }

    /// Returns the page's index in its heap's directory.
    pub fn position(self) -> u32 {
        position(self.bytes)
    }

    /// Returns what is wrong with the slots of the page.
    pub fn check(self) -> Vec<String> {
        match arrangement(self.bytes) {
            Arrangement::Fixed { .. } => fixed::check(self.bytes),
            Arrangement::Slotted => slotted::check(self.bytes),
        }
    }

    /// Returns the page as a heap page when its slot table fits the page.
    pub fn framed(self) -> Option<HeapPage<'g>> {
        framed(self.bytes).then_some(HeapPage {
            bytes: self.bytes,
            owner: self.owner,
        })
    }
}

/// A heap page of either arrangement whose slot table fits the page.
#[derive(Clone, Copy, Debug)]
pub(super) struct HeapPage<'g> {
    bytes: &'g [u8],
    owner: u32,
}

impl Deref for HeapPage<'_> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        self.bytes
    }
}

impl<'g> HeapPage<'g> {
    /// Returns `page` as a heap page of any owner.
    pub fn any(page: &'g impl Owned) -> Option<HeapPage<'g>> {
        Kinded::of(page)?.framed()
    }

    pub fn owner(self) -> u32 {
        self.owner
    }

    pub fn bytes(self) -> &'g [u8] {
        self.bytes
    }

    pub fn to_copy(self) -> PageCopy {
        PageCopy {
            owner: self.owner,
            bytes: self.bytes.to_vec(),
        }
    }

    /// Returns what is wrong with the slots of the page.
    pub fn check(self) -> Vec<String> {
        Kinded {
            bytes: self.bytes,
            owner: self.owner,
        }
        .check()
    }

    pub fn arrangement(self) -> Arrangement {
        arrangement(self.bytes)
    }

    /// Returns the page's index in its heap's directory.
    pub fn position(self) -> u32 {
        position(self.bytes)
    }

    /// Returns whether the page header's all-visible flag is set.
    pub fn all_visible(self) -> bool {
        storage::flags(self.bytes) & ALL_VISIBLE != 0
    }

    /// Returns whether no slot of the page holds anything.
    pub fn is_empty(self) -> bool {
        (0..self.slot_count()).all(|s| self.get(s) == Slot::Unused)
    }

    pub fn slot_count(self) -> usize {
        match self.arrangement() {
            Arrangement::Fixed { .. } => fixed::count(self.bytes),
            Arrangement::Slotted => slotted::count(self.bytes),
        }
    }

    pub fn get(self, slot: usize) -> Slot {
        match self.arrangement() {
            Arrangement::Fixed { .. } => fixed::get(self.bytes, slot),
            Arrangement::Slotted => slotted::get(self.bytes, slot),
        }
    }

    /// Returns the bytes free for new versions.
    pub fn free(self) -> usize {
        match self.arrangement() {
            Arrangement::Fixed { .. } => fixed::free(self.bytes),
            Arrangement::Slotted => slotted::free(self.bytes),
        }
    }

    /// Returns a copy of the page with its header naming row format `format` and arrangement
    /// `arrangement`, and its contents formatted empty at the page's position.
    pub fn reformat(self, format: u16, arrangement: Arrangement) -> PageCopy {
        let mut bytes = self.bytes.to_vec();
        let header = storage::header_edit(
            format,
            arrangement.kind(),
            self.owner,
            storage::flags(self.bytes),
        );
        bytes[storage::FIRST_EDITABLE..HEADER_LEN].copy_from_slice(&header);
        arrangement.init(&mut bytes, self.position());
        PageCopy {
            owner: self.owner,
            bytes,
        }
    }
}

/// A heap page, as [`HeapPage`], open to change.
#[derive(Debug)]
pub(super) struct HeapPageMut<'g> {
    bytes: &'g mut [u8],
    owner: u32,
}

impl<'g> HeapPageMut<'g> {
    /// Returns `page` as a heap page of any owner, open to change.
    pub fn any(page: &'g mut PageCopy) -> Option<HeapPageMut<'g>> {
        let owner = page.owner;
        HeapPage::any(&*page)?;
        Some(HeapPageMut {
            bytes: &mut page.bytes,
            owner,
        })
    }

    pub fn view(&self) -> HeapPage<'_> {
        HeapPage {
            bytes: self.bytes,
            owner: self.owner,
        }
    }

    /// Returns the bytes `at`, a range of a version [`HeapPage::get`] returned.
    pub fn version_mut(&mut self, at: Range<usize>) -> &mut [u8] {
        &mut self.bytes[at]
    }

    pub fn set_all_visible(&mut self, on: bool) {
        let flags = storage::flags(self.bytes);
        let flags = if on {
            flags | ALL_VISIBLE
        } else {
            flags & !ALL_VISIBLE
        };
        self.bytes[PAGE_FLAGS..PAGE_FLAGS + 2].copy_from_slice(&flags.to_le_bytes());
    }

    pub fn set_flags(&mut self, slot: usize, flags: Flags) {
        match self.view().arrangement() {
            Arrangement::Fixed { .. } => fixed::set_flags(self.bytes, slot, flags),
            Arrangement::Slotted => slotted::set_flags(self.bytes, slot, flags),
        }
    }

    /// Write `version` into a free slot. Returns the slot, or `None` if the page lacks the room.
    pub fn insert(&mut self, version: &[u8], flags: Flags) -> Option<usize> {
        match self.view().arrangement() {
            Arrangement::Fixed { .. } => fixed::insert(self.bytes, version, flags),
            Arrangement::Slotted => slotted::insert(self.bytes, version, flags),
        }
    }

    /// Move a slotted page's tuples together at its end. A fixed-slot page is left as it is.
    pub fn compact(&mut self) {
        if self.view().arrangement() == Arrangement::Slotted {
            slotted::compact(self.bytes);
        }
    }

    /// Empty slot `slot`, leaving it unused, dead or a redirect as `content` says.
    ///
    /// Panic if `content` is a version, or a dead slot or redirect on a fixed-slot page.
    pub fn release(&mut self, slot: usize, content: &Slot) {
        let page = &mut *self.bytes;
        match (arrangement(page), content) {
            (Arrangement::Fixed { .. }, Slot::Unused) => fixed::release(page, slot),
            (Arrangement::Slotted, Slot::Unused) => slotted::release(page, slot, None),
            (Arrangement::Slotted, Slot::Dead) => {
                slotted::release(page, slot, Some(slotted::Content::Dead));
            }
            (Arrangement::Slotted, Slot::Redirect(to)) => {
                slotted::release(page, slot, Some(slotted::Content::Redirect(*to)));
            }
            (a, c) => panic!("a {a:?} page cannot release a slot to {c:?}"),
        }
    }

    /// Fill the unused slot `slot` with what `content` holds. Returns `false`, changing nothing,
    /// when the page cannot.
    pub fn put(&mut self, slot: usize, content: &Slot, version: &[u8]) -> bool {
        let page = &mut *self.bytes;
        match (arrangement(page), content) {
            (Arrangement::Fixed { .. }, Slot::Normal { flags, .. }) => {
                fixed::put(page, slot, version, *flags)
            }
            (Arrangement::Fixed { .. }, Slot::Dead | Slot::Redirect(_)) => false,
            (Arrangement::Slotted, Slot::Normal { flags, .. }) => {
                slotted::put(page, slot, slotted::Content::Version(version, *flags))
            }
            (Arrangement::Slotted, Slot::Dead) => slotted::put(page, slot, slotted::Content::Dead),
            (Arrangement::Slotted, Slot::Redirect(to)) => {
                slotted::put(page, slot, slotted::Content::Redirect(*to))
            }
            (Arrangement::Fixed { .. } | Arrangement::Slotted, Slot::Unused) => true,
        }
    }
}

fn has_heap_kind(page: &[u8]) -> bool {
    matches!(
        PageKind::from_u16(storage::kind(page)),
        Some(PageKind::HeapFixed | PageKind::HeapSlotted)
    )
}

/// Returns whether the slot table of `page`, a page of a heap kind, fits the page.
fn framed(page: &[u8]) -> bool {
    match arrangement(page) {
        Arrangement::Fixed { .. } => fixed::framed(page),
        Arrangement::Slotted => slotted::framed(page),
    }
}

/// Returns the arrangement of a heap page.
///
/// Panics if `page` is not a heap page.
fn arrangement(page: &[u8]) -> Arrangement {
    match PageKind::from_u16(storage::kind(page)) {
        Some(PageKind::HeapFixed) => Arrangement::Fixed {
            width: fixed::width(page),
        },
        Some(PageKind::HeapSlotted) => Arrangement::Slotted,
        k => panic!("page of kind {k:?} is not a heap page"),
    }
}

fn position(page: &[u8]) -> u32 {
    u32_at(page, POSITION)
}
