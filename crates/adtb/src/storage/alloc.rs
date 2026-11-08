use crate::runtime::Runtime;

pub(super) use self::owner::{Allocation, Hold};
use super::error::Damage;
use super::page::{self, FIRST_EDITABLE, HEADER_LEN, PageKind};
use super::pager::{PageError, PageExclusive, PageShared, Pager};
use super::record::{Edit, Record};
use super::recovery::Replaying;
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
        ((self.page_size() - HEADER_LEN) * 8) as u64
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
    fn map_is_sound(&self, page: &[u8], g: u64) -> bool {
        page::kind(page) == PageKind::AllocationBitmap as u16
            || g >= self.allocation().started() && page.iter().all(|&b| b == 0)
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
    pub(super) fn count_file_groups(&self, replaying: &Replaying) -> u64 {
        let pages = self.file_pages();
        let group = pages.saturating_sub(1) / self.bitmap_span();
        self.allocation().replayed_through(replaying, group + 1);
        pages
    }

    /// Refuse each page the allocation bitmap marks allocated past the end of the file, the first
    /// bitmap page when the file ends before it, the last bitmap page the file holds when that
    /// holds no bitmap, and each later bitmap page with an expected LSN as a blank copy of it.
    ///
    /// Preconditions: recovery has written every page the log holds and armed the read-in gate,
    /// and the file holds `pages`.
    pub(crate) fn refuse_cut_pages(&self, pages: u64) {
        let span = self.bitmap_span();
        let group = pages.saturating_sub(1) / span;
        self.refuse_cut_group(pages, group);
        let end = self.evidence().expected_end();
        let later = (group + 1..).map(|g| self.bitmap_page(g));
        for map in later.take_while(|m| m.0 < end) {
            let expected = self.evidence().expected(map).filter(|&l| l > Lsn(0));
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
    fn hold_free_page(&self) -> Result<Pid, PageError> {
        let span = self.bitmap_span();
        let mut state = self.allocation().lock(self.rt());
        let mut g = state.group();
        loop {
            let map = self.bitmap_page(g);
            let first = g * span;
            let bits = self.read_map(g)?[HEADER_LEN..].to_vec();
            let taken = |pid: u64| {
                let i = (pid - first) as usize;
                pid < Pid::FIRST_DATA.0
                    || pid == map.0
                    || bits[i / 8] >> (i % 8) & 1 == 1
                    || state.is_held(Pid(pid))
                    || self.refusal(Pid(pid)) == Some(Damage::Unreadable)
            };
            let Some(pid) = (first..first + span).find(|&p| !taken(p)) else {
                g += 1;
                continue;
            };
            state.hold(g, Pid(pid));
            return Ok(Pid(pid));
        }
    }

    /// Apply `changes` to their pages and log them, every page or none.
    ///
    /// Returns the LSN of the last record logged. Fails, changing nothing, if a page is damaged.
    ///
    /// Panic if a page appears twice, if the pool cannot hold every page at once, or if an
    /// edit writes the page's LSN or checksum or past its end.
    pub fn change(&self, changes: &[(Pid, Vec<Edit>)]) -> Result<Lsn, PageError> {
        let pids: Vec<Pid> = changes.iter().map(|(p, _)| *p).collect();
        let edits: Vec<Vec<Edit>> = changes.iter().map(|(_, e)| e.clone()).collect();
        self.with_latched(&pids, |pager, guards| pager.apply(guards, edits, None))
    }

    /// Modify `pids` together through `f`, which sees a copy of every page at once, and log what
    /// changed.
    ///
    /// Returns the LSN of the last record logged. Fails, calling nothing, if a page is damaged.
    ///
    /// Panic if `f` changes a page's LSN or checksum, or as [`Pager::change`] does.
    pub fn update<F>(&self, pids: &[Pid], f: F) -> Result<Lsn, PageError>
    where
        F: FnOnce(&mut [&mut [u8]]),
    {
        self.with_latched(pids, |pager, guards| {
            let edits = edits_by(guards.iter().map(|g| &***g), f);
            pager.apply(guards, edits, None)
        })
    }

    /// Change several pages under their exclusive latches through `f`, which sees a copy of
    /// each, logging every record in one batch.
    pub fn update_all<F>(&self, guards: &mut [&mut PageExclusive<'_, R>], f: F) -> Lsn
    where
        F: FnOnce(&mut [&mut [u8]]),
    {
        let edits = edits_by(guards.iter().map(|g| &***g), f);
        self.apply(guards, edits, None)
    }

    /// Change several pages under their exclusive latches through `f`, as
    /// [`Pager::update_all`] does, and log the commit record of transaction `xid` after the page
    /// records, in the same batch.
    ///
    /// Returns the LSN of the commit record.
    pub fn commit_all<F>(&self, xid: u64, guards: &mut [&mut PageExclusive<'_, R>], f: F) -> Lsn
    where
        F: FnOnce(&mut [&mut [u8]]),
    {
        let edits = edits_by(guards.iter().map(|g| &***g), f);
        self.apply(guards, edits, Some(self.commit_record(xid)))
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
        self.allocating(&new, guards, f, None)
    }

    /// Mark and format the pages of `new`, apply `f` to the pages under `guards`, and log all of
    /// it in one batch.
    fn allocating<'p, F>(
        &'p self,
        new: &Reservation<'_, R>,
        guards: &mut [&mut PageExclusive<'p, R>],
        f: F,
        tail: Option<Record>,
    ) -> Result<Lsn, PageError>
    where
        F: FnOnce(&mut [&mut [u8]]),
    {
        let span = self.bitmap_span();
        let mut state = self.allocation().lock(self.rt());
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
            if let Some(r) = new.pages.iter().find(|r| r.pid == g.pid()) {
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
                state.lower_group(g);
            }
            edits.push(diff(map, &bits));
        }
        let mut all: Vec<&mut PageExclusive<'p, R>> = Vec::new();
        for g in guards.iter_mut() {
            all.push(g);
        }
        for m in &mut maps {
            all.push(m);
        }
        let lsn = self.apply_freeing(&mut all, edits, Some(new), tail);
        state.committed(new.allocated(), groups.last().map(|g| g + 1));
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
        self.pager.latch_blank(pid, &header)
    }

    /// Returns the pages this reservation allocates.
    pub(super) fn allocated(&self) -> impl Iterator<Item = Pid> + '_ {
        self.pages.iter().map(|r| r.pid)
    }

    /// Returns the pages this reservation frees.
    pub(super) fn freed(&self) -> &[Pid] {
        &self.freed
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
        let mut out = vec![0; self.pager.page_size()];
        out[FIRST_EDITABLE..HEADER_LEN]
            .copy_from_slice(&page::header_edit(r.format, r.kind, r.owner, 0));
        out
    }
}

impl<R: Runtime> Drop for Reservation<'_, R> {
    fn drop(&mut self) {
        let mut state = self.pager.allocation().lock(self.pager.rt());
        for r in &self.pages {
            state.release(r.pid, Hold::Reservation);
        }
    }
}

mod owner {
    use std::collections::BTreeMap;
    use std::sync::MutexGuard;
    use std::sync::atomic::{AtomicU64, Ordering};

    use adtb_sys::lock::Lock;

    use crate::runtime::Runtime;
    use crate::storage::Pid;
    use crate::storage::recovery::Replaying;

    /// What allocators share: under the allocation lock, the lowest bitmap group that may have a
    /// free page and the free pages held; beside it, the bitmap groups a batch has started.
    ///
    /// The allocation lock is held across the search for a free page, which latches and may read
    /// in a bitmap page, and across marking pages allocated, which applies the batch. A holder of
    /// a reserved page's latch takes it to latch the page blank or end that latch.
    ///
    /// Invariants:
    ///
    /// - A free page is held while its reservation or a blank latch of it lives, and released
    ///   once neither does.
    /// - The started count only rises. A batch counts its groups only once it is in the log and
    ///   while their bitmap pages are latched, so a checkpoint that captures a lower count finds
    ///   those pages dirty and its redo point covers the batch.
    #[derive(Debug)]
    pub(in crate::storage) struct Allocation {
        state: Lock<State>,
        started: AtomicU64,
    }

    #[derive(Debug, Default)]
    struct State {
        group: u64,
        held: BTreeMap<Pid, Holds>,
    }

    /// What holds a free page.
    #[derive(Debug, Default)]
    struct Holds {
        reservation: bool,
        blank: bool,
    }

    /// One holder of a free page.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(in crate::storage) enum Hold {
        /// The reservation holding it, until the reservation drops.
        Reservation,
        /// The blank latch of it, until an allocation of it commits or the latch drops.
        Blank,
    }

    /// The allocation lock, held.
    pub(in crate::storage) struct AllocGuard<'a> {
        owner: &'a Allocation,
        state: MutexGuard<'a, State>,
    }

    impl Allocation {
        /// Returns the state of a file whose header counts `started` groups, holding no page.
        pub(in crate::storage) fn new(started: u64) -> Allocation {
            Allocation {
                state: Lock::new(State::default()),
                started: AtomicU64::new(started),
            }
        }

        pub(in crate::storage) fn lock<R: Runtime>(&self, rt: &R) -> AllocGuard<'_> {
            AllocGuard {
                owner: self,
                state: self.state.lock(rt),
            }
        }

        /// Returns the bitmap groups a batch has started, without the allocation lock.
        pub(in crate::storage) fn started(&self) -> u64 {
            self.started.load(Ordering::Acquire)
        }

        /// Count every group below `groups` as started, as recovery found the file.
        pub(in crate::storage) fn replayed_through(&self, _: &Replaying, groups: u64) {
            self.started.fetch_max(groups, Ordering::Release);
        }
    }

    impl AllocGuard<'_> {
        /// Returns the lowest bitmap group that may have a free page.
        pub(in crate::storage) fn group(&self) -> u64 {
            self.state.group
        }

        /// Returns whether a reservation or a blank latch holds `pid`.
        pub(in crate::storage) fn is_held(&self, pid: Pid) -> bool {
            self.state.held.contains_key(&pid)
        }

        /// Returns whether a blank latch holds `pid`.
        pub(in crate::storage) fn is_blank(&self, pid: Pid) -> bool {
            self.state.held.get(&pid).is_some_and(|h| h.blank)
        }

        /// Hold `pid`, a free page of group `g`, for a reservation, and search from `g` next.
        pub(in crate::storage) fn hold(&mut self, g: u64, pid: Pid) {
            self.state.group = g;
            let holds = self.state.held.entry(pid).or_default();
            holds.reservation = true;
        }

        /// Search from group `g` next when it is lower: a batch freed a page in it.
        pub(in crate::storage) fn lower_group(&mut self, g: u64) {
            self.state.group = self.state.group.min(g);
        }

        /// Hold `pid` by its blank latch too.
        ///
        /// Panic if no reservation holds `pid`.
        pub(in crate::storage) fn blank(&mut self, pid: Pid) {
            let holds = self.state.held.get_mut(&pid);
            let holds = holds.filter(|h| h.reservation);
            holds.expect("only a reserved page is latched blank").blank = true;
        }

        /// Stop holding `pid` by `hold`, releasing it once nothing holds it.
        pub(in crate::storage) fn release(&mut self, pid: Pid, hold: Hold) {
            let Some(holds) = self.state.held.get_mut(&pid) else {
                return;
            };
            match hold {
                Hold::Reservation => holds.reservation = false,
                Hold::Blank => holds.blank = false,
            }
            if !holds.reservation && !holds.blank {
                self.state.held.remove(&pid);
            }
        }

        /// End the blank latches of `allocated`, which a logged batch allocated, and count every
        /// group below `started` as started.
        ///
        /// Preconditions: the batch is in the log and the caller latches the bitmap page of each
        /// group it allocated in or freed from.
        pub(in crate::storage) fn committed(
            &mut self,
            allocated: impl Iterator<Item = Pid>,
            started: Option<u64>,
        ) {
            for pid in allocated {
                self.release(pid, Hold::Blank);
            }
            if let Some(started) = started {
                self.owner.started.fetch_max(started, Ordering::Release);
            }
        }
    }
}

/// Returns the edits `f` makes to copies of `pages`.
pub(super) fn edits_by<'p, F>(pages: impl Iterator<Item = &'p [u8]>, f: F) -> Vec<Vec<Edit>>
where
    F: FnOnce(&mut [&mut [u8]]),
{
    let originals: Vec<&[u8]> = pages.collect();
    let mut copies: Vec<Vec<u8>> = originals.iter().map(|p| p.to_vec()).collect();
    let mut views: Vec<&mut [u8]> = copies.iter_mut().map(Vec::as_mut_slice).collect();
    f(&mut views);
    originals
        .iter()
        .zip(&copies)
        .map(|(old, new)| diff(old, new))
        .collect()
}

#[cfg(test)]
mod tests {
    use adtb_sys::os::{Backend, OsRuntime};
    use adtb_testkit::scratch::Scratch;

    use super::{Allocation, Hold};
    use crate::storage::Pid;
    use crate::storage::recovery::replaying_for_test;

    const P: Pid = Pid(9);

    fn runtime(scratch: &Scratch) -> OsRuntime {
        OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 })
    }

    #[test]
    fn a_page_reserved_and_latched_blank_is_released_only_once_both_end() {
        let scratch = Scratch::new("storage", "alloc-holds");
        let rt = runtime(&scratch);
        for (first, second) in [
            (Hold::Reservation, Hold::Blank),
            (Hold::Blank, Hold::Reservation),
        ] {
            let alloc = Allocation::new(0);
            let mut held = alloc.lock(&rt);
            held.hold(0, P);
            held.blank(P);
            held.release(P, first);
            assert!(held.is_held(P), "the page left when {first:?} ended alone");
            held.release(P, second);
            assert!(!held.is_held(P), "the page stayed once both ended");
        }
    }

    #[test]
    fn a_committed_allocation_ends_the_blank_latch_and_the_reservation_releases_the_page() {
        let scratch = Scratch::new("storage", "alloc-commit");
        let rt = runtime(&scratch);
        let alloc = Allocation::new(0);
        let mut held = alloc.lock(&rt);
        held.hold(0, P);
        held.blank(P);
        held.committed([P].into_iter(), None);
        assert!(!held.is_blank(P));
        assert!(held.is_held(P));
        held.release(P, Hold::Reservation);
        assert!(!held.is_held(P));
    }

    #[test]
    fn the_started_count_never_falls() {
        let scratch = Scratch::new("storage", "alloc-started");
        let rt = runtime(&scratch);
        let alloc = Allocation::new(4);
        alloc.lock(&rt).committed(std::iter::empty(), None);
        assert_eq!(4, alloc.started());
        alloc.lock(&rt).committed(std::iter::empty(), Some(7));
        assert_eq!(7, alloc.started());
        alloc.lock(&rt).committed(std::iter::empty(), Some(3));
        assert_eq!(7, alloc.started());
        alloc.replayed_through(&replaying_for_test(), 5);
        assert_eq!(7, alloc.started());
        alloc.replayed_through(&replaying_for_test(), 9);
        assert_eq!(9, alloc.started());
    }
}
