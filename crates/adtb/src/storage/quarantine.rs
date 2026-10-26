use std::collections::BTreeSet;

use crate::runtime::Runtime;

use super::page::{HEADER_LEN, PageKind};
use super::pager::{PageError, Pager};
use super::{Lsn, Pid};

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
    pub(crate) fn load_marks(&self, page: Pid) {
        let read = self.read(page).ok().and_then(|p| decode(&p));
        let mut marks = self.marks.lock(&self.rt);
        marks.page = page;
        match read {
            Some(set) => marks.set = set,
            None => {
                marks.set = BTreeSet::from([Quarantine::Database]);
                marks.unreadable = true;
            }
        }
    }

    /// Returns every object quarantined.
    pub fn quarantined(&self) -> BTreeSet<Quarantine> {
        self.marks.lock(&self.rt).set.clone()
    }

    /// Returns whether `object` is quarantined, by its own mark or the database's.
    pub fn is_quarantined(&self, object: Quarantine) -> bool {
        let marks = self.marks.lock(&self.rt);
        marks.set.contains(&object) || marks.set.contains(&Quarantine::Database)
    }

    /// Mark `object` quarantined. The mark is durable on return and lasts until
    /// [`Pager::release`] removes it.
    ///
    /// Fails if the page holding the marks is damaged.
    pub fn quarantine(&self, object: Quarantine) -> Result<(), PageError> {
        self.change_marks(|set| {
            set.insert(object);
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
                damage: super::Damage::Checksum,
            }));
        }
        let mut guard = self.exclusive(page)?;
        let mut set = self.marks.lock(&self.rt).set.clone();
        f(&mut set);
        let lsn: Lsn = guard.update(|p| encode(p, &set));
        drop(guard);
        if lsn < self.log().next_lsn() {
            self.wait_durable(lsn);
        }
        self.marks.lock(&self.rt).set = set;
        Ok(())
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
