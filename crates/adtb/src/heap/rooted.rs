use std::collections::BTreeMap;

use crate::key::{Tid, Value};
use crate::runtime::Runtime;
use crate::storage::{self, PageError, Pager, Pid};

use super::handle::{Heap, Version, tid};
use super::page::{self, Slot};
use super::version::{self, Flags};

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
            let page = pager.read_formatted(pid)?;
            if !page::is_heap(&page) || storage::owner(&page) != self.id {
                return Ok(());
            }
            let mut roots: BTreeMap<usize, Vec<Tid>> = BTreeMap::new();
            for slot in 0..page::slot_count(&page) {
                let start = match page::get(&page, slot) {
                    Slot::Redirect(to) => usize::from(to),
                    Slot::Normal { flags, .. } if !flags.contains(Flags::HEAP_ONLY) => slot,
                    _ => continue,
                };
                for member in version::chain(start, |s| version::link(&page, pid.0, s)) {
                    roots.entry(member).or_default().push(tid(pid, slot));
                }
            }
            for v in self.versions_on(pager, pid, &page)? {
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
    /// the slot holds no version: unused, dead, or a page no longer of the heap.
    pub fn chain_rows<R: Runtime>(
        &self,
        pager: &Pager<R>,
        at: Tid,
    ) -> Result<Option<Vec<Vec<Value>>>, PageError> {
        let pid = Pid(at.page());
        let page = pager.read(pid)?;
        if !page::is_heap(&page) || storage::owner(&page) != self.id {
            return Ok(None);
        }
        let format = storage::format(&page);
        let start = match page::get(&page, usize::from(at.slot())) {
            Slot::Redirect(to) => usize::from(to),
            Slot::Normal { .. } => usize::from(at.slot()),
            _ => return Ok(None),
        };
        let mut rows = Vec::new();
        for slot in version::chain(start, |s| version::link(&page, pid.0, s)) {
            let Slot::Normal { at: bytes, .. } = page::get(&page, slot) else {
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
        let page = pager.read(pid)?;
        if !page::is_heap(&page) || storage::owner(&page) != self.id {
            return Ok(Vec::new());
        }
        Ok(version::link_faults(&page, pid.0))
    }

    /// Mark the version at `at` hot-updated with successor `next`. Plants a fault for a test of
    /// the checks.
    #[doc(hidden)]
    pub fn plant_next<R: Runtime>(
        &self,
        pager: &Pager<R>,
        at: Tid,
        next: Tid,
    ) -> Result<(), PageError> {
        pager.update(&[Pid(at.page())], |pages| {
            let page = &mut *pages[0];
            let slot = usize::from(at.slot());
            let Slot::Normal { flags, at: bytes } = page::get(page, slot) else {
                panic!("a planted link starts at a version");
            };
            let mut header = version::Header::read(&page[bytes.clone()]);
            header.next = Some(next);
            header.write(&mut page[bytes]);
            page::set_flags(page, slot, flags.with(Flags::HOT_UPDATED));
        })?;
        Ok(())
    }
}
