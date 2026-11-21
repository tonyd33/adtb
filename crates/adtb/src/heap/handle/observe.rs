//! The calls of a heap only tests and the test surface make.

use super::*;
use crate::heap::Tested;

/// One position of a heap's directory, as a test prints it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Laid {
    /// A position that names no page.
    Hole,
    /// A page and what each of its slots holds, with the header of each version.
    Page {
        pid: Pid,
        kind: Option<storage::PageKind>,
        format: u16,
        flagged: bool,
        mapped: bool,
        slots: Vec<(Slot, Option<Header>)>,
    },
}

impl Heap {
    /// Returns each position of the heap's directory in order, with the page it names: the
    /// page's kind and row format, whether its header flags it all-visible, whether the
    /// visibility map has its bit set, and its slots. A page whose slot table does not fit it
    /// lists no slots.
    pub fn layout<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Laid>, PageError> {
        let mut out = Vec::new();
        for leaf in self.space.check(pager)?.leaves {
            let Some((pid, _)) = leaf else {
                out.push(Laid::Hole);
                continue;
            };
            let guard = pager.read(pid)?;
            let page = HeapPage::any(&guard);
            let slots = page.map_or_else(Vec::new, |page| {
                (0..page.slot_count())
                    .map(|s| match page.get(s) {
                        Slot::Normal { flags, at } => {
                            let header = Header::read(&page[at.clone()]);
                            (Slot::Normal { flags, at }, Some(header))
                        }
                        slot => (slot, None),
                    })
                    .collect()
            });
            let (kind, format) = (storage::kind(&guard), storage::format(&guard));
            let flagged = page.is_some_and(HeapPage::all_visible);
            drop(guard);
            out.push(Laid::Page {
                pid,
                kind: storage::PageKind::from_u16(kind),
                format,
                flagged,
                mapped: self.all_visible(pager, pid)?,
                slots,
            });
        }
        Ok(out)
    }

    /// Add a newer row format whose rows `upgrade` makes from rows of the newest one. Returns its
    /// format version.
    pub fn add_format(&mut self, row: Vec<FieldType>, upgrade: Upgrade) -> Result<u16, Error> {
        let layout = Layout::new(row);
        let arrangement = arrangement_of(&layout, self.page_size)?;
        self.formats.push(Format {
            layout,
            arrangement,
            upgrade: Some(upgrade),
        });
        Ok(self.newest())
    }

    /// Returns the pointers the version at `tid` holds, if a version of this heap is there.
    pub fn pointers_at<R: Runtime>(
        &self,
        pager: &Pager<R>,
        tid: Tid,
    ) -> Result<Option<Vec<Pointer>>, PageError> {
        loop {
            let Some((format, _, bytes)) = self.raw(pager, tid)? else {
                return Ok(None);
            };
            match self.pointers(format, &bytes) {
                Ok(pointers) => return Ok(Some(pointers)),
                Err(_) => self.refuse_row(pager, Pid(tid.page()), ())?,
            }
        }
    }

    /// Returns whether the header of page `pid` has the all-visible flag set.
    pub fn flagged_all_visible<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
    ) -> Result<bool, PageError> {
        Ok(HeapPage::any(&pager.read(pid)?).is_some_and(HeapPage::all_visible))
    }

    /// Update as [`Heap::update_pruning`] does, pruning nothing.
    pub fn update<R: Runtime>(
        &self,
        pager: &Pager<R>,
        at: impl Into<WriteTarget>,
        xid: Xid,
        row: &[Value],
        indexed: &[usize],
    ) -> Result<Updated, Error> {
        let put = self.put_successor(pager, at.into(), xid, row, indexed, None)?;
        Ok(put.kept(&Tested)?.0)
    }

    /// Copy `v` as [`Heap::copy_version_pruning`] does, pruning nothing.
    pub fn copy_version<R: Runtime>(
        &self,
        held: &Vacuuming<'_, R>,
        from: &Heap,
        v: &Version,
    ) -> Result<Option<Tid>, Error> {
        self.put_copy(held, from, v, None)
    }

    /// Insert `row` as [`Heap::insert_pruning`] does, pruning nothing.
    pub fn insert<R: Runtime>(
        &self,
        pager: &Pager<R>,
        xid: Xid,
        row: &[Value],
    ) -> Result<Tid, Error> {
        self.put_version(pager, xid, row, None)?.kept(&Tested)
    }

    /// Move the row whose version is at `at` into heap `to` as [`Heap::retag_pruning`] does,
    /// pruning nothing.
    pub fn retag<R: Runtime>(
        &self,
        pager: &Pager<R>,
        at: impl Into<WriteTarget>,
        xid: Xid,
        to: &Heap,
        row: &[Value],
    ) -> Result<Tid, Error> {
        let put = self.put_moved(pager, at.into(), xid, to, row, None)?;
        Ok(put.kept(&Tested)?.0)
    }

    /// Returns what is wrong with the heap's pages, directory and visibility map, directory
    /// entries promising too little included.
    pub fn verify<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<String>, PageError> {
        let mut problems = self.check(pager)?;
        problems.extend(self.unsettled(pager)?);
        Ok(problems
            .into_iter()
            .map(|(pid, what)| format!("{pid:?} {what}"))
            .collect())
    }

    /// Set every inner directory entry to what its child holds, each in an update of its own.
    pub fn settle_directory<R: Runtime>(&self, pager: &Pager<R>) -> Result<(), PageError> {
        self.space.settle(pager)
    }

    /// Returns the number of positions of the heap's directory that name no page.
    pub fn holes<R: Runtime>(&self, pager: &Pager<R>) -> Result<usize, PageError> {
        let check = self.space.check(pager)?;
        Ok(check.leaves.iter().filter(|l| l.is_none()).count())
    }

    /// Returns whether the heap has no page left: every version it held was freed.
    pub fn is_empty<R: Runtime>(&self, pager: &Pager<R>) -> Result<bool, PageError> {
        Ok(self.space.pages(pager)?.is_empty())
    }

    /// Free every page the heap owns in one pager update, and with it the heap.
    ///
    /// Preconditions: the heap has no page left, and nothing refers to it once the update is
    /// applied.
    ///
    /// Panic if the heap has a page left.
    pub fn drop_empty<R: Runtime>(self, pager: &Pager<R>) -> Result<(), PageError> {
        assert!(self.is_empty(pager)?, "heap {} has pages left", self.id);
        let mut gone = pager.reserve();
        for pid in self.owned_pages(pager)? {
            gone.free(pid);
        }
        pager.update_with(gone, &[], |_| {})?;
        Ok(())
    }
}
