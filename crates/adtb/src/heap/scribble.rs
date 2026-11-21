//! Edits a test makes to the pages of heaps, their directories and large-value stores, beneath
//! the writes that keep them sound.

use crate::mvcc::{Writers, Xid};
use crate::runtime::Runtime;
use crate::storage::{self, FIRST_EDITABLE, HEADER_LEN, PageError, PageKind, Pager, Pid};

use super::page::{HeapPageMut, Owned, PageCopy, Slot};
use super::version::Header;
use super::{dir, large, slotted};

/// An edit of one page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Relabel the page as of `kind`.
    Kind(PageKind),
    /// Make entry `at` of a directory page name `names`, raising the page's count to hold it.
    Entry { at: usize, names: Pid },
    /// Make a directory root name no visibility map.
    NoMap,
    /// Give slot `slot` of a slotted heap page a state no slot holds.
    Broken(usize),
    /// Make the version in slot `slot` of a heap page deleted by `by`.
    Deleter { slot: usize, by: Xid },
    /// Make the version in slot `slot` of a heap page inserted by `by`, keeping its deleter.
    Inserter { slot: usize, by: Xid },
    /// Empty the version in slot `slot` of a heap page, leaving the slot unused.
    Vacate(usize),
    /// Make the chunk in slot `slot` of a large-value store page count `count` versions.
    Count { slot: usize, count: u32 },
}

/// Apply `edit` to page `pid` as one logged change. Returns `false`, changing nothing, when the
/// page holds nothing the edit applies to.
pub fn scribble<R: Runtime>(pager: &Pager<R>, pid: Pid, edit: Edit) -> Result<bool, PageError> {
    let read = pager.read(pid)?;
    let mut copy = PageCopy::owned_by(read.owner(), read.to_vec());
    drop(read);
    if !edited(&mut copy, edit) {
        return Ok(false);
    }
    let bytes = copy.into_bytes();
    pager.update(&[pid], |p| {
        p[0][FIRST_EDITABLE..].copy_from_slice(&bytes[FIRST_EDITABLE..]);
    })?;
    Ok(true)
}

fn edited(copy: &mut PageCopy, edit: Edit) -> bool {
    match edit {
        Edit::Kind(kind) => {
            let header = storage::header_edit(
                storage::format(copy),
                kind,
                copy.owner(),
                storage::flags(copy),
            );
            copy[FIRST_EDITABLE..HEADER_LEN].copy_from_slice(&header);
        }
        Edit::Entry { at, names } => dir::plant_entry(copy, at, names),
        Edit::NoMap => dir::plant_no_map(copy),
        Edit::Broken(slot) => {
            let slotted = storage::kind(copy) == PageKind::HeapSlotted as u16;
            if !slotted || slot >= slotted::count(copy) {
                return false;
            }
            slotted::break_pointer(copy, slot);
        }
        Edit::Deleter { slot, by } => {
            let Some((mut page, at)) = version(copy, slot) else {
                return false;
            };
            let mut header = Header::read(&page.view()[at.clone()]);
            header.writers = header.writers.superseded_by(by);
            header.write(page.version_mut(at));
        }
        Edit::Inserter { slot, by } => {
            let Some((mut page, at)) = version(copy, slot) else {
                return false;
            };
            let mut header = Header::read(&page.view()[at.clone()]);
            let inserted = Writers::inserted(by);
            header.writers = match header.writers.deleter() {
                Some(d) => inserted.superseded_by(d),
                None => inserted,
            };
            header.write(page.version_mut(at));
        }
        Edit::Vacate(slot) => {
            let Some((mut page, _)) = version(copy, slot) else {
                return false;
            };
            page.release(slot, &Slot::Unused);
        }
        Edit::Count { slot, count } => {
            let Some((mut page, at)) = version(copy, slot) else {
                return false;
            };
            let tuple = page.version_mut(at);
            tuple[large::COUNT..large::COUNT + 4].copy_from_slice(&count.to_le_bytes());
        }
    }
    true
}

/// Returns the page of `copy` open to change and the bytes of the version in `slot`.
fn version(copy: &mut PageCopy, slot: usize) -> Option<(HeapPageMut<'_>, std::ops::Range<usize>)> {
    let page = HeapPageMut::any(copy)?;
    let Slot::Normal { at, .. } = page.view().get(slot) else {
        return None;
    };
    Some((page, at))
}
