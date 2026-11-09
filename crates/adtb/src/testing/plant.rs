use crate::catalog::Catalog;
use crate::key::{Entry, Tid, Value};
use crate::runtime::Runtime;
use crate::storage::{Damage, PageError, PageKind, Pid, Quarantine};
use crate::write;

use super::{Failed, Object, Testing};

/// A place in the engine where a planted panic fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Site {
    /// The background worker's next read of a page.
    BackgroundRead,
    /// The next write of kept findings, after its first replacement.
    KeptWrite,
}

/// A fault a test plants in a running engine, by the path a real fault of its kind takes where
/// one exists.
#[derive(Clone, Debug, PartialEq)]
pub enum Plant {
    /// Refuse `page` from now on as if it failed as `damage` says, recording the damage.
    Refuse { page: Pid, damage: Damage },
    /// Refuse every page of the store of kept rows as failing its checksum.
    RefuseKeptStore,
    /// Set the mark, durably.
    Mark(Quarantine),
    /// Clear the mark, durably.
    Unmark(Quarantine),
    /// Panic at `site` with `message`, once.
    Panic { site: Site, message: String },
    /// Add `entry` to `index` with no row behind it. Fails as unchanged when the index holds it.
    AddEntry { index: Object, entry: Entry },
    /// Add an entry to the tree of `index` as `entry` encodes under its schema, whatever its
    /// constructor. Fails as unchanged when the tree holds it.
    AddRawEntry { index: Object, entry: Entry },
    /// Remove `entry` from `index`, leaving its row. Fails as unchanged when the index lacks it.
    DropEntry { index: Object, entry: Entry },
    /// Swap two entries of leaf `page` of the tree of `index`, breaking its order.
    MisorderLeaf { index: Object, page: Pid },
    /// Mark the version at `at` of `heap` hot-updated with successor `next`.
    NextTid { heap: Object, at: Tid, next: Tid },
    /// Free `page` in the allocation bitmap, whatever still names it.
    Free(Pid),
    /// Write `bytes` at offset `at` of `page`, as one logged change.
    Write {
        page: Pid,
        at: usize,
        bytes: Vec<u8>,
    },
    /// Flip a bit of the owner in the header of `page`, as one logged change.
    FlipOwner(Pid),
    /// Give `page` the contents of `from` and free `from`, durably.
    Replace { page: Pid, from: Pid },
    /// Leave the catalog marked as naming the write path's page of generated counters, as a
    /// crash inside that change leaves it.
    NamingSequences,
    /// Allocate a page of `kind` and owner `owner` that no structure names, its contents zero.
    /// Answers [`Planted::Page`].
    Orphan { kind: PageKind, owner: u32 },
    /// Commit `rows` into heap `heap` as one transaction of the write path, with no index entry
    /// and no check. Answers [`Planted::Tids`], where each row landed.
    Insert { heap: Object, rows: Vec<Vec<Value>> },
    /// Read the marks from `page` as an open does, replacing the marks held in memory.
    LoadMarks(Pid),
    /// Fail as the write path fails on meeting `error`, setting the marks it sets. Answers
    /// [`Planted::Refused`].
    FailOn(PageError),
}

/// What a plant answers.
#[derive(Debug)]
pub enum Planted {
    Done,
    Page(Pid),
    Tids(Vec<Tid>),
    Refused(write::Error),
}

impl Planted {
    /// Returns the page an `Orphan` plant allocated.
    pub fn page(self) -> Option<Pid> {
        match self {
            Planted::Page(p) => Some(p),
            _ => None,
        }
    }

    /// Returns where each row of an `Insert` plant landed.
    pub fn tids(self) -> Option<Vec<Tid>> {
        match self {
            Planted::Tids(t) => Some(t),
            _ => None,
        }
    }

    /// Returns the error a `FailOn` plant failed with.
    pub fn refused(self) -> Option<write::Error> {
        match self {
            Planted::Refused(e) => Some(e),
            _ => None,
        }
    }
}

impl<R: Runtime> Testing<'_, R> {
    /// Plant `plant`.
    pub fn plant(self, plant: Plant) -> Result<Planted, Failed> {
        let pager = self.db.tested_pager();
        match plant {
            Plant::Refuse { page, damage } => {
                pager.damage(page, damage);
            }
            Plant::RefuseKeptStore => self.db.refuse_kept_store(),
            Plant::Mark(q) => pager.quarantine(q)?,
            Plant::Unmark(q) => pager.release(q)?,
            Plant::Panic { site, message } => self.db.panic_at(site, message),
            Plant::AddEntry { index, entry } => {
                changed(self.index_object(index)?.insert(pager, &entry)?)?;
            }
            Plant::AddRawEntry { index, entry } => {
                let index = self.index_object(index)?;
                let bytes = index.schema().encode_entry(&entry);
                changed(index.tree().insert(pager, &bytes)?)?;
            }
            Plant::DropEntry { index, entry } => {
                changed(self.index_object(index)?.delete(pager, &entry)?)?;
            }
            Plant::MisorderLeaf { index, page } => self.tree(index)?.misorder_leaf(pager, page)?,
            Plant::NextTid { heap, at, next } => {
                self.heap_object(heap)?.plant_next(pager, at, next)?;
            }
            Plant::Free(page) => {
                pager.free(page)?;
            }
            Plant::Write { page, at, bytes } => {
                pager.update(&[page], |p| {
                    p[0][at..at + bytes.len()].copy_from_slice(&bytes);
                })?;
            }
            Plant::FlipOwner(page) => {
                pager.update(&[page], |p| crate::storage::flip_owner(p[0]))?;
            }
            Plant::Replace { page, from } => pager.replace_contents(page, from, None)?,
            Plant::NamingSequences => Catalog::naming_sequences(pager, pager.catalog_root())?,
            Plant::Orphan { kind, owner } => {
                return Ok(Planted::Page(pager.allocate(1, kind, owner)?));
            }
            Plant::Insert { heap, rows } => return self.insert(heap, &rows).map(Planted::Tids),
            Plant::LoadMarks(page) => pager.load_marks(page)?,
            Plant::FailOn(e) => {
                let w = self.db.tested_db();
                return Ok(Planted::Refused(w.quarantining(write::Error::Page(e))));
            }
        }
        Ok(Planted::Done)
    }

    fn insert(self, heap: Object, rows: &[Vec<Value>]) -> Result<Vec<Tid>, Failed> {
        let (w, pager) = (self.db.tested_db(), self.db.tested_pager());
        let heap = self.heap_object(heap)?;
        let tx = w.begin().map_err(Failed::Write)?;
        let xid = tx.xid().expect("a read-write transaction has an id");
        let tids = rows.iter().map(|row| heap.insert(pager, xid, row));
        let tids = tids.collect::<Result<Vec<_>, _>>()?;
        w.commit(tx).map_err(Failed::Write)?;
        Ok(tids)
    }
}

/// Fail as unchanged when a change found nothing to change.
fn changed(done: bool) -> Result<(), Failed> {
    if done { Ok(()) } else { Err(Failed::Unchanged) }
}
