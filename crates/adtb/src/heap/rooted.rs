use std::collections::{BTreeMap, BTreeSet};

use crate::key::{Tid, Value};
use crate::runtime::Runtime;
use crate::storage::{self, PageError, Pager, Pid};

use super::handle::{Heap, Version, tid};
use super::page::{self, Slot};
use super::version::{Flags, Header};

impl Heap {
    /// Call `f` on every version on `pid` with the slot index entries name for it: the root of
    /// the chain of heap-only successors it lies on, as one latched read of the page shows. A page
    /// no longer of the heap has none.
    pub fn scan_rooted<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        mut f: impl FnMut(Version, Tid),
    ) -> Result<(), PageError> {
        let mut found = Vec::new();
        {
            let page = pager.read(pid)?;
            if !page::is_heap(&page) || storage::owner(&page) != self.id {
                return Ok(());
            }
            let format = storage::format(&page);
            let mut roots = BTreeMap::new();
            for slot in 0..page::slot_count(&page) {
                let start = match page::get(&page, slot) {
                    Slot::Redirect(to) => usize::from(to),
                    Slot::Normal { flags, .. } if !flags.contains(Flags::HEAP_ONLY) => slot,
                    _ => continue,
                };
                let mut at = start;
                while let Slot::Normal { flags, at: bytes } = page::get(&page, at) {
                    if roots.insert(at, slot).is_some() || !flags.contains(Flags::HOT_UPDATED) {
                        break;
                    }
                    let Some(next) = Header::read(&page[bytes])
                        .next
                        .filter(|n| n.page() == pid.0)
                    else {
                        break;
                    };
                    at = usize::from(next.slot());
                }
            }
            for slot in 0..page::slot_count(&page) {
                let Slot::Normal { flags, at } = page::get(&page, slot) else {
                    continue;
                };
                let bytes = &page[at];
                let (row, _) = self.decode(pager, format, bytes)?;
                let root = roots.get(&slot).copied().unwrap_or(slot);
                found.push((
                    Version {
                        tid: tid(pid, slot),
                        header: Header::read(bytes),
                        flags,
                        row: self.upgrade(format, row),
                    },
                    tid(pid, root),
                ));
            }
        }
        for (v, root) in found {
            f(v, root);
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
        let mut slot = match page::get(&page, usize::from(at.slot())) {
            Slot::Redirect(to) => usize::from(to),
            Slot::Normal { .. } => usize::from(at.slot()),
            _ => return Ok(None),
        };
        let mut rows = Vec::new();
        let mut seen = BTreeSet::new();
        while seen.insert(slot) {
            let Slot::Normal { flags, at: bytes } = page::get(&page, slot) else {
                break;
            };
            let (row, _) = self.decode(pager, format, &page[bytes.clone()])?;
            rows.push(self.upgrade(format, row));
            let next = Header::read(&page[bytes])
                .next
                .filter(|n| n.page() == pid.0);
            match next.filter(|_| flags.contains(Flags::HOT_UPDATED)) {
                Some(n) => slot = usize::from(n.slot()),
                None => break,
            }
        }
        Ok(Some(rows))
    }
}
