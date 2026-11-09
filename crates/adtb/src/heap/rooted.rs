use std::collections::BTreeMap;

use crate::key::{Tid, Value};
use crate::runtime::Runtime;
use crate::storage::{self, PageError, Pager, Pid};

use super::chain;
use super::handle::{Heap, Version, tid};
use super::page::{self, Slot};
#[cfg(any(test, feature = "testing"))]
use super::version::{Flags, Header};

impl Heap {
    /// Call `f` on every version on `pid` with each slot index entries may name for it: the roots
    /// of the chains of heap-only successors it lies on, as one latched read of the page shows. A
    /// sound page gives a version one root. A page no longer of the heap has none.
    #[expect(clippy::disallowed_methods, reason = "until heap owns page checks")]
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
            for (root, members) in chain::roots(&page, |s| chain::link(&page, pid.0, s)) {
                for member in members {
                    roots.entry(member).or_default().push(tid(pid, root));
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
    /// the slot leads to no version: unused, dead, a redirect to no version, or on a page no longer
    /// of the heap.
    #[expect(clippy::disallowed_methods, reason = "until heap owns page checks")]
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
        let link = |s| chain::link(&page, pid.0, s);
        let Some(members) = chain::from(&page, usize::from(at.slot()), link) else {
            return Ok(None);
        };
        let mut rows = Vec::new();
        for slot in members {
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
    #[expect(clippy::disallowed_methods, reason = "until heap owns page checks")]
    pub fn broken_links<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
    ) -> Result<Vec<String>, PageError> {
        let page = pager.read(pid)?;
        if !page::is_heap(&page) || storage::owner(&page) != self.id {
            return Ok(Vec::new());
        }
        Ok(chain::link_faults(&page, pid.0))
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
        pager.update(&[Pid(at.page())], |pages| {
            let page = &mut *pages[0];
            let slot = usize::from(at.slot());
            let Slot::Normal { flags, at: bytes } = page::get(page, slot) else {
                panic!("a planted link starts at a version");
            };
            let mut header = Header::read(&page[bytes.clone()]);
            header.next = Some(next);
            header.write(&mut page[bytes]);
            page::set_flags(page, slot, flags.with(Flags::HOT_UPDATED));
        })?;
        Ok(())
    }
}
