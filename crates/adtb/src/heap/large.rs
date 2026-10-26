use std::collections::BTreeSet;

use crate::key::Tid;
use crate::runtime::Runtime;
use crate::storage::{self, PageError, Pager, Pid};

use super::dir::{Space, category, max_fanout, need_category};
use super::page::{self, Arrangement, Slot};
use super::row::Pointer;
use super::slotted;
use super::stage::Stage;
use super::version::{Flags, tid_bits, tid_from_bits};

const NEXT: usize = 8;

/// The large-value store of a sum, shared by its constructor heaps.
///
/// Its pages are slotted heap pages owned by the store's id. Each tuple is a chunk of one value:
/// the TID of the next chunk, zero for the last, then bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LargeStore {
    space: Space,
}

impl LargeStore {
    /// Create an empty store; `id` comes from the heap id space.
    pub fn create<R: Runtime>(pager: &Pager<R>, id: u32) -> Result<LargeStore, PageError> {
        let fanout = max_fanout(pager.page_size());
        Ok(LargeStore {
            space: Space::create(pager, id, fanout, None)?,
        })
    }

    pub fn open(id: u32, root: Pid) -> LargeStore {
        LargeStore {
            space: Space::open(id, root),
        }
    }

    pub fn id(&self) -> u32 {
        self.space.owner()
    }

    /// Returns the root page of the store's directory.
    pub fn root(&self) -> Pid {
        self.space.root()
    }

    /// Returns every page of the store, in directory order.
    pub fn pages<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pid>, PageError> {
        self.space.pages(pager)
    }

    /// Store `bytes` and return the pointer to them. Chunks are written last first, so every
    /// chunk written names one already present.
    pub fn write<R: Runtime>(&self, pager: &Pager<R>, bytes: &[u8]) -> Result<Pointer, PageError> {
        let chunk = slotted::max_version(pager.page_size()) - NEXT;
        let mut next = None;
        for piece in bytes.chunks(chunk).rev() {
            let mut tuple = next.map_or(0, tid_bits).to_le_bytes().to_vec();
            tuple.extend_from_slice(piece);
            next = Some(self.insert(pager, &tuple)?);
        }
        Ok(Pointer {
            len: bytes.len() as u64,
            first: next.expect("an out-of-line value is not empty"),
        })
    }

    fn insert<R: Runtime>(&self, pager: &Pager<R>, tuple: &[u8]) -> Result<Tid, PageError> {
        let size = pager.page_size();
        let need = need_category(Arrangement::Slotted.need(tuple.len()), size);
        let pid = match self
            .space
            .find(&Stage::new(), pager, need, Arrangement::Slotted, 1)?
        {
            Some(pid) => pid,
            None => self.space.append(pager, Arrangement::Slotted, 1)?,
        };
        let mut stage = Stage::new();
        let page = stage.page(pager, pid)?;
        let slot = page::insert(page, tuple, Flags::NONE).expect("the directory found room");
        let position = page::position(page);
        let cat = category(page::free(page), size);
        self.space.set_category(&mut stage, pager, position, cat)?;
        stage.commit(pager)?;
        Ok(Tid::new(pid.0, slot as u16).expect("48-bit page id"))
    }

    /// Returns the value `pointer` names.
    pub fn read<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pointer: Pointer,
    ) -> Result<Vec<u8>, PageError> {
        let mut out = Vec::with_capacity(pointer.len as usize);
        let mut at = Some(pointer.first);
        while let Some(tid) = at {
            let page = &pager.read(Pid(tid.page()))?.to_vec();
            let Slot::Normal { at: range, .. } = page::get(page, usize::from(tid.slot())) else {
                panic!("chunk {tid:?} of a large value is missing");
            };
            let tuple = &page[range];
            at = tid_from_bits(u64::from_le_bytes(tuple[..NEXT].try_into().unwrap()));
            out.extend_from_slice(&tuple[NEXT..]);
        }
        assert_eq!(
            pointer.len,
            out.len() as u64,
            "large value has the wrong length"
        );
        Ok(out)
    }

    /// Returns the TID of every chunk of the value `pointer` names, first to last.
    pub fn chunks<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pointer: Pointer,
    ) -> Result<Vec<Tid>, PageError> {
        let mut out = Vec::new();
        let mut at = Some(pointer.first);
        while let Some(tid) = at {
            out.push(tid);
            let page = &pager.read(Pid(tid.page()))?.to_vec();
            let Slot::Normal { at: range, .. } = page::get(page, usize::from(tid.slot())) else {
                panic!("chunk {tid:?} of a large value is missing");
            };
            at = tid_from_bits(u64::from_le_bytes(page[range][..NEXT].try_into().unwrap()));
        }
        Ok(out)
    }

    /// Free every chunk not in `keep`, each page in one update, and remove the pages left empty
    /// from the store. Returns the number of chunks freed.
    ///
    /// Precondition: every chunk of a value some version points at is in `keep`, and no write
    /// that stores a value is in progress.
    pub fn sweep<R: Runtime>(
        &self,
        pager: &Pager<R>,
        keep: &BTreeSet<Tid>,
    ) -> Result<usize, PageError> {
        let size = pager.page_size();
        let mut freed = 0;
        for pid in self.space.pages(pager)? {
            let page = pager.read(pid)?.to_vec();
            let mut out = page.clone();
            for slot in 0..page::slot_count(&page) {
                let tid = Tid::new(pid.0, slot as u16).expect("48-bit page id");
                if matches!(page::get(&page, slot), Slot::Normal { .. }) && !keep.contains(&tid) {
                    page::release(&mut out, slot, &Slot::Unused);
                    freed += 1;
                }
            }
            if out == page {
                continue;
            }
            page::compact(&mut out);
            let position = page::position(&page);
            let empty = page::is_empty(&out);
            let mut stage = Stage::new();
            let cat = category(page::free(&out), size);
            stage.put(pid, out);
            if empty {
                self.space.vacate(&mut stage, pager, position)?;
            } else {
                self.space.set_category(&mut stage, pager, position, cat)?;
            }
            stage.commit(pager)?;
            if empty {
                pager.free(pid)?;
            }
        }
        Ok(freed)
    }

    /// Returns every page of the store's directory and every page it names.
    pub fn owned_pages<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pid>, PageError> {
        let mut out = self.space.nodes(pager)?;
        out.extend(self.space.pages(pager)?);
        Ok(out)
    }

    /// Returns what is wrong with the store's directory and pages.
    pub fn verify<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<String>, PageError> {
        Ok(self
            .check(pager)?
            .into_iter()
            .map(|(pid, what)| format!("{pid:?} {what}"))
            .collect())
    }

    /// Returns each page of the store found wrong, with what is wrong with it.
    pub fn check<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<(Pid, String)>, PageError> {
        let check = self.space.check(pager)?;
        let mut problems = check.problems;
        for (position, leaf) in check.leaves.into_iter().enumerate() {
            let Some((pid, cat)) = leaf else {
                continue;
            };
            let page = &pager.read(pid)?.to_vec();
            let ours = page::is_heap(page) && storage::owner(page) == self.id();
            if !ours || page::position(page) as usize != position {
                problems.push((
                    pid,
                    format!("is not page {position} of store {}", self.id()),
                ));
                continue;
            }
            let layout = page::check(page);
            if !layout.is_empty() {
                problems.extend(layout.into_iter().map(|what| (pid, what)));
                continue;
            }
            let actual = category(page::free(page), pager.page_size());
            if cat != actual {
                problems.push((
                    pid,
                    format!("has category {cat} in the directory, {actual} on the page"),
                ));
            }
        }
        Ok(problems)
    }
}
