use crate::key::Tid;
use crate::runtime::Runtime;
use crate::storage::{self, PageError, Pager, Pid};

use super::handle::Heap;
use super::page::{self, Slot};

/// An index entry naming an unused slot of a page its heap still holds, or a page its heap's
/// directory does not name: damage to the index with owner `index`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrphanEntry {
    pub index: u32,
}

/// Refuse a read through an entry of the index with owner `index`, naming TID `tid` of heap
/// `heap`, when the entry is still there and `tid` names no slot of `heap` both before and after
/// `named` finds it. A page of another owner or kind that `heap`'s directory names is refused as
/// [`Space::refuse_misnamed`](super::dir::Space::refuse_misnamed) does; an unused slot on a page
/// of `heap`, or a page its directory does not name, refuses the index as an [`OrphanEntry`],
/// leaving the page readable.
///
/// Preconditions: the caller holds no page latch.
#[expect(clippy::disallowed_methods, reason = "until heap owns page checks")]
pub fn refuse_named<R: Runtime, E: From<PageError> + From<OrphanEntry>>(
    pager: &Pager<R>,
    (heap, tid): (&Heap, Tid),
    index: u32,
    named: impl FnOnce() -> Result<bool, E>,
) -> Result<(), E> {
    let pid = Pid(tid.page());
    let gone = || -> Result<Option<bool>, PageError> {
        let page = pager.read_formatted(pid)?;
        if !page::is_heap(&page) || storage::owner(&page) != heap.id().0 {
            return Ok(Some(true));
        }
        Ok((page::get(&page, usize::from(tid.slot())) == Slot::Unused).then_some(false))
    };
    let Some(foreign) = gone()? else {
        return Ok(());
    };
    if !named()? || gone()? != Some(foreign) {
        return Ok(());
    }
    if foreign && heap.space.refuse_misnamed(pager, pid)? {
        return Ok(());
    }
    Err(OrphanEntry { index }.into())
}
