use std::collections::{BTreeMap, BTreeSet};

use crate::key::Tid;
use crate::runtime::Runtime;
use crate::storage::{self, PageError, Pager, Pid};

use super::dir::{Space, category, max_fanout, need_category};
use super::page::{self, Arrangement, Slot};
use super::row::Pointer;
use super::slotted;
use super::stage::Stage;
use super::version::{Flags, tid_bits, tid_from_bits};

const NEXT: usize = 0;
const XMIN: usize = 8;
const COUNT: usize = 16;
const ROLE: usize = 20;
const BYTES: usize = 24;

const HEAD: u32 = 0;
const TAIL: u32 = 1;

/// The header of one chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub next: Option<Tid>,
    /// The transaction that wrote the chunk.
    pub xmin: u64,
    /// The first chunk of a value, the one a pointer names; every other chunk is named by the
    /// chunk before it.
    pub head: bool,
    /// The number of versions that point at the value, kept in its first chunk.
    pub count: u32,
}

fn chunk(tuple: &[u8]) -> Chunk {
    let u64_at = |at: usize| u64::from_le_bytes(tuple[at..at + 8].try_into().unwrap());
    let u32_at = |at: usize| u32::from_le_bytes(tuple[at..at + 4].try_into().unwrap());
    Chunk {
        next: tid_from_bits(u64_at(NEXT)),
        xmin: u64_at(XMIN),
        head: u32_at(ROLE) == HEAD,
        count: u32_at(COUNT),
    }
}

fn set_chunk(tuple: &mut [u8], c: Chunk) {
    tuple[NEXT..NEXT + 8].copy_from_slice(&c.next.map_or(0, tid_bits).to_le_bytes());
    tuple[XMIN..XMIN + 8].copy_from_slice(&c.xmin.to_le_bytes());
    tuple[COUNT..COUNT + 4].copy_from_slice(&c.count.to_le_bytes());
    let role = if c.head { HEAD } else { TAIL };
    tuple[ROLE..ROLE + 4].copy_from_slice(&role.to_le_bytes());
}

/// What [`LargeStore::check_chunks`] found.
#[derive(Debug, Default)]
pub struct ChunkCheck {
    /// Each page found wrong, with what is wrong with it.
    pub problems: Vec<(Pid, String)>,
    /// Each head counting more versions than point at it, with its count and the number that
    /// do.
    pub overcounted: Vec<(Tid, u32, u32)>,
}

/// Returns the chunk header in slot `slot` of store page `page`, if a chunk is there.
fn chunk_at(page: &[u8], slot: usize) -> Option<Chunk> {
    match page::get(page, slot) {
        Slot::Normal { at, .. } => Some(chunk(&page[at])),
        _ => None,
    }
}

fn update_chunk(page: &mut [u8], slot: usize, f: impl FnOnce(&mut Chunk)) {
    let Slot::Normal { at, .. } = page::get(page, slot) else {
        panic!("slot {slot} holds no chunk");
    };
    let mut c = chunk(&page[at.clone()]);
    f(&mut c);
    set_chunk(&mut page[at], c);
}

fn tid(pid: Pid, slot: usize) -> Tid {
    Tid::new(pid.0, slot as u16).expect("48-bit page id")
}

/// The large-value store of a sum, shared by its constructor heaps.
///
/// Its pages are slotted heap pages owned by the store's id. Each tuple is a chunk of one value:
/// the TID of the next chunk, zero for the last, the writer, the count and role, then bytes.
///
/// Invariants:
///
/// - A value's first chunk is a head whose count is the number of versions pointing at it;
///   every later chunk is a tail named by exactly one chunk's `next`.
/// - Every page holding a head of count 0 is touched in the store's directory.
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

    /// Store `bytes`, written by `xid`, and return the pointer to them, its head of count 0.
    ///
    /// Chunks are written last first, each in an update of its own that also makes the chunk
    /// written before it a tail, so a crash leaves a head of count 0 on a touched page.
    pub fn write<R: Runtime>(
        &self,
        pager: &Pager<R>,
        xid: u64,
        bytes: &[u8],
    ) -> Result<Pointer, PageError> {
        let room = slotted::max_version(pager.page_size()) - BYTES;
        let mut next = None;
        for piece in bytes.chunks(room).rev() {
            let mut tuple = vec![0; BYTES];
            set_chunk(
                &mut tuple,
                Chunk {
                    next,
                    xmin: xid,
                    head: true,
                    count: 0,
                },
            );
            tuple.extend_from_slice(piece);
            next = Some(self.insert(pager, &tuple, next)?);
        }
        Ok(Pointer {
            len: bytes.len() as u64,
            first: next.expect("an out-of-line value is not empty"),
        })
    }

    /// Insert `tuple` on some page, making the chunk at `tail` a tail in the same update.
    fn insert<R: Runtime>(
        &self,
        pager: &Pager<R>,
        tuple: &[u8],
        tail: Option<Tid>,
    ) -> Result<Tid, PageError> {
        let size = pager.page_size();
        let need = need_category(Arrangement::Slotted.need(tuple.len()), size);
        loop {
            let pid = match self
                .space
                .find(&Stage::new(), pager, need, Arrangement::Slotted, 1)?
            {
                Some(pid) => pid,
                None => self.space.append(pager, Arrangement::Slotted, 1)?,
            };
            let mut stage = Stage::new();
            let mut pids = vec![pid];
            pids.extend(tail.map(|t| Pid(t.page())));
            pids.sort();
            pids.dedup();
            for &p in &pids {
                stage.page(pager, p)?;
            }
            let page = stage.page(pager, pid)?;
            let Some(slot) = page::insert(page, tuple, Flags::NONE) else {
                drop(stage);
                let mut fix = Stage::new();
                let page = fix.page(pager, pid)?;
                let (position, cat) = (page::position(page), category(page::free(page), size));
                self.space.set_category(&mut fix, pager, position, cat)?;
                fix.commit(pager)?;
                continue;
            };
            let position = page::position(page);
            let cat = category(page::free(page), size);
            if let Some(t) = tail {
                let page = stage.page(pager, Pid(t.page()))?;
                update_chunk(page, usize::from(t.slot()), |c| c.head = false);
            }
            self.space.set_category(&mut stage, pager, position, cat)?;
            self.space.touch(&mut stage, pager, position, true)?;
            stage.commit(pager)?;
            return Ok(tid(pid, slot));
        }
    }

    /// Returns the pages a sweep reads: those touched since the sweep that last left them, in
    /// position order.
    pub fn touched<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pid>, PageError> {
        Ok(self
            .space
            .touched(pager)?
            .into_iter()
            .map(|(_, p)| p)
            .collect())
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
            let page = pager.read(Pid(tid.page()))?;
            let Slot::Normal { at: range, .. } = page::get(&page, usize::from(tid.slot())) else {
                panic!("chunk {tid:?} of a large value is missing");
            };
            let tuple = &page[range];
            at = chunk(tuple).next;
            out.extend_from_slice(&tuple[BYTES..]);
        }
        assert_eq!(
            pointer.len,
            out.len() as u64,
            "large value has the wrong length"
        );
        Ok(out)
    }

    /// Returns the TID and header of every chunk of the value `pointer` names, first to last.
    pub fn chunks<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pointer: Pointer,
    ) -> Result<Vec<(Tid, Chunk)>, PageError> {
        let mut out = Vec::new();
        let mut at = Some(pointer.first);
        while let Some(t) = at {
            let page = pager.read(Pid(t.page()))?;
            let c = chunk_at(&page, usize::from(t.slot()))
                .unwrap_or_else(|| panic!("chunk {t:?} of a large value is missing"));
            out.push((t, c));
            at = c.next;
        }
        Ok(out)
    }

    /// Count one more version pointing at `pointer`, in `stage`, latching its head's page.
    pub(super) fn retain<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        pointer: Pointer,
    ) -> Result<(), PageError> {
        let page = stage.page(pager, Pid(pointer.first.page()))?;
        update_chunk(page, usize::from(pointer.first.slot()), |c| {
            assert!(c.head, "{:?} is not the head of a value", pointer.first);
            c.count += 1;
        });
        Ok(())
    }

    /// Count one version fewer pointing at `pointer`, in an update of its own. A head left at
    /// count 0 is freed by a later [`LargeStore::sweep`].
    pub fn release<R: Runtime>(&self, pager: &Pager<R>, pointer: Pointer) -> Result<(), PageError> {
        let pid = Pid(pointer.first.page());
        let mut stage = Stage::new();
        let page = stage.page(pager, pid)?;
        let mut left = 0;
        update_chunk(page, usize::from(pointer.first.slot()), |c| {
            assert!(
                c.head && c.count > 0,
                "{:?} is not a head pointed at",
                pointer.first
            );
            c.count -= 1;
            left = c.count;
        });
        if left == 0 {
            let position = page::position(page);
            self.space.touch(&mut stage, pager, position, true)?;
        }
        stage.commit(pager)
    }

    /// Free every value whose head on a touched page has count 0 and whose writer `resolved`
    /// accepts, and unmark each touched page left with no head of count 0. Returns the number of
    /// chunks freed.
    ///
    /// Each step frees one chunk and makes the next one a head of count 0, in one update, so a
    /// crash leaves the rest of the value for the next sweep.
    ///
    /// Precondition: no other sweep of this store runs.
    pub fn sweep<R: Runtime>(
        &self,
        pager: &Pager<R>,
        mut resolved: impl FnMut(u64) -> Result<bool, PageError>,
    ) -> Result<usize, PageError> {
        let mut freed = 0;
        for (position, pid) in self.space.touched(pager)? {
            let page = pager.read(pid)?.to_vec();
            let mut pending = false;
            for slot in 0..page::slot_count(&page) {
                let Some(c) = chunk_at(&page, slot) else {
                    continue;
                };
                if !c.head || c.count > 0 {
                    continue;
                }
                if !resolved(c.xmin)? {
                    pending = true;
                    continue;
                }
                freed += self.free_value(pager, tid(pid, slot), c.xmin)?;
            }
            if !pending {
                self.settle(pager, pid, position)?;
            }
        }
        Ok(freed)
    }

    /// Free the value whose head is at `at`, written by `xmin`, one chunk at a time. Returns the
    /// number of chunks freed.
    fn free_value<R: Runtime>(
        &self,
        pager: &Pager<R>,
        mut at: Tid,
        xmin: u64,
    ) -> Result<usize, PageError> {
        let mut freed = 0;
        loop {
            let here = Pid(at.page());
            let next = {
                let page = pager.read(here)?;
                match chunk_at(&page, usize::from(at.slot())) {
                    Some(c) if c.head && c.count == 0 && c.xmin == xmin => c.next,
                    _ => return Ok(freed),
                }
            };
            let mut stage = Stage::new();
            let mut pids = vec![here];
            pids.extend(next.map(|n| Pid(n.page())));
            pids.sort();
            pids.dedup();
            for &p in &pids {
                stage.page(pager, p)?;
            }
            let page = stage.page(pager, here)?;
            if chunk_at(page, usize::from(at.slot())).is_none_or(|c| !c.head || c.count > 0) {
                return Ok(freed);
            }
            page::release(page, usize::from(at.slot()), &Slot::Unused);
            page::compact(page);
            let position = page::position(page);
            let empty = page::is_empty(page);
            let cat = category(page::free(page), pager.page_size());
            if let Some(n) = next {
                let page = stage.page(pager, Pid(n.page()))?;
                update_chunk(page, usize::from(n.slot()), |c| {
                    c.head = true;
                    c.count = 0;
                });
                let p = page::position(page);
                self.space.touch(&mut stage, pager, p, true)?;
            }
            let mut gone = pager.reserve();
            if empty {
                self.space.vacate(&mut stage, pager, position)?;
                gone.free(here);
            } else {
                self.space.set_category(&mut stage, pager, position, cat)?;
            }
            stage.commit_with(pager, gone)?;
            freed += 1;
            match next {
                Some(n) => at = n,
                None => return Ok(freed),
            }
        }
    }

    /// Unmark touched page `pid` at `position` if it holds no head of count 0.
    fn settle<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        position: u32,
    ) -> Result<(), PageError> {
        let mut stage = Stage::new();
        let page = stage.page(pager, pid)?;
        if !page::is_heap(page)
            || storage::owner(page) != self.id()
            || page::position(page) != position
        {
            return Ok(());
        }
        let waiting = (0..page::slot_count(page))
            .filter_map(|s| chunk_at(page, s))
            .any(|c| c.head && c.count == 0);
        if !waiting {
            self.space.touch(&mut stage, pager, position, false)?;
            stage.commit(pager)?;
        }
        Ok(())
    }

    /// Set the count of the head at `head` to `count`, marking its page touched when the count
    /// is 0.
    ///
    /// Panics if no head is at `head`.
    pub fn recount<R: Runtime>(
        &self,
        pager: &Pager<R>,
        head: Tid,
        count: u32,
    ) -> Result<(), PageError> {
        let mut stage = Stage::new();
        let page = stage.page(pager, Pid(head.page()))?;
        update_chunk(page, usize::from(head.slot()), |c| {
            assert!(c.head, "{head:?} is not the head of a value");
            c.count = count;
        });
        if count == 0 {
            let position = page::position(page);
            self.space.touch(&mut stage, pager, position, true)?;
        }
        stage.commit(pager)
    }

    /// Returns what is wrong with the store's chunks, `versions` being the number of versions
    /// that point at each head: a head counting fewer versions than point at it, a head of
    /// count 0 on a page not touched, a tail named by no chunk or by several, and a `next` that
    /// names no tail. A head counting more versions than point at it is returned apart, with
    /// its count and the number pointing at it.
    pub fn check_chunks<R: Runtime>(
        &self,
        pager: &Pager<R>,
        versions: &BTreeMap<Tid, u32>,
    ) -> Result<ChunkCheck, PageError> {
        let touched: BTreeSet<Pid> = self
            .space
            .touched(pager)?
            .into_iter()
            .map(|(_, p)| p)
            .collect();
        let mut chunks = BTreeMap::new();
        for pid in self.space.pages(pager)? {
            let page = pager.read(pid)?;
            if !page::is_heap(&page) || !page::check(&page).is_empty() {
                continue;
            }
            for slot in 0..page::slot_count(&page) {
                if let Some(c) = chunk_at(&page, slot) {
                    chunks.insert(tid(pid, slot), c);
                }
            }
        }
        let mut named: BTreeMap<Tid, u32> = BTreeMap::new();
        let mut found = ChunkCheck::default();
        for (&t, c) in &chunks {
            if let Some(n) = c.next {
                *named.entry(n).or_default() += 1;
                if chunks.get(&n).is_none_or(|nc| nc.head) {
                    found
                        .problems
                        .push((Pid(t.page()), format!("chunk {} names no tail", t.slot())));
                }
            }
        }
        for (&t, c) in &chunks {
            let pid = Pid(t.page());
            let refs = versions.get(&t).copied().unwrap_or(0);
            if c.head {
                if c.count < refs {
                    found.problems.push((
                        pid,
                        format!(
                            "head {} counts {} versions, {refs} point at it",
                            t.slot(),
                            c.count
                        ),
                    ));
                } else if c.count > refs {
                    found.overcounted.push((t, c.count, refs));
                }
                if c.count == 0 && !touched.contains(&pid) {
                    found.problems.push((
                        pid,
                        format!("head {} of no version is not to be swept", t.slot()),
                    ));
                }
            } else if named.get(&t).copied().unwrap_or(0) != 1 {
                found.problems.push((
                    pid,
                    format!(
                        "tail {} is named by {} chunks",
                        t.slot(),
                        named.get(&t).copied().unwrap_or(0)
                    ),
                ));
            }
        }
        for t in versions.keys() {
            if chunks.get(t).is_none_or(|c| !c.head) {
                found.problems.push((
                    Pid(t.page()),
                    format!("slot {} is pointed at and holds no head", t.slot()),
                ));
            }
        }
        Ok(found)
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
