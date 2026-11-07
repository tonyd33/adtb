use crate::runtime::Runtime;

use super::page::{self, FIRST_EDITABLE, HEADER_LEN, PageKind};
use super::pager::{Damage, PageError, PageExclusive, PageShared, Pager};
use super::record::{Edit, Record};
use super::{Lsn, Pid};

/// Unchanged runs shorter than this between two changed ones are logged as changed.
const MERGE_GAP: usize = 16;

/// Returns the edits that turn `old` into `new`.
pub(crate) fn diff(old: &[u8], new: &[u8]) -> Vec<Edit> {
    let mut edits: Vec<Edit> = Vec::new();
    let mut i = 0;
    while i < new.len() {
        i = first_difference(old, new, i);
        if i == new.len() {
            break;
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

/// Returns the first index from `i` where `old` and `new` differ, or `new.len()`.
fn first_difference(old: &[u8], new: &[u8], mut i: usize) -> usize {
    let word = |b: &[u8], at: usize| {
        u64::from_ne_bytes(b[at..at + 8].try_into().expect("the range is 8 bytes long"))
    };
    while i + 8 <= new.len() && word(old, i) == word(new, i) {
        i += 8;
    }
    while i < new.len() && old[i] == new[i] {
        i += 1;
    }
    i
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

    /// Returns whether `pid` is an allocation bitmap page.
    pub(crate) fn is_bitmap_page(&self, pid: Pid) -> bool {
        pid == self.bitmap_page(pid.0 / self.bitmap_span())
    }

    /// Returns whether `page`, read under a latch as the bitmap page of group `g`, can serve as
    /// one: it is a bitmap page, or blank in a group no batch has started.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    fn map_is_sound(&self, page: &[u8], g: u64) -> bool {
        page::kind(page) == PageKind::AllocationBitmap as u16
            || g >= self.meta().groups && page.iter().all(|&b| b == 0)
    }

    /// Returns the bitmap page of group `g` under a shared latch. Refuses it, from then on, when
    /// the group was started and the page holds no bitmap.
    fn read_map(&self, g: u64) -> Result<PageShared<'_>, PageError> {
        let map = self.bitmap_page(g);
        let page = self.read(map)?;
        if self.map_is_sound(&page, g) {
            return Ok(page);
        }
        drop(page);
        Err(self.damage(map, Damage::Checksum))
    }

    /// Count every group the file reaches into as started. Returns the pages the file holds.
    ///
    /// Preconditions: recovery has written every page the log holds.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    pub(crate) fn count_file_groups(&self) -> u64 {
        let pages = self.file_pages();
        let group = pages.saturating_sub(1) / self.bitmap_span();
        let mut m = self.meta();
        m.groups = m.groups.max(group + 1);
        pages
    }

    /// Refuse each page the allocation bitmap marks allocated past the end of the file, the first
    /// bitmap page when the file ends before it, the last bitmap page the file holds when that
    /// holds no bitmap, and each later bitmap page with an expected LSN as a blank copy of it.
    ///
    /// Preconditions: recovery has written every page the log holds and armed the read-in gate,
    /// and the file holds `pages`.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    pub(crate) fn refuse_cut_pages(&self, pages: u64) {
        let span = self.bitmap_span();
        let group = pages.saturating_sub(1) / span;
        self.refuse_cut_group(pages, group);
        let end = self.meta().gate.end();
        let later = (group + 1..).map(|g| self.bitmap_page(g));
        for map in later.take_while(|m| m.0 < end) {
            let expected = self.meta().gate.expected(map).filter(|&l| l > Lsn(0));
            if let Some(expected) = expected {
                let found = Lsn(0);
                self.damage(map, Damage::Stale { expected, found });
            }
        }
    }

    fn refuse_cut_group(&self, pages: u64, group: u64) {
        let span = self.bitmap_span();
        let map = self.bitmap_page(group);
        if map.0 >= pages {
            self.damage(map, Damage::Checksum);
            return;
        }
        let Ok(bits) = self.read_map(group).map(|page| page[HEADER_LEN..].to_vec()) else {
            return;
        };
        let first = group * span;
        for pid in pages..first + span {
            let i = (pid - first) as usize;
            if bits[i / 8] >> (i % 8) & 1 == 1 {
                self.damage(Pid(pid), Damage::Checksum);
            }
        }
    }

    /// Allocate a free page, set its header to `format`, `kind` and `owner`, and zero its contents.
    ///
    /// Fail, allocating nothing, if an allocation bitmap page it reads is damaged.
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

    /// Returns a free page no allocator or reservation holds, held as reserved. A page the device
    /// could not read is skipped.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    fn hold_free_page(&self) -> Result<Pid, PageError> {
        let span = self.bitmap_span();
        let mut state = self.alloc.lock(&self.rt);
        let mut g = state.group;
        loop {
            let map = self.bitmap_page(g);
            let first = g * span;
            let bits = self.read_map(g)?[HEADER_LEN..].to_vec();
            let taken = |pid: u64| {
                let i = (pid - first) as usize;
                pid < Pid::FIRST_DATA.0 || pid == map.0 || bits[i / 8] >> (i % 8) & 1 == 1 || {
                    let m = self.meta();
                    m.reserved.contains(&Pid(pid))
                        || m.damaged.get(&Pid(pid)) == Some(&Damage::Unreadable)
                }
            };
            let Some(pid) = (first..first + span).find(|&p| !taken(p)) else {
                g += 1;
                continue;
            };
            state.group = g;
            self.meta().reserved.insert(Pid(pid));
            return Ok(Pid(pid));
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
        self.update_with_tail(new, pids, f, None)
    }

    /// Change `pids` as [`Pager::update_with`] does, logging `tail` after them in the same batch.
    pub(crate) fn update_with_tail<F>(
        &self,
        new: Reservation<'_, R>,
        pids: &[Pid],
        f: F,
        tail: Option<Record>,
    ) -> Result<Lsn, PageError>
    where
        F: FnOnce(&mut [&mut [u8]]),
    {
        let mut f = Some(f);
        let mut tail = Some(tail);
        let latch = |pid| match new.pages.iter().any(|r| r.pid == pid) {
            true => Ok(new.latch(pid)),
            false => self.exclusive(pid),
        };
        self.with_latched_by(pids, latch, |pager, guards| {
            pager.allocating(&new, guards, f.take().unwrap(), tail.take().unwrap())
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
        self.allocating(&new, &mut inner, f, None)
    }

    /// Mark and format the pages of `new`, apply `f` to the pages under `guards`, and log all of
    /// it in one batch.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    fn allocating<'p, F>(
        &'p self,
        new: &Reservation<'_, R>,
        guards: &mut [&mut adtb_sys::pages::Exclusive<'p>],
        f: F,
        tail: Option<Record>,
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
            let map = self.exclusive(self.bitmap_page(g))?;
            if !self.map_is_sound(&map, g) {
                drop(map);
                return Err(self.damage(self.bitmap_page(g), Damage::Checksum));
            }
            maps.push(map);
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
        {
            let mut m = self.meta();
            m.overwritten.extend(new.pages.iter().map(|r| r.pid));
            for r in &new.pages {
                m.blanked.remove(&r.pid);
            }
        }
        let mut all: Vec<&mut adtb_sys::pages::Exclusive<'p>> = Vec::new();
        for g in guards.iter_mut() {
            all.push(g);
        }
        for m in &mut maps {
            all.push(m.inner());
        }
        let lsn = self.apply_freeing(&mut all, edits, &new.freed, tail);
        if let Some(&g) = groups.last() {
            let mut m = self.meta();
            m.groups = m.groups.max(g + 1);
        }
        Ok(lsn)
    }

    /// Mark `pid` free. Its contents stay until it is allocated again.
    ///
    /// Preconditions: nothing refers to `pid`, and it is allocated.
    pub fn free(&self, pid: Pid) -> Result<Lsn, PageError> {
        let mut new = self.reserve();
        new.free(pid);
        self.update_with(new, &[], |_| {})
    }

    /// Returns whether the allocation bitmap calls `pid` free.
    ///
    /// Fail if its bitmap page is damaged or was lost.
    pub fn is_free(&self, pid: Pid) -> Result<bool, PageError> {
        let span = self.bitmap_span();
        let map = self.bitmap_page(pid.0 / span);
        if pid == map || pid < Pid::FIRST_DATA {
            return Ok(false);
        }
        let page = self.read_map(pid.0 / span)?;
        if page::kind(&page) != PageKind::AllocationBitmap as u16 {
            return Ok(true);
        }
        let i = pid.0 % span;
        let bits = &page[HEADER_LEN..];
        Ok(bits[(i / 8) as usize] >> (i % 8) & 1 == 0)
    }

    /// Returns every page the allocation bitmap marks allocated, in order, leaving out the pages
    /// the pager owns itself: the file header copies, the bitmap's pages, the quarantine marks'
    /// page, the damage list's page and the page LSN map's pages.
    ///
    /// Fail if an allocation bitmap page or a map page is refused.
    pub fn allocated(&self) -> Result<Vec<Pid>, PageError> {
        let span = self.bitmap_span();
        let marks = self.quarantine_page();
        let list = self.damage_list_page();
        let map = self.map_pages()?;
        let mut out = Vec::new();
        for g in 0.. {
            let bitmap = self.bitmap_page(g);
            let page = self.read_map(g)?.to_vec();
            if page::kind(&page) != PageKind::AllocationBitmap as u16 {
                break;
            }
            let bits = &page[HEADER_LEN..];
            out.extend(
                (0..span)
                    .filter(|&i| bits[(i / 8) as usize] >> (i % 8) & 1 == 1)
                    .map(|i| Pid(g * span + i))
                    .filter(|&p| p != bitmap && p != marks && p != list && p >= Pid::FIRST_DATA)
                    .filter(|p| !map.contains(p)),
            );
        }
        Ok(out)
    }

    /// Returns the exclusive latch on `pid`, a free page a reservation holds, without reading the
    /// file: when not resident it is read in with `header` and zero contents, and no longer
    /// refused. The latch's drop undoes both unless an allocation of `pid` committed.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    pub(crate) fn latch_reserved(&self, pid: Pid, header: &[u8]) -> PageExclusive<'_, R> {
        let mut g = self.mem.lock_exclusive(pid.0, &self.rt);
        self.check_bound();
        if g.resident() {
            return PageExclusive::of(self, g);
        }
        {
            let mut m = self.meta();
            let refusal = m.damaged.remove(&pid);
            m.blanked.insert(pid, refusal);
        }
        self.make_room(pid);
        let mut blank = vec![0; self.page_size];
        blank[FIRST_EDITABLE..HEADER_LEN].copy_from_slice(header);
        g.write(0, &blank);
        PageExclusive {
            pager: self,
            g: Some(g),
            blank: true,
        }
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

impl<'a, R: Runtime> Reservation<'a, R> {
    /// Hold a free page, to be formatted with `format`, `kind` and `owner` when allocated.
    ///
    /// Fails if an allocation bitmap page is damaged.
    pub fn page(&mut self, format: u16, kind: PageKind, owner: u32) -> Result<Pid, PageError> {
        let pid = self.pager.hold_free_page()?;
        self.pages.push(Reserved {
            pid,
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

    /// Returns the exclusive latch on `pid`, read without the file as [`Reservation::image`]
    /// formats it when not resident. Latch a page reserved here only this way.
    ///
    /// Postconditions, which allocation relies on:
    /// - Only this latch gives a page a blank frame. A reader other than the reservation meets
    ///   the page as any free page, and waits on this latch while it is held.
    /// - The allocation that commits `pid` logs its first change as an unchained full-page
    ///   image, so nothing of the free page's contents or evidence judges the new page.
    ///
    /// Panic if `pid` is not reserved here.
    pub fn latch(&self, pid: Pid) -> PageExclusive<'a, R> {
        let r = self.reserved(pid);
        let header = page::header_edit(r.format, r.kind, r.owner, 0);
        self.pager.latch_reserved(pid, &header)
    }

    fn reserved(&self, pid: Pid) -> &Reserved {
        self.pages
            .iter()
            .find(|r| r.pid == pid)
            .unwrap_or_else(|| panic!("{pid:?} is not reserved here"))
    }

    /// Returns the contents of page `pid` as allocation will format it, from offset
    /// [`FIRST_EDITABLE`].
    ///
    /// Panics if `pid` is not reserved here.
    pub fn image(&self, pid: Pid) -> Vec<u8> {
        let r = self.reserved(pid);
        let mut out = vec![0; self.pager.page_size];
        out[FIRST_EDITABLE..HEADER_LEN]
            .copy_from_slice(&page::header_edit(r.format, r.kind, r.owner, 0));
        out
    }
}

impl<R: Runtime> Drop for PageExclusive<'_, R> {
    /// Evict a reservation's blank frame no allocation committed, and refuse the page again as
    /// before it.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    fn drop(&mut self) {
        let Some(g) = self.g.take().filter(|_| self.blank) else {
            return;
        };
        let pid = Pid(g.pid());
        let mut m = self.pager.meta();
        let Some(refusal) = m.blanked.remove(&pid) else {
            return;
        };
        if let Some(damage) = refusal {
            m.damaged.insert(pid, damage);
        }
        m.reserved.remove(&pid);
        drop(m);
        g.evict();
        self.pager.unreserve(pid);
    }
}

impl<R: Runtime> Drop for Reservation<'_, R> {
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    fn drop(&mut self) {
        let mut m = self.pager.meta();
        for r in &self.pages {
            if !m.blanked.contains_key(&r.pid) {
                m.reserved.remove(&r.pid);
            }
        }
    }
}

/// What allocators share: the lowest bitmap group that may have a free page.
#[derive(Debug, Default)]
pub(crate) struct AllocState {
    pub(crate) group: u64,
}
