use std::collections::BTreeSet;
use std::sync::atomic::Ordering;

use crate::runtime::Runtime;

use super::page::{HEADER_LEN, PageKind};
use super::pager::{PageError, Pager};
use super::{Damage, Pid};

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
    /// On a page that failed its checksum or log position.
    Damage,
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
            Stopped::Damage
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
            Err(Stopped::Damage) if damaged => return Err(Stopped::Damage),
            Err(stop) => damaged = matches!(stop, Stopped::Damage),
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
}

impl Holders {
    /// Take what `walk` finds of `object`, whose damage marks nothing when `None`. Leave the
    /// mapping incomplete when the walk ends on anything but damage.
    pub fn walk(
        &mut self,
        object: Option<Quarantine>,
        walk: impl FnMut() -> Result<Vec<Pid>, Stopped>,
    ) {
        let holds = match settled(walk) {
            Ok(pages) => pages.contains(&self.pid),
            Err(Stopped::Damage) => true,
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
                None => self.incomplete = true,
            }
        }
    }
}

/// The marks in memory, and whether their page could be read at open.
#[derive(Debug, Default)]
pub(crate) struct Marks {
    pub(crate) set: BTreeSet<Quarantine>,
    pub(crate) page: Pid,
    pub(crate) unreadable: bool,
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
        self.any_mark
            .store(!marks.set.is_empty(), Ordering::Release);
        Ok(())
    }

    /// Returns every object quarantined.
    pub fn quarantined(&self) -> BTreeSet<Quarantine> {
        self.marks.lock(&self.rt).set.clone()
    }

    /// Returns whether any object is quarantined, without taking a lock.
    pub fn any_quarantined(&self) -> bool {
        self.any_mark.load(Ordering::Acquire)
    }

    /// Returns whether `object` is quarantined, by its own mark or the database's.
    pub fn is_quarantined(&self, object: Quarantine) -> bool {
        let marks = self.marks.lock(&self.rt);
        marks.set.contains(&object) || marks.set.contains(&Quarantine::Database)
    }

    /// Mark `object` quarantined. The mark is durable on return, for every caller marking it,
    /// and lasts until [`Pager::release`] removes it. Marking an object already marked and
    /// durable writes nothing.
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

    /// Remove the mark on `object`, once it is repaired. Durable on return.
    pub fn release(&self, object: Quarantine) -> Result<(), PageError> {
        self.change_marks(|set| {
            set.remove(&object);
        })
    }

    /// Returns the page holding the marks.
    pub fn quarantine_page(&self) -> Pid {
        self.marks.lock(&self.rt).page
    }

    fn change_marks(&self, f: impl FnOnce(&mut BTreeSet<Quarantine>)) -> Result<(), PageError> {
        let (page, unreadable) = {
            let m = self.marks.lock(&self.rt);
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
            self.any_mark
                .store(!marks.set.is_empty(), Ordering::Release);
            marks.set.clone()
        };
        guard.update(|p| encode(p, &set));
        let lsn = super::page::lsn(&guard);
        drop(guard);
        self.wait_durable(lsn);
        Ok(())
    }

    /// Returns what the damage at `pid` quarantines. `walk` reports the pages of every object to
    /// the [`Holders`]. A page in no object quarantines `unheld`, once every walk is complete.
    ///
    /// Postconditions:
    ///
    /// - A page the allocation bitmap calls free, before or after the walk, quarantines nothing
    ///   unless a walk found it.
    /// - A walk stopped by damage quarantines its object. One stopped by an I/O failure, or on a
    ///   shape it could not follow in all its tries, leaves the page unmapped: `unheld` then plays
    ///   no part.
    /// - The database's mark stands alone, as it covers every object.
    pub fn holders_of(
        &self,
        pid: Pid,
        unheld: Option<Quarantine>,
        walk: impl FnOnce(&mut Holders),
    ) -> BTreeSet<Quarantine> {
        if self.called_free(pid) {
            return BTreeSet::new();
        }
        let mut h = Holders {
            pid,
            held: BTreeSet::new(),
            incomplete: false,
        };
        walk(&mut h);
        if h.held.contains(&Quarantine::Database) {
            return BTreeSet::from([Quarantine::Database]);
        }
        if !h.held.is_empty() || h.incomplete || self.called_free(pid) {
            return h.held;
        }
        unheld.into_iter().collect()
    }

    /// Returns whether the allocation bitmap calls `pid` free, or cannot be read to say.
    fn called_free(&self, pid: Pid) -> bool {
        match self.is_free(pid) {
            Ok(free) => free,
            Err(e) => e.damage == Damage::Unreadable,
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
