use std::collections::BTreeSet;

use crate::runtime::Runtime;

use super::evidence::{Against, Judged, file_copy};
use super::page::Verdict;
use super::page::{FIRST_EDITABLE, HEADER_LEN, PageKind};
use super::pager::{PageError, PageExclusive, Pager};
use super::{Damage, Lsn, Pid};
use crate::mvcc::Settling;

/// A status page whose file's copy lost a write, as [`Pager::read_stale`] judged it. Dropped
/// unsettled, it refuses the page and records its damage.
#[must_use = "a stale copy dropped unsettled refuses its page"]
pub(crate) struct StaleCopy<'a, R: Runtime> {
    pager: &'a Pager<R>,
    lost: Option<PageError>,
}

impl<'a, R: Runtime> StaleCopy<'a, R> {
    fn of(pager: &'a Pager<R>, pid: Pid, damage: Damage) -> StaleCopy<'a, R> {
        StaleCopy {
            pager,
            lost: Some(PageError { pid, damage }),
        }
    }

    fn take(&mut self) -> PageError {
        let lost = self.lost.take();
        lost.expect("a stale copy holds its refusal until it is settled or refused")
    }

    /// Refuse the page and record its damage. Returns the refusal.
    ///
    /// Preconditions: as for [`Pager::damage`].
    pub(crate) fn refuse(mut self) -> PageError {
        let e = self.take();
        self.pager.damage(e.pid, e.damage)
    }

    /// Latch the page over `copy`, the file's copy, record it settled, then change it to
    /// `contents`, logging the change as one from that copy.
    ///
    /// Fail, recording nothing, when the page is refused other than as stale or resident.
    pub(crate) fn settle(
        mut self,
        settling: &Settling,
        copy: &[u8],
        contents: &[u8],
    ) -> Result<(), PageError> {
        let e = self.take();
        let pager = self.pager;
        let mut g = pager.latch_restored(e, copy)?;
        pager.record_settled(settling, e);
        let new = pager.reserve();
        pager.update_all_with(new, &mut [&mut g], |p| {
            p[0][FIRST_EDITABLE..].copy_from_slice(&contents[FIRST_EDITABLE..]);
        })?;
        Ok(())
    }
}

impl<R: Runtime> Drop for StaleCopy<'_, R> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        if let Some(e) = self.lost.take() {
            self.pager.damage(e.pid, e.damage);
        }
    }
}

/// What the allocation bitmap says of a page.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Bitmapped {
    Allocated,
    Free,
    /// Its bitmap page is refused.
    Unknown,
}

/// What a detected corruption stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Quarantine {
    /// Every relation: the commit log, or the marks themselves, are damaged.
    Database,
    Heap(u32),
    Index(u32),
}

const FORMAT: u16 = 1;
const COUNT: usize = HEADER_LEN;
const ENTRIES: usize = HEADER_LEN + 8;
const ENTRY: usize = 8;

impl Quarantine {
    fn encode(self) -> [u8; ENTRY] {
        let (tag, id) = match self {
            Quarantine::Database => (1u32, 0),
            Quarantine::Heap(id) => (2, id),
            Quarantine::Index(id) => (3, id),
        };
        let mut out = [0; ENTRY];
        out[..4].copy_from_slice(&tag.to_le_bytes());
        out[4..].copy_from_slice(&id.to_le_bytes());
        out
    }

    fn decode(bytes: &[u8]) -> Option<Quarantine> {
        let tag = u32::from_le_bytes(bytes[..4].try_into().unwrap());
        let id = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        match tag {
            1 => Some(Quarantine::Database),
            2 => Some(Quarantine::Heap(id)),
            3 => Some(Quarantine::Index(id)),
            _ => None,
        }
    }
}

/// Returns the marks a quarantine page holds, or `None` if it does not decode.
pub(crate) fn decode(page: &[u8]) -> Option<BTreeSet<Quarantine>> {
    if super::page::kind(page) != PageKind::Quarantine as u16 {
        return None;
    }
    let n = u64::from_le_bytes(page[COUNT..COUNT + 8].try_into().unwrap()) as usize;
    if ENTRIES + n * ENTRY > page.len() {
        return None;
    }
    (0..n)
        .map(|i| Quarantine::decode(&page[ENTRIES + i * ENTRY..ENTRIES + (i + 1) * ENTRY]))
        .collect()
}

fn encode(page: &mut [u8], marks: &BTreeSet<Quarantine>) {
    assert!(
        ENTRIES + marks.len() * ENTRY <= page.len(),
        "{} quarantine marks overflow their page",
        marks.len()
    );
    page[COUNT..].fill(0);
    page[COUNT..COUNT + 8].copy_from_slice(&(marks.len() as u64).to_le_bytes());
    for (i, m) in marks.iter().enumerate() {
        page[ENTRIES + i * ENTRY..ENTRIES + (i + 1) * ENTRY].copy_from_slice(&m.encode());
    }
}

/// Why a walk of an object's pages stopped.
#[derive(Clone, Copy, Debug)]
pub enum Stopped {
    /// On the page named, which failed its checksum or log position.
    Damage(Pid),
    /// On a shape the walk could not follow, which proves nothing: the walk takes no latch, so
    /// the object may have changed under it.
    Shape,
    /// On an I/O failure, which says nothing of the object.
    Io,
}

impl From<PageError> for Stopped {
    fn from(e: PageError) -> Stopped {
        if e.damage == Damage::Unreadable {
            Stopped::Io
        } else {
            Stopped::Damage(e.pid)
        }
    }
}

/// How many times the mapping of a damaged page runs a walk that keeps stopping.
const WALK_TRIES: usize = 16;

/// Returns the outcome of `walk`, repeated while it stops. Damage counts once two walks in a row
/// stop on it, since one walk may have followed a link off a changing object. `Stopped::Shape`
/// when the tries run out.
fn settled(mut walk: impl FnMut() -> Result<Vec<Pid>, Stopped>) -> Result<Vec<Pid>, Stopped> {
    let mut damaged = false;
    for _ in 0..WALK_TRIES {
        match walk() {
            Ok(pages) => return Ok(pages),
            Err(Stopped::Io) => return Err(Stopped::Io),
            Err(Stopped::Damage(at)) if damaged => return Err(Stopped::Damage(at)),
            Err(stop) => damaged = matches!(stop, Stopped::Damage(_)),
        }
    }
    Err(Stopped::Shape)
}

/// What the walks of every object find of one damaged page.
#[derive(Debug)]
pub struct Holders {
    pid: Pid,
    held: BTreeSet<Quarantine>,
    incomplete: bool,
    /// Whether an object whose damage marks nothing holds the page.
    unmarked: bool,
    /// The pages whose damage stopped a walk.
    met: BTreeSet<Pid>,
}

impl Holders {
    /// Take what `walk` finds of `object`, whose damage marks nothing when `None`. Leave the
    /// mapping incomplete when the walk ends on anything but damage, or on damage of an object
    /// whose damage marks nothing.
    pub fn walk(
        &mut self,
        object: Option<Quarantine>,
        walk: impl FnMut() -> Result<Vec<Pid>, Stopped>,
    ) {
        let holds = match settled(walk) {
            Ok(pages) => pages.contains(&self.pid),
            Err(Stopped::Damage(at)) => {
                self.met.insert(at);
                self.incomplete |= object.is_none();
                object.is_some()
            }
            Err(Stopped::Shape | Stopped::Io) => {
                self.incomplete = true;
                false
            }
        };
        if holds {
            match object {
                Some(o) => {
                    self.held.insert(o);
                }
                None => self.unmarked = true,
            }
        }
    }
}

pub(super) use self::marks::{Destination, Marks};

mod marks {
    use std::collections::BTreeSet;
    use std::sync::MutexGuard;
    use std::sync::atomic::{AtomicBool, Ordering};

    use adtb_sys::lock::Lock;

    use super::Quarantine;
    use crate::runtime::Runtime;
    use crate::storage::Pid;

    /// The marks in memory, under the marks lock, a leaf, and whether any is set, read without
    /// it.
    ///
    /// Invariants:
    ///
    /// - The flag is set exactly while a mark is set or held; it changes with the marks in one
    ///   hold of the lock.
    /// - The set mirrors the marks page unless a mark is held for this open or the page could
    ///   not be read at open.
    #[derive(Debug)]
    pub(in crate::storage) struct Marks {
        state: Lock<State>,
        any: AtomicBool,
    }

    #[derive(Debug, Default)]
    struct State {
        set: BTreeSet<Quarantine>,
        /// Marks held for this open only, never written.
        held: BTreeSet<Quarantine>,
        page: Pid,
        unreadable: bool,
    }

    /// Where a change of the marks goes.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(in crate::storage) enum Destination {
        /// Memory only: a mark is held for this open.
        Memory,
        /// Nowhere: the page, named, could not be read at open.
        Unreadable(Pid),
        /// The page named.
        Page(Pid),
    }

    impl Marks {
        /// Returns the marks of a file whose header names `page`, none set until they are
        /// loaded.
        pub(in crate::storage) fn new(page: Pid) -> Marks {
            Marks {
                state: Lock::new(State {
                    page,
                    ..State::default()
                }),
                any: AtomicBool::new(false),
            }
        }

        fn lock<R: Runtime>(&self, rt: &R) -> Guard<'_> {
            Guard {
                any: &self.any,
                state: self.state.lock(rt),
            }
        }

        /// Take `read`, the marks the page `page` holds, `None` when it does not decode, which
        /// marks the database and stops every write of the marks.
        pub(in crate::storage) fn load<R: Runtime>(
            &self,
            rt: &R,
            page: Pid,
            read: Option<BTreeSet<Quarantine>>,
        ) {
            let mut m = self.lock(rt);
            m.state.page = page;
            match read {
                Some(set) => m.state.set = set,
                None => {
                    m.state.set = BTreeSet::from([Quarantine::Database]);
                    m.state.unreadable = true;
                }
            }
        }

        /// Hold the marks on page `page`, formatted empty.
        pub(in crate::storage) fn formatted<R: Runtime>(&self, rt: &R, page: Pid) {
            self.lock(rt).state.page = page;
        }

        /// Returns the page holding the marks.
        pub(in crate::storage) fn page<R: Runtime>(&self, rt: &R) -> Pid {
            self.lock(rt).state.page
        }

        /// Returns the page holding the marks, or `Err` with it when it could not be read at open.
        pub(in crate::storage) fn readable_page<R: Runtime>(&self, rt: &R) -> Result<Pid, Pid> {
            let m = self.lock(rt);
            match m.state.unreadable {
                true => Err(m.state.page),
                false => Ok(m.state.page),
            }
        }

        /// Returns where a change of the marks goes now.
        pub(in crate::storage) fn destination<R: Runtime>(&self, rt: &R) -> Destination {
            let m = self.lock(rt);
            if !m.state.held.is_empty() {
                Destination::Memory
            } else if m.state.unreadable {
                Destination::Unreadable(m.state.page)
            } else {
                Destination::Page(m.state.page)
            }
        }

        /// Returns whether any object is quarantined, without the lock.
        pub(in crate::storage) fn any(&self) -> bool {
            self.any.load(Ordering::Acquire)
        }

        /// Returns every mark, set or held.
        pub(in crate::storage) fn all<R: Runtime>(&self, rt: &R) -> BTreeSet<Quarantine> {
            let m = self.lock(rt);
            m.state.set.union(&m.state.held).copied().collect()
        }

        /// Returns whether any of `objects` is marked, set or held.
        pub(in crate::storage) fn any_of<R: Runtime>(
            &self,
            rt: &R,
            objects: &[Quarantine],
        ) -> bool {
            let m = self.lock(rt);
            let marked = |o| m.state.set.contains(o) || m.state.held.contains(o);
            objects.iter().any(marked)
        }

        /// Returns the set as `f` would change it, changing nothing.
        pub(in crate::storage) fn changed<R: Runtime>(
            &self,
            rt: &R,
            f: impl FnOnce(&mut BTreeSet<Quarantine>),
        ) -> BTreeSet<Quarantine> {
            let mut set = self.lock(rt).state.set.clone();
            f(&mut set);
            set
        }

        /// Change the set through `f`, in memory only.
        pub(in crate::storage) fn change<R: Runtime>(
            &self,
            rt: &R,
            f: impl FnOnce(&mut BTreeSet<Quarantine>),
        ) {
            f(&mut self.lock(rt).state.set);
        }

        /// Change the set through `f`. Returns the set, for the caller to write to the marks
        /// page.
        ///
        /// Preconditions: the caller latches the marks page, which [`Marks::destination`] named
        /// as the place of the change before the latch.
        pub(in crate::storage) fn change_on_page<R: Runtime>(
            &self,
            rt: &R,
            f: impl FnOnce(&mut BTreeSet<Quarantine>),
        ) -> BTreeSet<Quarantine> {
            let mut m = self.lock(rt);
            f(&mut m.state.set);
            m.state.set.clone()
        }

        /// Hold `object` marked for this open only.
        pub(in crate::storage) fn hold<R: Runtime>(&self, rt: &R, object: Quarantine) {
            self.lock(rt).state.held.insert(object);
        }
    }

    /// The marks lock, held. Its drop sets the flag from the marks.
    struct Guard<'a> {
        any: &'a AtomicBool,
        state: MutexGuard<'a, State>,
    }

    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            let any = !self.state.set.is_empty() || !self.state.held.is_empty();
            self.any.store(any, Ordering::Release);
        }
    }
}

impl<R: Runtime> Pager<R> {
    /// Returns a reservation that allocates the page holding the marks, and the page.
    pub(crate) fn reserve_marks(&self) -> Result<(super::Reservation<'_, R>, Pid), PageError> {
        let mut new = self.reserve();
        let pid = new.page(FORMAT, PageKind::Quarantine, 0)?;
        Ok((new, pid))
    }

    /// Read the marks from their page into memory. A damaged page quarantines the database.
    ///
    /// Fail, marking nothing, if the page cannot be read.
    pub(crate) fn load_marks(&self, page: Pid) -> Result<(), PageError> {
        let read = match self.read(page) {
            Ok(p) => decode(&p),
            Err(e) if e.damage == Damage::Unreadable => return Err(e),
            Err(_) => None,
        };
        self.marks().load(self.rt(), page, read);
        Ok(())
    }

    /// Returns every object quarantined.
    pub(crate) fn quarantined(&self) -> BTreeSet<Quarantine> {
        self.marks().all(self.rt())
    }

    /// Returns whether any object is quarantined, without taking a lock.
    pub fn any_quarantined(&self) -> bool {
        self.marks().any()
    }

    /// Returns whether `object` is quarantined, by its own mark or the database's.
    pub(crate) fn is_quarantined(&self, object: Quarantine) -> bool {
        self.marks()
            .any_of(self.rt(), &[object, Quarantine::Database])
    }

    /// Mark `object` quarantined. The mark is durable on return, for every caller marking it,
    /// and lasts until [`Pager::release`] removes it. Marking an object already marked and
    /// durable writes nothing. While a mark is held for this open, the mark lives in memory only,
    /// until the pager closes.
    ///
    /// Precondition: the calling thread holds no page latch.
    ///
    /// Fail if the page holding the marks is damaged. The mark then holds in memory until the
    /// pager closes.
    pub fn quarantine(&self, object: Quarantine) -> Result<(), PageError> {
        self.change_marks(|set| {
            set.insert(object);
        })
        .inspect_err(|_| {
            self.marks().change(self.rt(), |set| {
                set.insert(object);
            });
        })
    }

    /// Mark `object` quarantined for this open only. From then on no change of the marks is
    /// written.
    pub(crate) fn hold_quarantine(&self, object: Quarantine) {
        self.marks().hold(self.rt(), object);
    }

    /// Remove the mark on `object`, once it is repaired. Durable on return.
    #[cfg(any(test, feature = "testing"))]
    pub fn release(&self, object: Quarantine) -> Result<(), PageError> {
        self.change_marks(|set| {
            set.remove(&object);
        })
    }

    /// Returns the exclusive latch on `pid`, over a blank page when it is damaged. The next change
    /// to a blank page logs its whole image, which no earlier record need lead to; dropping the
    /// latch before one refuses the page again.
    ///
    /// Fail on an I/O failure.
    fn latch_overwriting(&self, pid: Pid) -> Result<(PageExclusive<'_, R>, bool), PageError> {
        match self.exclusive(pid) {
            Ok(g) => Ok((g, false)),
            Err(e) if e.pid == pid && e.damage != Damage::Unreadable => {
                Ok((self.latch_overwritten(pid), true))
            }
            Err(e) => Err(e),
        }
    }

    /// Give `pid` the contents of `from` and free `from`, removing the mark on `release` if one is
    /// named and held, in one batch. A damaged `pid` is overwritten unread. Returns once that is
    /// durable.
    ///
    /// Preconditions: nothing refers to `from` after the call.
    ///
    /// Fail, changing nothing, if `from`, the marks' page or an allocation bitmap page cannot be
    /// read.
    pub fn replace_contents(
        &self,
        pid: Pid,
        from: Pid,
        release: Option<Quarantine>,
    ) -> Result<(), PageError> {
        let contents = self.read(from)?.to_vec();
        let page = self
            .marks()
            .readable_page(self.rt())
            .map_err(|pid| PageError {
                pid,
                damage: Damage::Checksum,
            })?;
        let mut marks_page = self.exclusive(page)?;
        let (mut g, blank) = self.latch_overwriting(pid)?;
        let mut new = self.reserve();
        new.free(from);
        let unmark = |set: &mut BTreeSet<Quarantine>| {
            if let Some(release) = &release {
                set.remove(release);
            }
        };
        let set = self.marks().changed(self.rt(), unmark);
        let lsn = self.update_all_with(new, &mut [&mut g, &mut marks_page], |p| {
            p[0][FIRST_EDITABLE..].copy_from_slice(&contents[FIRST_EDITABLE..]);
            encode(p[1], &set);
        })?;
        self.marks().change(self.rt(), unmark);
        drop(marks_page);
        self.wait_durable(lsn);
        if blank {
            self.write_out(pid, &g);
        }
        Ok(())
    }

    /// Returns page `pid` as the file holds it, with its judgement as stale when the file's copy
    /// lost a write it must have, for the commit log to settle it from the log. A page neither
    /// resident nor refused is judged without refusing it or recording its damage.
    ///
    /// Fail with any other refusal, or when the file's copy is blank, past the log, or fails its
    /// checksum.
    pub(crate) fn read_stale(
        &self,
        pid: Pid,
        _: &Settling,
    ) -> Result<(Vec<u8>, Option<StaleCopy<'_, R>>), PageError> {
        if let Some((copy, damage)) = self.judge_quietly(pid) {
            return Ok((copy, Some(StaleCopy::of(self, pid, damage))));
        }
        let e = match self.read_formatted(pid) {
            Ok(page) => return Ok((page.to_vec(), None)),
            Err(e) => e,
        };
        let Damage::Stale { expected, found } = e.damage else {
            return Err(e);
        };
        if found == Lsn(0) || found > expected {
            return Err(e);
        }
        match self.read_disk(pid) {
            Ok((buf, Verdict::Valid)) => {
                Ok((buf.to_vec(), Some(StaleCopy::of(self, pid, e.damage))))
            }
            _ => Err(e),
        }
    }

    /// Returns the file's copy of `pid` and its damage when the page is neither resident nor
    /// refused, and the copy is valid, allocated and behind its expected LSN with an LSN of its
    /// own, refusing nothing.
    fn judge_quietly(&self, pid: Pid) -> Option<(Vec<u8>, Damage)> {
        let held = self.hold(pid);
        if held.resident() || self.refusal(pid).is_some() {
            return None;
        }
        let read = self.read_disk(pid);
        let judged = self.judge(pid, file_copy(&read), Against::Expected);
        let Judged::Behind(damage @ Damage::Stale { found, .. }) = judged else {
            return None;
        };
        let (buf, verdict) = read.ok()?;
        let lost = verdict == Verdict::Valid && found != Lsn(0);
        (lost && self.bitmapped(pid) != Bitmapped::Free).then(|| (buf.to_vec(), damage))
    }

    /// Free damaged page `pid`, overwriting it, so no later read of the file finds the damage.
    ///
    /// Preconditions: nothing refers to `pid`, and it is allocated.
    ///
    /// Fail, leaving `pid` as it was, if an allocation bitmap page cannot be read.
    pub fn scrap(&self, pid: Pid) -> Result<(), PageError> {
        let (mut g, _) = self.latch_overwriting(pid)?;
        let mut new = self.reserve();
        new.free(pid);
        self.update_all_with(new, &mut [&mut g], |p| p[0][HEADER_LEN] = 1)?;
        self.write_out(pid, &g);
        Ok(())
    }

    /// Returns the page holding the marks.
    pub fn quarantine_page(&self) -> Pid {
        self.marks().page(self.rt())
    }

    fn change_marks(&self, f: impl FnOnce(&mut BTreeSet<Quarantine>)) -> Result<(), PageError> {
        let page = match self.marks().destination(self.rt()) {
            Destination::Memory => {
                self.marks().change(self.rt(), f);
                return Ok(());
            }
            Destination::Unreadable(page) => {
                return Err(self.read(page).err().unwrap_or(PageError {
                    pid: page,
                    damage: Damage::Checksum,
                }));
            }
            Destination::Page(page) => page,
        };
        self.change_marks_on(page, f)
    }

    /// Change the marks through `f` and write them to their page `page`, durable on return,
    /// whether or not a mark was held for this open since `page` was named.
    ///
    /// Fail, changing nothing, if the page is refused.
    fn change_marks_on(
        &self,
        page: Pid,
        f: impl FnOnce(&mut BTreeSet<Quarantine>),
    ) -> Result<(), PageError> {
        let mut guard = self.exclusive(page)?;
        let set = self.marks().change_on_page(self.rt(), f);
        guard.update(|p| encode(p, &set));
        let lsn = super::page::lsn(&guard);
        drop(guard);
        self.wait_durable(lsn);
        Ok(())
    }

    /// Returns what the damage at `pid` quarantines, `None` when the mapping is incomplete and
    /// found no holder. An object whose damage marks nothing that holds the page completes it.
    /// `walk` reports the pages of every object to the [`Holders`]. A page in no
    /// object quarantines `unheld`, once every walk is complete.
    ///
    /// Postconditions:
    ///
    /// - A page the allocation bitmap calls free, before or after the walk, quarantines nothing
    ///   unless a walk found it. A page whose bitmap page is refused counts as allocated.
    /// - A walk stopped by damage quarantines its object, or leaves the mapping incomplete when
    ///   the object's damage marks nothing. One stopped by an I/O failure, or on a shape it could
    ///   not follow in all its tries, leaves the mapping incomplete: `unheld` then plays no part.
    /// - An allocation bitmap page quarantines the database, with no walk.
    /// - The database's mark stands alone, as it covers every object.
    pub fn holders_of(
        &self,
        pid: Pid,
        unheld: Option<Quarantine>,
        walk: impl FnOnce(&mut Holders),
    ) -> Option<BTreeSet<Quarantine>> {
        self.holders_meeting(pid, unheld, &mut BTreeSet::new(), walk)
    }

    /// Returns what [`Pager::holders_of`] does, adding to `met` each page whose damage stopped a
    /// walk.
    pub fn holders_meeting(
        &self,
        pid: Pid,
        unheld: Option<Quarantine>,
        met: &mut BTreeSet<Pid>,
        walk: impl FnOnce(&mut Holders),
    ) -> Option<BTreeSet<Quarantine>> {
        if self.is_bitmap_page(pid) {
            return Some(BTreeSet::from([Quarantine::Database]));
        }
        self.evidence().walked_one();
        let mut h = Holders {
            pid,
            held: BTreeSet::new(),
            incomplete: false,
            unmarked: false,
            met: BTreeSet::new(),
        };
        walk(&mut h);
        met.append(&mut h.met);
        if h.held.contains(&Quarantine::Database) {
            return Some(BTreeSet::from([Quarantine::Database]));
        }
        if h.incomplete && h.held.is_empty() && !h.unmarked {
            return None;
        }
        if !h.held.is_empty() || h.unmarked || self.bitmapped(pid) == Bitmapped::Free {
            return Some(h.held);
        }
        Some(unheld.into_iter().collect())
    }

    /// Hold `pids`, pages of a tree a swap replaced, as retired until each one's own free.
    pub fn retire(&self, pids: impl IntoIterator<Item = Pid>) {
        self.evidence().retire(pids);
    }

    /// Stop holding `pids` as retired: the swap that would have replaced their tree failed.
    pub fn unretire(&self, pids: &[Pid]) {
        self.evidence().unretire(pids);
    }

    /// Returns the pages held as retired, none of them freed since.
    pub fn retired(&self) -> Vec<Pid> {
        self.evidence().retired()
    }

    /// Returns what the allocation bitmap says of `pid`.
    pub(super) fn bitmapped(&self, pid: Pid) -> Bitmapped {
        match self.is_free(pid) {
            Ok(true) => Bitmapped::Free,
            Ok(false) => Bitmapped::Allocated,
            Err(_) => Bitmapped::Unknown,
        }
    }

    /// Format the marks page `pid`, reserved by `new`, empty.
    pub(crate) fn format_marks(
        &self,
        new: super::Reservation<'_, R>,
        pid: Pid,
    ) -> Result<(), PageError> {
        self.update_with(new, &[pid], |p| encode(p[0], &BTreeSet::new()))?;
        self.marks().formatted(self.rt(), pid);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use adtb_sys::os::{Backend, Faults, OsRuntime};
    use adtb_testkit::scratch::Scratch;

    use super::{Destination, Marks, Quarantine};
    use crate::storage::Pid;
    use crate::storage::pager::{Options, Pager};

    const PAGE: Pid = Pid(5);

    #[test]
    fn the_any_flag_follows_the_marks_after_every_change() {
        let scratch = Scratch::new("storage", "marks-any");
        let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
        let marks = Marks::new(PAGE);
        marks.load(&rt, PAGE, Some(BTreeSet::new()));
        assert!(!marks.any());
        let heap = Quarantine::Heap(1);
        let index = Quarantine::Index(2);
        let set = marks.change_on_page(&rt, |s| {
            s.insert(heap);
        });
        assert_eq!(BTreeSet::from([heap]), set);
        assert!(marks.any());
        marks.change_on_page(&rt, |s| {
            s.insert(index);
        });
        marks.change_on_page(&rt, |s| {
            s.remove(&heap);
        });
        assert!(marks.any());
        let set = marks.change_on_page(&rt, |s| {
            s.remove(&index);
        });
        assert_eq!(BTreeSet::new(), set);
        assert!(!marks.any());
        marks.hold(&rt, Quarantine::Database);
        assert!(marks.any());
        assert_eq!(Destination::Memory, marks.destination(&rt));
        marks.change(&rt, |s| {
            s.insert(heap);
        });
        marks.change(&rt, |s| {
            s.remove(&heap);
        });
        assert!(marks.any());
    }

    #[test]
    fn marks_whose_page_does_not_decode_mark_the_database_and_take_no_write() {
        let scratch = Scratch::new("storage", "marks-unreadable");
        let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
        let marks = Marks::new(PAGE);
        assert!(!marks.any());
        marks.load(&rt, PAGE, None);
        assert!(marks.any());
        assert_eq!(BTreeSet::from([Quarantine::Database]), marks.all(&rt));
        assert_eq!(Destination::Unreadable(PAGE), marks.destination(&rt));
    }

    #[test]
    fn a_mark_whose_page_was_named_before_a_hold_is_on_the_page_after_a_crash() {
        let scratch = Scratch::new("storage", "marks-hold-race");
        let rt = || {
            OsRuntime::with_faults(
                scratch.path(),
                Backend::Pool { threads: 1 },
                Faults::none(4096),
            )
        };
        let options = Options {
            pool_frames: 16,
            max_pages: 1024,
            ..Options::new("marks-hold-race.adt")
        };
        let pager = Pager::create(rt(), &options).expect("create pager");
        let Destination::Page(page) = pager.marks().destination(pager.rt()) else {
            panic!("a new file's marks go to their page");
        };
        pager.hold_quarantine(Quarantine::Database);
        pager
            .change_marks_on(page, |s| {
                s.insert(Quarantine::Heap(1));
            })
            .expect("write the marks");
        drop(pager);
        let (pager, _) = Pager::open(rt(), &options).expect("reopen");
        assert_eq!(BTreeSet::from([Quarantine::Heap(1)]), pager.quarantined());
        pager.close();
    }
}
