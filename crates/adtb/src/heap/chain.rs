use std::borrow::Borrow;
use std::collections::BTreeSet;

use crate::key::{Tid, Value};
use crate::runtime::Runtime;
use crate::storage::{PageError, Pager, Pid};

use super::handle::{Heap, HeapNo, Link, tid};
use super::page::{HeapPage, Owned, Slot};
use super::version::{Flags, Header};

/// Returns the slot on `page`, page `pid` of its heap, holding the heap-only successor the version
/// in `slot` links to, if any.
pub(super) fn link(page: HeapPage<'_>, pid: u64, slot: usize) -> Option<usize> {
    let Slot::Normal { flags, at } = page.get(slot) else {
        return None;
    };
    if !flags.contains(Flags::HOT_UPDATED) {
        return None;
    }
    let next = Header::read(&page[at]).next.filter(|n| n.page() == pid)?;
    let to = usize::from(next.slot());
    matches!(page.get(to), Slot::Normal { flags, .. } if flags.contains(Flags::HEAP_ONLY))
        .then_some(to)
}

/// Returns the slots of the chain from `start`, following `link` until it ends or a slot repeats.
fn chain(start: usize, link: impl Fn(usize) -> Option<usize>) -> Vec<usize> {
    let mut members = vec![start];
    while let Some(to) = link(*members.last().unwrap()) {
        if members.contains(&to) {
            break;
        }
        members.push(to);
    }
    members
}

/// Returns the slot holding the first version of the chain at `slot` of `page`: `slot` when it
/// holds a version, or the slot a redirect there names when that holds one.
fn start(page: HeapPage<'_>, slot: usize) -> Option<usize> {
    match page.get(slot) {
        Slot::Normal { .. } => Some(slot),
        Slot::Redirect(to) if matches!(page.get(usize::from(to)), Slot::Normal { .. }) => {
            Some(usize::from(to))
        }
        _ => None,
    }
}

/// Returns the slots of the chain an index entry naming `slot` of `page` reaches, following
/// `link` from the version [`start`] finds, or `None` when it finds none.
pub(super) fn from(
    page: HeapPage<'_>,
    slot: usize,
    link: impl Fn(usize) -> Option<usize>,
) -> Option<Vec<usize>> {
    start(page, slot).map(|s| chain(s, link))
}

/// Returns each chain root of `page`, a slot index entries may name, with the slots of the chain
/// [`from`] reaches from it. A root holds a version that is not heap-only, or redirects to a
/// version.
pub(super) fn roots(
    page: HeapPage<'_>,
    link: impl Fn(usize) -> Option<usize>,
) -> Vec<(usize, Vec<usize>)> {
    (0..page.slot_count())
        .filter(|&s| {
            !matches!(page.get(s), Slot::Normal { flags, .. } if flags.contains(Flags::HEAP_ONLY))
        })
        .filter_map(|s| from(page, s, &link).map(|members| (s, members)))
        .collect()
}

/// Returns what is wrong with the links of `page`, page `pid` of its heap: each hot-updated slot
/// that links to no version its deleter wrote, and each version two slots link to.
pub(super) fn link_faults(page: HeapPage<'_>, pid: u64) -> Vec<String> {
    let header = |s| match page.get(s) {
        Slot::Normal { at, .. } => Some(Header::read(&page[at])),
        _ => None,
    };
    let mut faults = Vec::new();
    let mut successors = BTreeSet::new();
    let mut shared = BTreeSet::new();
    for slot in 0..page.slot_count() {
        if !matches!(page.get(slot), Slot::Normal { flags, .. } if flags.contains(Flags::HOT_UPDATED))
        {
            continue;
        }
        let written = link(page, pid, slot).filter(
            |&to| matches!((header(slot), header(to)), (Some(h), Some(s)) if h.writers.precedes(&s.writers)),
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

/// Returns whether `link` can be the version after `prior` in a chain starting in heap `heap`
/// and running across the heaps `in_sum` accepts.
fn follows(
    prior: Option<&Link>,
    heap: HeapNo,
    in_sum: impl Fn(HeapNo) -> bool,
    link: &Link,
) -> bool {
    let Some(p) = prior else {
        return link.heap == heap;
    };
    let placed = if p.flags.contains(Flags::MOVED) {
        link.heap != p.heap && in_sum(link.heap) && link.flags.contains(Flags::RETAGGED_IN)
    } else {
        link.heap == p.heap
    };
    placed && p.header.writers.precedes(&link.header.writers)
}

/// Returns the heap of `sum` holding `page`, with the page.
fn holder<'s, 'g>(
    sum: &'s [impl Borrow<Heap>],
    page: &'g impl Owned,
) -> Option<(&'s Heap, HeapPage<'g>)> {
    let page = HeapPage::any(page)?;
    let heap = sum
        .iter()
        .map(Borrow::borrow)
        .find(|h| h.id().0 == page.owner())?;
    Some((heap, page))
}

/// Returns the version at `at` on `page`, page `at.page()` held by `heap`, as a link, following a
/// redirect to the version it names. Returns `None` when the slot leads to no version.
fn link_at(page: HeapPage<'_>, heap: HeapNo, at: Tid) -> Option<(Link, std::ops::Range<usize>)> {
    let slot = start(page, usize::from(at.slot()))?;
    let Slot::Normal { flags, at: range } = page.get(slot) else {
        unreachable!("a chain starts at a version")
    };
    let link = Link {
        tid: tid(Pid(at.page()), slot),
        heap,
        header: Header::read(&page[range.clone()]),
        flags,
    };
    Some((link, range))
}

/// Returns the successor of `of` in its chain across the heaps of `sum`, placed as [`walk`]
/// requires. Returns `None` when `of` names none, or names a slot holding no such version, a
/// redirect, or a slot on a page out of shape.
///
/// Precondition: `sum` holds the heap of every constructor of `of`'s relation.
pub fn successor<R: Runtime>(
    pager: &Pager<R>,
    sum: &[impl Borrow<Heap>],
    of: &Link,
) -> Result<Option<Link>, PageError> {
    let Some(next) = of.header.next else {
        return Ok(None);
    };
    let guard = pager.read(Pid(next.page()))?;
    let Some((heap, page)) = holder(sum, &guard) else {
        return Ok(None);
    };
    if !page.check().is_empty() {
        return Ok(None);
    }
    let in_sum = |h| sum.iter().any(|s| s.borrow().id() == h);
    Ok(link_at(page, heap.id(), next)
        .map(|(link, _)| link)
        .filter(|link| link.tid == next && follows(Some(of), of.heap, in_sum, link)))
}

/// The versions of one stretch of a chain on one page, each with its row, and where the chain
/// goes on.
type Stretch = (Vec<(Link, Vec<Value>)>, Option<Tid>);

/// Returns the versions of the chain from `at` that lie on its page, read and decoded under one
/// shared latch of that page, after `prior`, and the version on another page the chain goes on
/// to. A version in `seen` ends the chain; each version returned is added to it.
fn stretch<R: Runtime>(
    pager: &Pager<R>,
    (heap, sum): (HeapNo, &[impl Borrow<Heap>]),
    seen: &mut BTreeSet<Tid>,
    mut prior: Option<Link>,
    mut at: Tid,
) -> Result<Stretch, PageError> {
    let pid = at.page();
    let guard = pager.read_formatted(Pid(pid))?;
    let mut found = Vec::new();
    let Some((holder, page)) = holder(sum, &guard) else {
        return Ok((found, None));
    };
    let in_sum = |h| sum.iter().any(|s| s.borrow().id() == h);
    let Some(format) = holder.format_of(page) else {
        return Err(Heap::refuse_format(pager, Pid(pid), guard));
    };
    loop {
        let Some((link, range)) = link_at(page, holder.id(), at) else {
            return Ok((found, None));
        };
        if !seen.insert(link.tid) || !follows(prior.as_ref(), heap, in_sum, &link) {
            return Ok((found, None));
        }
        let (row, _) = holder.decode(pager, format, &page[range])?;
        found.push((link, holder.upgrade(format, row)));
        match link.header.next {
            Some(next) if next.page() == pid => at = next,
            next => return Ok((found, next)),
        }
        prior = Some(link);
    }
}

/// Follow the chain from the version at `start`, in heap `heap`, through each `next`, across
/// the heaps of `sum`, and return the first version `accept` accepts, with its row in its heap's
/// newest format. Returns `None` when the chain ends first, and the first error `accept` returns.
///
/// Precondition: `sum` holds the heap of every constructor of `heap`'s relation.
///
/// The chain ends at a slot leading to no version, at a page no heap of `sum` holds, at a version
/// whose inserter is not the deleter of the version before it, at a version already visited, and
/// at a version on a page of the wrong heap: the first must be in `heap`, the successor of a moved
/// version in another heap of `sum` and flagged retagged in, and any other successor in its
/// predecessor's heap. A redirect is followed to the version it names. The versions of one
/// stretch of the chain on one page, rows and out-of-line values included, are read under one
/// shared latch of that page, and `accept` sees them after it is released.
pub fn walk<R: Runtime, E: From<PageError>>(
    pager: &Pager<R>,
    heap: HeapNo,
    sum: &[impl Borrow<Heap>],
    start: Tid,
    mut accept: impl FnMut(&Link, &[Value]) -> Result<bool, E>,
) -> Result<Option<(Link, Vec<Value>)>, E> {
    let mut at = start;
    let mut prior = None;
    let mut seen = BTreeSet::new();
    loop {
        let (found, next) = stretch(pager, (heap, sum), &mut seen, prior, at)?;
        prior = found.last().map(|(link, _)| *link);
        for (link, row) in found {
            if accept(&link, &row)? {
                return Ok(Some((link, row)));
            }
        }
        match next {
            Some(next) => at = next,
            None => return Ok(None),
        }
    }
}

impl Heap {
    /// Returns the version at `at` and its heap-only successors: the versions an index entry for
    /// `at` stands for, as one latched read of its page shows. Each version after the first is
    /// written by the deleter of the one before it. A page no longer of the heap has none.
    pub fn hot_run<R: Runtime>(&self, pager: &Pager<R>, at: Tid) -> Result<Vec<Link>, PageError> {
        let pid = Pid(at.page());
        let guard = pager.read_formatted(pid)?;
        let Some(page) = self.page(&guard) else {
            return Ok(Vec::new());
        };
        let members = from(page, usize::from(at.slot()), |s| link(page, pid.0, s));
        let mut run: Vec<Link> = Vec::new();
        for slot in members.into_iter().flatten() {
            let Some((link, _)) = link_at(page, self.id(), tid(pid, slot)) else {
                break;
            };
            if run
                .last()
                .is_some_and(|p| !p.header.writers.precedes(&link.header.writers))
            {
                break;
            }
            run.push(link);
        }
        Ok(run)
    }
}
