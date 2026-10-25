use super::page::{CONTENTS, Slot, align8, u16_at, u64_at};
use super::version::Flags;

const WIDTH: usize = 28;
const COUNT: usize = 30;
const USED: usize = 32;

/// Returns the offset of slot 0 in a page of `n` slots.
fn slots_at(n: usize) -> usize {
    align8(USED + 8 * n.div_ceil(64) + 2 * n)
}

fn flags_at(n: usize, slot: usize) -> usize {
    USED + 8 * n.div_ceil(64) + 2 * slot
}

/// Returns the number of slots of `width` bytes a page of `page_size` bytes holds.
pub fn capacity(page_size: usize, width: usize) -> usize {
    let mut n = ((page_size - USED) / width).min(usize::from(u16::MAX));
    while n > 0 && slots_at(n) + n * width > page_size {
        n -= 1;
    }
    n
}

pub fn width(page: &[u8]) -> usize {
    usize::from(u16_at(page, WIDTH))
}

pub fn count(page: &[u8]) -> usize {
    usize::from(u16_at(page, COUNT))
}

/// Format `page` as an empty fixed-slot page of slots `width` bytes wide.
pub fn init(page: &mut [u8], width: usize) {
    let n = capacity(page.len(), width);
    assert!(n > 0, "a slot of {width} bytes does not fit a page");
    page[CONTENTS..].fill(0);
    page[WIDTH..WIDTH + 2].copy_from_slice(&(width as u16).to_le_bytes());
    page[COUNT..COUNT + 2].copy_from_slice(&(n as u16).to_le_bytes());
}

fn used(page: &[u8], slot: usize) -> bool {
    u64_at(page, USED + 8 * (slot / 64)) >> (slot % 64) & 1 == 1
}

fn set_used(page: &mut [u8], slot: usize, on: bool) {
    let at = USED + 8 * (slot / 64);
    let word = u64_at(page, at);
    let bit = 1 << (slot % 64);
    let word = if on { word | bit } else { word & !bit };
    page[at..at + 8].copy_from_slice(&word.to_le_bytes());
}

pub fn get(page: &[u8], slot: usize) -> Slot {
    let n = count(page);
    if slot >= n || !used(page, slot) {
        return Slot::Unused;
    }
    let w = width(page);
    let at = slots_at(n) + slot * w;
    Slot::Normal {
        flags: Flags(u16_at(page, flags_at(n, slot))),
        at: at..at + w,
    }
}

pub fn set_flags(page: &mut [u8], slot: usize, flags: Flags) {
    let at = flags_at(count(page), slot);
    page[at..at + 2].copy_from_slice(&flags.0.to_le_bytes());
}

/// Returns the bytes free for new versions.
pub fn free(page: &[u8]) -> usize {
    let n = count(page);
    (0..n).filter(|&s| !used(page, s)).count() * width(page)
}

/// Write `version` into slot `slot`, padded to the slot's width. Returns `false`, changing
/// nothing, when the slot does not exist, is taken, or is too narrow.
pub fn put(page: &mut [u8], slot: usize, version: &[u8], flags: Flags) -> bool {
    let (n, w) = (count(page), width(page));
    if slot >= n || used(page, slot) || version.len() > w {
        return false;
    }
    let at = slots_at(n) + slot * w;
    page[at..at + version.len()].copy_from_slice(version);
    page[at + version.len()..at + w].fill(0);
    set_used(page, slot, true);
    set_flags(page, slot, flags);
    true
}

/// Free slot `slot`, clearing its bit, its flags and its bytes.
pub fn release(page: &mut [u8], slot: usize) {
    let (n, w) = (count(page), width(page));
    let at = slots_at(n) + slot * w;
    page[at..at + w].fill(0);
    set_used(page, slot, false);
    set_flags(page, slot, Flags::NONE);
}

/// Write `version` into the first free slot. Returns the slot, or `None` if no slot is free.
pub fn insert(page: &mut [u8], version: &[u8], flags: Flags) -> Option<usize> {
    let slot = (0..count(page)).find(|&s| !used(page, s))?;
    put(page, slot, version, flags).then_some(slot)
}
