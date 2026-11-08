use std::collections::BTreeSet;
use std::sync::atomic::Ordering;

use crate::runtime::Runtime;

use super::page::Verdict;
use super::page::{FIRST_EDITABLE, HEADER_LEN, PageKind};
use super::pager::{PageError, PageExclusive, Pager};
use super::{Damage, Lsn, Pid};
use crate::mvcc::Settling;

/// What the allocation bitmap says of a page.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Allocation {
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

/// The marks in memory, and whether their page could be read at open.
#[derive(Debug, Default)]
pub(crate) struct Marks {
    pub(crate) set: BTreeSet<Quarantine>,
    /// Marks held for this open only, never written.
    pub(crate) held: BTreeSet<Quarantine>,
    pub(crate) page: Pid,
    pub(crate) unreadable: bool,
}

impl Marks {
    fn any(&self) -> bool {
        !self.set.is_empty() || !self.held.is_empty()
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
    #[doc(hidden)]
    pub fn load_marks(&self, page: Pid) -> Result<(), PageError> {
        let read = match self.read(page) {
            Ok(p) => decode(&p),
            Err(e) if e.damage == Damage::Unreadable => return Err(e),
            Err(_) => None,
        };
        let mut marks = self.marks.lock(&self.rt);
        marks.page = page;
        match read {
            Some(set) => marks.set = set,
            None => {
                marks.set = BTreeSet::from([Quarantine::Database]);
                marks.unreadable = true;
            }
        }
        self.any_mark.store(marks.any(), Ordering::Release);
        Ok(())
    }

    /// Returns every object quarantined.
    pub(crate) fn quarantined(&self) -> BTreeSet<Quarantine> {
        let marks = self.marks.lock(&self.rt);
        marks.set.union(&marks.held).copied().collect()
    }

    /// Returns whether any object is quarantined, without taking a lock.
    pub fn any_quarantined(&self) -> bool {
        self.any_mark.load(Ordering::Acquire)
    }

    /// Returns whether `object` is quarantined, by its own mark or the database's.
    pub(crate) fn is_quarantined(&self, object: Quarantine) -> bool {
        let marks = self.marks.lock(&self.rt);
        [object, Quarantine::Database]
            .iter()
            .any(|o| marks.set.contains(o) || marks.held.contains(o))
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
            self.marks.lock(&self.rt).set.insert(object);
            self.any_mark.store(true, Ordering::Release);
        })
    }

    /// Mark `object` quarantined for this open only. From then on no change of the marks is
    /// written.
    pub(crate) fn hold_quarantine(&self, object: Quarantine) {
        self.marks.lock(&self.rt).held.insert(object);
        self.any_mark.store(true, Ordering::Release);
    }

    /// Remove the mark on `object`, once it is repaired. Durable on return.
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
        let (page, unreadable) = {
            let m = self.marks.lock(&self.rt);
            (m.page, m.unreadable)
        };
        if unreadable {
            return Err(PageError {
                pid: page,
                damage: Damage::Checksum,
            });
        }
        let mut marks_page = self.exclusive(page)?;
        let (mut g, blank) = self.latch_overwriting(pid)?;
        let mut new = self.reserve();
        new.free(from);
        let set = {
            let mut set = self.marks.lock(&self.rt).set.clone();
            if let Some(release) = &release {
                set.remove(release);
            }
            set
        };
        let lsn = self.update_all_with(new, &mut [&mut g, &mut marks_page], |p| {
            p[0][FIRST_EDITABLE..].copy_from_slice(&contents[FIRST_EDITABLE..]);
            encode(p[1], &set);
        })?;
        {
            let mut marks = self.marks.lock(&self.rt);
            if let Some(release) = &release {
                marks.set.remove(release);
            }
            self.any_mark.store(marks.any(), Ordering::Release);
        }
        drop(marks_page);
        self.wait_durable(lsn);
        if blank {
            self.write_out(pid, &g);
        }
        Ok(())
    }

    /// Returns page `pid` as the file holds it, with the refusal when the file's copy lost a write
    /// it must have, for the commit log to settle it from the log.
    ///
    /// Fail with any other refusal, or when the file's copy is blank, past the log, or fails its
    /// checksum.
    pub(crate) fn read_stale(
        &self,
        pid: Pid,
        _: &Settling,
    ) -> Result<(Vec<u8>, Option<PageError>), PageError> {
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
            Ok((buf, Verdict::Valid)) => Ok((buf.to_vec(), Some(e))),
            _ => Err(e),
        }
    }

    /// Change page `pid`, which [`Pager::read_stale`] returned refused, from the file's copy
    /// `copy` to `contents`, logging the change as one from that copy.
    pub(crate) fn rewrite_stale(
        &self,
        pid: Pid,
        _: &Settling,
        copy: &[u8],
        contents: &[u8],
    ) -> Result<(), PageError> {
        let mut g = match self.exclusive(pid) {
            Ok(g) => g,
            Err(e) if e.pid == pid && matches!(e.damage, Damage::Stale { .. }) => {
                self.latch_restored(pid, copy)
            }
            Err(e) => return Err(e),
        };
        let new = self.reserve();
        self.update_all_with(new, &mut [&mut g], |p| {
            p[0][FIRST_EDITABLE..].copy_from_slice(&contents[FIRST_EDITABLE..]);
        })?;
        Ok(())
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
        self.marks.lock(&self.rt).page
    }

    fn change_marks(&self, f: impl FnOnce(&mut BTreeSet<Quarantine>)) -> Result<(), PageError> {
        let (page, unreadable) = {
            let mut m = self.marks.lock(&self.rt);
            if !m.held.is_empty() {
                f(&mut m.set);
                self.any_mark.store(m.any(), Ordering::Release);
                return Ok(());
            }
            (m.page, m.unreadable)
        };
        if unreadable {
            return Err(self.read(page).err().unwrap_or(PageError {
                pid: page,
                damage: Damage::Checksum,
            }));
        }
        let mut guard = self.exclusive(page)?;
        let set = {
            let mut marks = self.marks.lock(&self.rt);
            f(&mut marks.set);
            self.any_mark.store(marks.any(), Ordering::Release);
            marks.set.clone()
        };
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
        self.walked.fetch_add(1, Ordering::Relaxed);
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
        if !h.held.is_empty() || h.unmarked || self.allocation(pid) == Allocation::Free {
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
    pub(super) fn allocation(&self, pid: Pid) -> Allocation {
        match self.is_free(pid) {
            Ok(true) => Allocation::Free,
            Ok(false) => Allocation::Allocated,
            Err(_) => Allocation::Unknown,
        }
    }

    /// Format the marks page `pid`, reserved by `new`, empty.
    pub(crate) fn format_marks(
        &self,
        new: super::Reservation<'_, R>,
        pid: Pid,
    ) -> Result<(), PageError> {
        self.update_with(new, &[pid], |p| encode(p[0], &BTreeSet::new()))?;
        self.marks.lock(&self.rt).page = pid;
        Ok(())
    }
}
