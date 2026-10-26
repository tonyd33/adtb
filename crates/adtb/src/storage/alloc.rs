use crate::runtime::Runtime;

use std::collections::BTreeSet;

use super::page::{self, FIRST_EDITABLE, HEADER_LEN, PageKind};
use super::pager::{PageError, PageExclusive, Pager};
use super::record::Edit;
use super::{Lsn, Pid};

/// Unchanged runs shorter than this between two changed ones are logged as changed.
const MERGE_GAP: usize = 16;

/// Returns the edits that turn `old` into `new`.
pub(crate) fn diff(old: &[u8], new: &[u8]) -> Vec<Edit> {
    let mut edits: Vec<Edit> = Vec::new();
    let mut i = 0;
    while i < new.len() {
        if old[i] == new[i] {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i + 1;
        let mut j = end;
        while j < new.len() && j - end < MERGE_GAP {
            if old[j] != new[j] {
                end = j + 1;
            }
            j += 1;
        }
        edits.push(Edit {
            offset: start as u32,
            bytes: new[start..end].to_vec(),
        });
        i = end;
    }
    edits
}

impl<R: Runtime> Pager<R> {
    /// Returns the number of pages one allocation bitmap page covers.
    fn bitmap_span(&self) -> u64 {
        ((self.page_size - HEADER_LEN) * 8) as u64
    }

    /// Returns the bitmap page that covers group `g` of [`Self::bitmap_span`] pages.
    fn bitmap_page(&self, g: u64) -> Pid {
        Pid((g * self.bitmap_span()).max(Pid::FIRST_DATA.0))
    }

    /// Allocate a free page, set its header to `format`, `kind` and `owner`, and zero its contents.
    ///
    /// Fails, allocating nothing, if an allocation bitmap page it reads is damaged. A free page
    /// found damaged is refused from then on and the search goes on.
    ///
    /// Postconditions:
    ///
    /// - The page was free, and is now marked allocated in the same batch that formats it.
    pub fn allocate(&self, format: u16, kind: PageKind, owner: u32) -> Result<Pid, PageError> {
        let mut new = self.reserve();
        let pid = new.page(format, kind, owner)?;
        self.update_with(new, &[pid], |_| {})?;
        Ok(pid)
    }

    /// Returns an empty reservation of free pages, for one [`Pager::update_with`] to allocate.
    pub fn reserve(&self) -> Reservation<'_, R> {
        Reservation {
            pager: self,
            pages: Vec::new(),
            freed: Vec::new(),
        }
    }

    /// Returns a free page no allocator or reservation holds, and holds it.
    fn reserve_page(&self) -> Result<(Pid, Lsn), PageError> {
        let span = self.bitmap_span();
        let mut state = self.alloc.lock(&self.rt);
        let mut g = state.group;
        loop {
            let map = self.bitmap_page(g);
            let first = g * span;
            let bits = self.read(map)?[HEADER_LEN..].to_vec();
            let taken = |pid: u64| {
                let i = (pid - first) as usize;
                pid < Pid::FIRST_DATA.0
                    || pid == map.0
                    || bits[i / 8] >> (i % 8) & 1 == 1
                    || state.reserved.contains(&Pid(pid))
                    || self.meta().damaged.contains_key(&Pid(pid))
            };
            let Some(pid) = (first..first + span).find(|&p| !taken(p)) else {
                g += 1;
                continue;
            };
            state.group = g;
            let pid = Pid(pid);
            match self.exclusive(pid) {
                Ok(g) => {
                    state.reserved.insert(pid);
                    return Ok((pid, page::lsn(&g)));
                }
                Err(e) if e.pid == pid => continue,
                Err(e) => return Err(e),
            }
        }
    }

    /// Change `pids` through `f` as [`Pager::update`] does, allocating every page of `new` in the
    /// same batch: each is marked in the allocation bitmap and formatted, header set and contents
    /// zero, before `f` sees it.
    ///
    /// Returns the LSN of the last record. Fails, allocating and changing nothing, if a page is
    /// damaged.
    ///
    /// Panics if a page of `new` is missing from `pids`.
    pub fn update_with<F>(
        &self,
        new: Reservation<'_, R>,
        pids: &[Pid],
        f: F,
    ) -> Result<Lsn, PageError>
    where
        F: FnOnce(&mut [&mut [u8]]),
    {
        for r in &new.pages {
            assert!(
                pids.contains(&r.pid),
                "reserved {:?} is not among the pages changed",
                r.pid
            );
        }
        let mut f = Some(f);
        self.with_latched(pids, |pager, guards| {
            pager.allocating(&new, guards, f.take().unwrap())
        })?
    }

    /// Change the pages under `guards` through `f` as [`Pager::update_all`] does, allocating every
    /// page of `new` in the same batch as [`Pager::update_with`] does.
    ///
    /// Panics if a page of `new` has no guard among `guards`.
    pub fn update_all_with<'p, F>(
        &'p self,
        new: Reservation<'_, R>,
        guards: &mut [&mut PageExclusive<'p, R>],
        f: F,
    ) -> Result<Lsn, PageError>
    where
        F: FnOnce(&mut [&mut [u8]]),
    {
        for r in &new.pages {
            assert!(
                guards.iter().any(|g| g.pid() == r.pid),
                "reserved {:?} is not among the pages changed",
                r.pid
            );
        }
        let mut inner: Vec<&mut adtb_sys::pages::Exclusive<'p>> =
            guards.iter_mut().map(|g| g.inner()).collect();
        self.allocating(&new, &mut inner, f)
    }

    /// Mark and format the pages of `new`, apply `f` to the pages under `guards`, and log all of
    /// it in one batch.
    fn allocating<'p, F>(
        &'p self,
        new: &Reservation<'_, R>,
        guards: &mut [&mut adtb_sys::pages::Exclusive<'p>],
        f: F,
    ) -> Result<Lsn, PageError>
    where
        F: FnOnce(&mut [&mut [u8]]),
    {
        let span = self.bitmap_span();
        let mut state = self.alloc.lock(&self.rt);
        let mut groups: Vec<u64> = new
            .pages
            .iter()
            .map(|r| r.pid.0 / span)
            .chain(new.freed.iter().map(|p| p.0 / span))
            .collect();
        groups.sort_unstable();
        groups.dedup();
        let mut maps = Vec::new();
        for &g in &groups {
            maps.push(self.exclusive(self.bitmap_page(g))?);
        }
        let mut copies: Vec<Vec<u8>> = guards.iter().map(|g| g.to_vec()).collect();
        for (copy, g) in copies.iter_mut().zip(guards.iter()) {
            if let Some(r) = new.pages.iter().find(|r| r.pid.0 == g.pid()) {
                copy[FIRST_EDITABLE..HEADER_LEN]
                    .copy_from_slice(&page::header_edit(r.format, r.kind, r.owner, 0));
                copy[HEADER_LEN..].fill(0);
            }
        }
        let mut views: Vec<&mut [u8]> = copies.iter_mut().map(Vec::as_mut_slice).collect();
        f(&mut views);
        let mut edits: Vec<Vec<Edit>> = guards
            .iter()
            .zip(&copies)
            .map(|(g, new)| diff(g, new))
            .collect();
        for (map, &g) in maps.iter().zip(&groups) {
            let mut bits = map.to_vec();
            bits[FIRST_EDITABLE..HEADER_LEN].copy_from_slice(&page::header_edit(
                1,
                PageKind::AllocationBitmap,
                0,
                0,
            ));
            let first = g * span;
            let owned = new.pages.iter().map(|r| r.pid.0).filter(|p| p / span == g);
            for p in std::iter::once(self.bitmap_page(g).0).chain(owned) {
                let i = (p - first) as usize;
                bits[HEADER_LEN + i / 8] |= 1 << (i % 8);
            }
            for p in new.freed.iter().filter(|p| p.0 / span == g) {
                let i = (p.0 - first) as usize;
                let byte = &mut bits[HEADER_LEN + i / 8];
                assert!(*byte >> (i % 8) & 1 == 1, "{p:?} is not allocated");
                *byte &= !(1 << (i % 8));
                state.group = state.group.min(g);
            }
            edits.push(diff(map, &bits));
        }
        let mut all: Vec<&mut adtb_sys::pages::Exclusive<'p>> = Vec::new();
        for g in guards.iter_mut() {
            all.push(g);
        }
        for m in &mut maps {
            all.push(m.inner());
        }
        Ok(self.apply(&mut all, edits, None))
    }

    /// Mark `pid` free. Its contents stay until it is allocated again.
    ///
    /// Preconditions: nothing refers to `pid`, and it is allocated.
    pub fn free(&self, pid: Pid) -> Result<Lsn, PageError> {
        let mut new = self.reserve();
        new.free(pid);
        self.update_with(new, &[], |_| {})
    }

    /// Returns every page the allocation bitmap marks allocated, in order, leaving out the pages
    /// the pager owns itself: the file header copies, the bitmap's pages and the quarantine
    /// marks' page.
    ///
    /// Fails if an allocation bitmap page is damaged.
    pub fn allocated(&self) -> Result<Vec<Pid>, PageError> {
        let span = self.bitmap_span();
        let marks = self.quarantine_page();
        let mut out = Vec::new();
        for g in 0.. {
            let map = self.bitmap_page(g);
            let page = self.read(map)?.to_vec();
            if page::kind(&page) != PageKind::AllocationBitmap as u16 {
                break;
            }
            let bits = &page[HEADER_LEN..];
            out.extend(
                (0..span)
                    .filter(|&i| bits[(i / 8) as usize] >> (i % 8) & 1 == 1)
                    .map(|i| Pid(g * span + i))
                    .filter(|&p| p != map && p != marks && p >= Pid::FIRST_DATA),
            );
        }
        Ok(out)
    }
}

/// Free pages held for one [`Pager::update_with`] or [`Pager::update_all_with`], which marks
/// them allocated, and allocated pages it frees. Dropping the reservation unused gives the free
/// pages back and frees nothing; nothing was logged.
pub struct Reservation<'a, R: Runtime> {
    pager: &'a Pager<R>,
    pages: Vec<Reserved>,
    freed: Vec<Pid>,
}

#[derive(Debug)]
struct Reserved {
    pid: Pid,
    lsn: Lsn,
    format: u16,
    kind: PageKind,
    owner: u32,
}

impl<R: Runtime> std::fmt::Debug for Reservation<'_, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reservation")
            .field("pages", &self.pages)
            .field("freed", &self.freed)
            .finish()
    }
}

impl<R: Runtime> Reservation<'_, R> {
    /// Hold a free page, to be formatted with `format`, `kind` and `owner` when allocated.
    ///
    /// Fails if an allocation bitmap page is damaged.
    pub fn page(&mut self, format: u16, kind: PageKind, owner: u32) -> Result<Pid, PageError> {
        let (pid, lsn) = self.pager.reserve_page()?;
        self.pages.push(Reserved {
            pid,
            lsn,
            format,
            kind,
            owner,
        });
        Ok(pid)
    }

    /// Free `pid` in the update this reservation is given to. Its contents stay until it is
    /// allocated again.
    ///
    /// Preconditions: `pid` is allocated, and nothing refers to it once the update is applied.
    ///
    /// Panics if `pid` is a file header or allocation bitmap page.
    pub fn free(&mut self, pid: Pid) {
        let g = pid.0 / self.pager.bitmap_span();
        assert!(
            pid != self.pager.bitmap_page(g) && pid >= Pid::FIRST_DATA,
            "{pid:?} is not an allocatable page"
        );
        self.freed.push(pid);
    }

    /// Returns the page `pid` as allocation will format it, with the LSN it has now.
    ///
    /// Panics if `pid` is not reserved here.
    pub fn image(&self, pid: Pid) -> Vec<u8> {
        let r = self
            .pages
            .iter()
            .find(|r| r.pid == pid)
            .unwrap_or_else(|| panic!("{pid:?} is not reserved here"));
        let mut out = vec![0; self.pager.page_size];
        page::set_lsn(&mut out, r.lsn);
        out[FIRST_EDITABLE..HEADER_LEN]
            .copy_from_slice(&page::header_edit(r.format, r.kind, r.owner, 0));
        out
    }
}

impl<R: Runtime> Drop for Reservation<'_, R> {
    fn drop(&mut self) {
        let mut state = self.pager.alloc.lock(&self.pager.rt);
        for r in &self.pages {
            state.reserved.remove(&r.pid);
        }
    }
}

/// What allocators share: the lowest bitmap group that may have a free page, and the pages
/// reservations hold.
#[derive(Debug, Default)]
pub(crate) struct AllocState {
    pub(crate) group: u64,
    pub(crate) reserved: BTreeSet<Pid>,
}
