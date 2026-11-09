use std::collections::BTreeMap;

use crate::key::{Tid, Value};
use crate::runtime::Runtime;
use crate::storage::{PageError, Pager, Pid};

use super::chain;
use super::handle::{Heap, Version, tid};
#[cfg(any(test, feature = "testing"))]
use super::page::HeapPageMut;
use super::page::Slot;
#[cfg(any(test, feature = "testing"))]
use super::stage::Stage;
#[cfg(any(test, feature = "testing"))]
use super::version::{Flags, Header};
#[cfg(any(test, feature = "testing"))]
use super::write::Rank;

impl Heap {
    /// Call `f` on every version on `pid` with each slot index entries may name for it: the roots
    /// of the chains of heap-only successors it lies on, as one latched read of the page shows. A
    /// sound page gives a version one root. A page no longer of the heap has none.
    pub fn scan_rooted<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        mut f: impl FnMut(Version, Vec<Tid>),
    ) -> Result<(), PageError> {
        let mut found = Vec::new();
        {
            let guard = pager.read_formatted(pid)?;
            let Some(page) = self.page(&guard) else {
                return Ok(());
            };
            let Some(format) = self.format_of(page) else {
                return Err(Heap::refuse_format(pager, pid, guard));
            };
            let mut roots: BTreeMap<usize, Vec<Tid>> = BTreeMap::new();
            for (root, members) in chain::roots(page, |s| chain::link(page, pid.0, s)) {
                for member in members {
                    roots.entry(member).or_default().push(tid(pid, root));
                }
            }
            for v in self.versions_on(pager, pid, page, format)? {
                let rooted = roots.remove(&usize::from(v.tid.slot()));
                let root = rooted.unwrap_or_else(|| vec![v.tid]);
                found.push((v, root));
            }
        }
        for (v, roots) in found {
            f(v, roots);
        }
        Ok(())
    }

    /// Returns the rows of the chain index entries naming `at` reach on its page, or `None` when
    /// the slot leads to no version: unused, dead, a redirect to no version, or on a page no longer
    /// of the heap.
    pub fn chain_rows<R: Runtime>(
        &self,
        pager: &Pager<R>,
        at: Tid,
    ) -> Result<Option<Vec<Vec<Value>>>, PageError> {
        let pid = Pid(at.page());
        let guard = pager.read(pid)?;
        let Some(page) = self.page(&guard) else {
            return Ok(None);
        };
        let Some(format) = self.format_of(page) else {
            return Err(Heap::refuse_format(pager, pid, guard));
        };
        let link = |s| chain::link(page, pid.0, s);
        let Some(members) = chain::from(page, usize::from(at.slot()), link) else {
            return Ok(None);
        };
        let mut rows = Vec::new();
        for slot in members {
            let Slot::Normal { at: bytes, .. } = page.get(slot) else {
                break;
            };
            let (row, _) = self.decode(pager, format, &page[bytes])?;
            rows.push(self.upgrade(format, row));
        }
        Ok(Some(rows))
    }

    /// Returns what is wrong with the links of `pid`, as one latched read of the page shows. A
    /// page no longer of the heap has nothing wrong.
    pub fn broken_links<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
    ) -> Result<Vec<String>, PageError> {
        let guard = pager.read(pid)?;
        Ok(self
            .page(&guard)
            .map_or_else(Vec::new, |page| chain::link_faults(page, pid.0)))
    }

    /// Mark the version at `at` hot-updated with successor `next`. Plants a fault for a test of
    /// the checks.
    #[doc(hidden)]
    #[cfg(any(test, feature = "testing"))]
    pub fn plant_next<R: Runtime>(
        &self,
        pager: &Pager<R>,
        at: Tid,
        next: Tid,
    ) -> Result<(), PageError> {
        let mut stage = Stage::new();
        let copy = stage.page(pager, Rank::Heap(Pid(at.page())))?;
        let mut page = HeapPageMut::any(copy).expect("a planted link starts on a heap page");
        let slot = usize::from(at.slot());
        let Slot::Normal { flags, at: bytes } = page.view().get(slot) else {
            panic!("a planted link starts at a version");
        };
        let mut header = Header::read(&page.view()[bytes.clone()]);
        header.next = Some(next);
        header.write(page.version_mut(bytes));
        page.set_flags(slot, flags.with(Flags::HOT_UPDATED));
        stage.commit(pager)
    }
}
