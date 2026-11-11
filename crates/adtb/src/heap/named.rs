use crate::key::Tid;
use crate::ranks;
use crate::runtime::Runtime;
use crate::storage::{PageError, Pager, Pid};

use super::handle::Heap;
use super::page::Slot;

/// An index entry naming an unused slot of a page its heap still holds, or a page its heap's
/// directory does not name: damage to the index with owner `index`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrphanEntry {
    pub index: u32,
}

#[cfg(any(test, feature = "testing"))]
std::thread_local! {
    /// What the calling thread's next [`refuse_named`] runs before it first looks at the slot.
    static BEFORE_LOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

/// Run `hook` once on the calling thread, at its next [`refuse_named`], before that first looks
/// at the slot.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn before_next_refusal(hook: impl FnOnce() + 'static) {
    BEFORE_LOOK.with(|b| *b.borrow_mut() = Some(Box::new(hook)));
}

/// Returns whether `tid` names no slot of `heap`: `Some(true)` when its page is not a page of
/// `heap`, `Some(false)` when its slot is unused, `None` when the slot is in use.
pub fn vacant<R: Runtime>(
    pager: &Pager<R>,
    (heap, tid): (&Heap, Tid),
) -> Result<Option<bool>, PageError> {
    let guard = pager.read_formatted(Pid(tid.page()))?;
    let Some(page) = heap.page(&guard) else {
        return Ok(Some(true));
    };
    Ok((page.get(usize::from(tid.slot())) == Slot::Unused).then_some(false))
}

/// Refuse a read through an entry of the index with owner `index`, naming TID `tid` of heap
/// `heap`, when the entry is still there and `tid` names no slot of `heap` both before and after
/// `named` finds it. A page of another owner or kind that `heap`'s directory names is refused as
/// [`Space::refuse_misnamed`](super::dir::Space::refuse_misnamed) does; an unused slot on a page
/// of `heap`, or a page its directory does not name, refuses the index as an [`OrphanEntry`],
/// leaving the page readable.
///
/// Preconditions: the caller holds no page latch and no index latch.
pub fn refuse_named<R: Runtime, E: From<PageError> + From<OrphanEntry>>(
    pager: &Pager<R>,
    (heap, tid): (&Heap, Tid),
    index: u32,
    named: impl FnOnce() -> Result<bool, E>,
) -> Result<(), E> {
    let latched = pager.latched_here();
    assert!(
        latched == 0,
        "an entry is refused while this thread latches {latched} pages"
    );
    assert!(
        !ranks::INDEX_LATCH.held_here(),
        "an entry is refused under an index latch"
    );
    #[cfg(any(test, feature = "testing"))]
    if let Some(hook) = BEFORE_LOOK.with(|b| b.borrow_mut().take()) {
        hook();
    }
    let pid = Pid(tid.page());
    let gone = || vacant(pager, (heap, tid));
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
