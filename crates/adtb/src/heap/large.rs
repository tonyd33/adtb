use std::collections::{BTreeMap, BTreeSet};

use crate::key::Tid;
use crate::runtime::Runtime;
use crate::storage::{self, Damage, LATCHES_PER_THREAD, PageError, Pager, Pid, Stopped, Walk};

use super::Error;
use super::dir::{Above, Space, category, max_fanout, need_category};
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

/// The store pages one update of [`LargeStore::retain_apart`] latches, beside the two heap pages
/// an update or a retag holds.
const COUNTED_PAGES: usize = LATCHES_PER_THREAD - 2;

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

/// Returns the chunk header in slot `slot` of `page`, if it is a page of store `owner` with a
/// chunk there.
fn chunk_at(page: &[u8], owner: u32, slot: usize) -> Option<Chunk> {
    tuple_at(page, owner, slot).map(chunk)
}

/// Returns the chunk tuple in slot `slot` of `page`, if it is a page of store `owner` with a
/// chunk there.
fn tuple_at(page: &[u8], owner: u32, slot: usize) -> Option<&[u8]> {
    if !page::is_heap(page) || storage::owner(page) != owner {
        return None;
    }
    match page::get(page, slot) {
        Slot::Normal { at, .. } if at.len() >= BYTES => Some(&page[at]),
        _ => None,
    }
}

/// Change the chunk at `tid` on `page`, a page of store `owner`, by `f`, unless `f` refuses it.
/// Returns false when no chunk is there or `f` refuses it, leaving the page unchanged.
fn update_chunk(page: &mut [u8], owner: u32, tid: Tid, f: impl FnOnce(&mut Chunk) -> bool) -> bool {
    let Some(mut c) = chunk_at(page, owner, usize::from(tid.slot())) else {
        return false;
    };
    if !f(&mut c) {
        return false;
    }
    let Slot::Normal { at, .. } = page::get(page, usize::from(tid.slot())) else {
        unreachable!("tuple_at found a chunk");
    };
    set_chunk(&mut page[at], c);
    true
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
    ) -> Result<Pointer, Error> {
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
    ) -> Result<Tid, Error> {
        let size = pager.page_size();
        let need = need_category(Arrangement::Slotted.need(tuple.len()), size);
        loop {
            let room = self.space.room(pager, need, Arrangement::Slotted, 1)?;
            let pid = room.ok_or(Error::NoDirectory)?;
            drop(pager.read_formatted(pid)?);
            let mut stage = Stage::new();
            let mut pids = vec![pid];
            pids.extend(tail.map(|t| Pid(t.page())));
            pids.sort();
            pids.dedup();
            for &p in &pids {
                stage.page(pager, p)?;
            }
            let page = stage.page(pager, pid)?;
            // A page found by an unlatched search may have been freed since.
            if !page::is_heap(page)
                || storage::owner(page) != self.id()
                || !self.space.names(pager, page::position(page), pid)?
            {
                continue;
            }
            let Some(slot) = page::insert(page, tuple, Flags::NONE) else {
                drop(stage);
                let mut fix = Stage::new();
                let page = fix.page(pager, pid)?;
                let (position, cat) = (page::position(page), category(page::free(page), size));
                let above = self
                    .space
                    .set_category(&mut fix, pager, pid, position, cat)?;
                fix.commit(pager)?;
                above.settle(pager)?;
                continue;
            };
            let position = page::position(page);
            let cat = category(page::free(page), size);
            if let Some(t) = tail {
                let page = stage.page(pager, Pid(t.page()))?;
                let written = update_chunk(page, self.id(), t, |c| {
                    c.head = false;
                    true
                });
                assert!(written, "the chunk this write wrote at {t:?} is there");
            }
            let above =
                self.space
                    .set_entry(&mut stage, pager, pid, position, Some(cat), Some(true))?;
            stage.commit(pager)?;
            above.settle(pager)?;
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
            let pid = Pid(tid.page());
            let page = pager.read_formatted(pid)?;
            let Some(tuple) = tuple_at(&page, self.id(), usize::from(tid.slot())) else {
                drop(page);
                return Err(pager.damage(pid, Damage::Checksum));
            };
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
            let pid = Pid(t.page());
            let page = pager.read_formatted(pid)?;
            let Some(c) = chunk_at(&page, self.id(), usize::from(t.slot())) else {
                drop(page);
                return Err(pager.damage(pid, Damage::Checksum));
            };
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
        self.update(stage, pager, pointer.first, |c| {
            c.count += 1;
            c.head
        })
    }

    /// Change the chunk at `tid` by `f` in `stage`, latching its page. Refuses the page when no
    /// chunk is there or `f` refuses it.
    fn update<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        tid: Tid,
        f: impl FnOnce(&mut Chunk) -> bool,
    ) -> Result<(), PageError> {
        let pid = Pid(tid.page());
        if update_chunk(stage.page(pager, pid)?, self.id(), tid, f) {
            return Ok(());
        }
        stage.unstage(pid);
        Err(pager.damage(pid, Damage::Checksum))
    }

    /// Count one version fewer pointing at `pointer`, in an update of its own. A head left at
    /// count 0 is freed by a later [`LargeStore::sweep`].
    pub fn release<R: Runtime>(&self, pager: &Pager<R>, pointer: Pointer) -> Result<(), PageError> {
        let pid = Pid(pointer.first.page());
        let mut stage = Stage::new();
        let mut left = 0;
        self.update(&mut stage, pager, pointer.first, |c| {
            let pointed = c.head && c.count > 0;
            c.count = c.count.saturating_sub(1);
            left = c.count;
            pointed
        })?;
        let page = stage.page(pager, pid)?;
        let mut above = Above::default();
        if left == 0 {
            let position = page::position(page);
            above = self.space.touch(&mut stage, pager, pid, position, true)?;
        }
        stage.commit(pager)?;
        above.settle(pager)
    }

    /// Settle the store's directory, then free every value whose head on a touched page has count
    /// 0 and whose writer `resolved` accepts, and unmark each touched page left with no head of
    /// count 0, ending early once `between` returns false after a chunk or a page. Returns the
    /// number of chunks freed, and whether it stopped early.
    ///
    /// Each step frees one chunk and makes the next one a head of count 0, in one update, so a
    /// crash leaves the rest of the value for the next sweep.
    ///
    /// Precondition: no other sweep of this store runs.
    pub fn sweep<R: Runtime>(
        &self,
        pager: &Pager<R>,
        mut resolved: impl FnMut(u64) -> Result<bool, PageError>,
        between: &mut dyn FnMut() -> bool,
    ) -> Result<Walk, PageError> {
        self.space.settle(pager)?;
        let mut freed = 0;
        for (position, pid) in self.space.touched(pager)? {
            let page = pager.read_formatted(pid)?.to_vec();
            let mut pending = false;
            for slot in 0..page::slot_count(&page) {
                let Some(c) = chunk_at(&page, self.id(), slot) else {
                    continue;
                };
                if !c.head || c.count > 0 {
                    continue;
                }
                if !resolved(c.xmin)? {
                    pending = true;
                    continue;
                }
                match self.free_value(pager, tid(pid, slot), c.xmin, between)? {
                    Walk::Done(n) => freed += n,
                    Walk::Stopped(n) => return Ok(Walk::Stopped(freed + n)),
                }
            }
            if !pending {
                self.settle(pager, pid, position)?;
            }
            if !between() {
                return Ok(Walk::Stopped(freed));
            }
        }
        Ok(Walk::Done(freed))
    }

    /// Free the value whose head is at `at`, written by `xmin`, one chunk at a time, calling
    /// `between` after each chunk and ending once it returns false. Returns the number of chunks
    /// freed, and whether it stopped early.
    fn free_value<R: Runtime>(
        &self,
        pager: &Pager<R>,
        mut at: Tid,
        xmin: u64,
        between: &mut dyn FnMut() -> bool,
    ) -> Result<Walk, PageError> {
        let mut freed = 0;
        loop {
            let here = Pid(at.page());
            let next = {
                let page = pager.read_formatted(here)?;
                match chunk_at(&page, self.id(), usize::from(at.slot())) {
                    Some(c) if c.head && c.count == 0 && c.xmin == xmin => c.next,
                    _ => return Ok(Walk::Done(freed)),
                }
            };
            if let Some(n) = next {
                self.touch_page(pager, Pid(n.page()))?;
            }
            let mut stage = Stage::new();
            let mut pids = vec![here];
            pids.extend(next.map(|n| Pid(n.page())));
            pids.sort();
            pids.dedup();
            for &p in &pids {
                stage.page(pager, p)?;
            }
            let page = stage.page(pager, here)?;
            if chunk_at(page, self.id(), usize::from(at.slot()))
                .is_none_or(|c| !c.head || c.count > 0)
            {
                return Ok(Walk::Done(freed));
            }
            page::release(page, usize::from(at.slot()), &Slot::Unused);
            page::compact(page);
            let position = page::position(page);
            let empty = page::is_empty(page);
            let cat = category(page::free(page), pager.page_size());
            if let Some(n) = next {
                self.update(&mut stage, pager, n, |c| {
                    c.head = true;
                    c.count = 0;
                    true
                })?;
            }
            let mut gone = pager.reserve();
            let above = if empty {
                gone.free(here);
                self.space.vacate(&mut stage, pager, here, position)?
            } else {
                self.space
                    .set_category(&mut stage, pager, here, position, cat)?
            };
            stage.commit_with(pager, gone)?;
            above.settle(pager)?;
            freed += 1;
            if !between() {
                return Ok(Walk::Stopped(freed));
            }
            let Some(n) = next else {
                return Ok(Walk::Done(freed));
            };
            at = n;
        }
    }

    /// Mark store page `pid` touched, in an update of its own.
    fn touch_page<R: Runtime>(&self, pager: &Pager<R>, pid: Pid) -> Result<(), PageError> {
        let mut stage = Stage::new();
        let position = page::position(stage.page(pager, pid)?);
        let above = self.space.touch(&mut stage, pager, pid, position, true)?;
        stage.commit(pager)?;
        above.settle(pager)
    }

    /// Count one more version pointing at each of `pointers`, in updates of their own, each
    /// latching at most [`COUNTED_PAGES`] store pages in page id order.
    ///
    /// Precondition: the versions pointing at `pointers` are written by a later update.
    pub(super) fn retain_apart<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pointers: &[Pointer],
    ) -> Result<(), PageError> {
        let mut heads = pointers.to_vec();
        heads.sort_by_key(|p| (p.first.page(), p.first.slot()));
        let by_page: Vec<&[Pointer]> = heads
            .chunk_by(|a, b| a.first.page() == b.first.page())
            .collect();
        for pages in by_page.chunks(COUNTED_PAGES) {
            let mut stage = Stage::new();
            for &p in pages.iter().copied().flatten() {
                self.retain(&mut stage, pager, p)?;
            }
            stage.commit(pager)?;
        }
        Ok(())
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
            .filter_map(|s| chunk_at(page, self.id(), s))
            .any(|c| c.head && c.count == 0);
        if !waiting {
            let above = self.space.touch(&mut stage, pager, pid, position, false)?;
            stage.commit(pager)?;
            above.settle(pager)?;
        }
        Ok(())
    }

    /// Set the count of the head at `head` to `count`, marking its page touched when the count
    /// is 0. Refuses the page when no head is at `head`.
    pub fn recount<R: Runtime>(
        &self,
        pager: &Pager<R>,
        head: Tid,
        count: u32,
    ) -> Result<(), PageError> {
        let mut stage = Stage::new();
        self.update(&mut stage, pager, head, |c| {
            c.count = count;
            c.head
        })?;
        let page = stage.page(pager, Pid(head.page()))?;
        let mut above = Above::default();
        if count == 0 {
            let position = page::position(page);
            above = self
                .space
                .touch(&mut stage, pager, Pid(head.page()), position, true)?;
        }
        stage.commit(pager)?;
        above.settle(pager)
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
        let touched: BTreeSet<Pid> = self.space.check(pager)?.touched.into_iter().collect();
        let mut chunks = BTreeMap::new();
        for pid in self.space.pages(pager)? {
            let page = pager.read_formatted(pid)?;
            if !page::is_heap(&page) || !page::check(&page).is_empty() {
                continue;
            }
            for slot in 0..page::slot_count(&page) {
                if let Some(c) = chunk_at(&page, self.id(), slot) {
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
        self.space.owned(pager)
    }

    /// Returns every page the store owns as [`LargeStore::owned_pages`] does, for a walk mapping
    /// damage to its holder: a directory page it cannot follow stops the walk as a shape,
    /// recording no damage.
    pub fn held_pages<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pid>, Stopped> {
        self.space.held(pager)
    }

    /// Returns what is wrong with the store's directory and pages, directory entries promising
    /// too little included.
    pub fn verify<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<String>, PageError> {
        let mut problems = self.check(pager)?;
        problems.extend(self.unsettled(pager)?);
        Ok(problems
            .into_iter()
            .map(|(pid, what)| format!("{pid:?} {what}"))
            .collect())
    }

    /// Returns each directory page with an entry that promises less than its child holds, with
    /// what it promises too little. A crash may leave them; [`LargeStore::settle_directory`]
    /// mends them.
    pub fn unsettled<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<(Pid, String)>, PageError> {
        Ok(self.space.check(pager)?.unsettled)
    }

    /// Set every inner directory entry to what its child holds, each in an update of its own.
    pub fn settle_directory<R: Runtime>(&self, pager: &Pager<R>) -> Result<(), PageError> {
        self.space.settle(pager)
    }

    /// Returns each page of the store found wrong, with what is wrong with it, leaving out
    /// [`LargeStore::unsettled`] directory entries.
    pub fn check<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<(Pid, String)>, PageError> {
        let check = self.space.check(pager)?;
        let mut problems = check.problems;
        for (position, leaf) in check.leaves.into_iter().enumerate() {
            let Some((pid, cat)) = leaf else {
                continue;
            };
            let page = &pager.read_formatted(pid)?.to_vec();
            let ours = page::has_heap_kind(page) && storage::owner(page) == self.id();
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
