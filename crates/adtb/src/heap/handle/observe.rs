//! The calls of a heap only tests and the test surface make.

use super::*;

impl Heap {
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
    #[expect(clippy::disallowed_methods, reason = "until heap owns page checks")]
    pub fn pointers_at<R: Runtime>(
        &self,
        pager: &Pager<R>,
        tid: Tid,
    ) -> Result<Option<Vec<Pointer>>, PageError> {
        let page = pager.read(Pid(tid.page()))?;
        if !page::is_heap(&page) || storage::owner(&page) != self.id {
            return Ok(None);
        }
        Ok(match page::get(&page, usize::from(tid.slot())) {
            Slot::Normal { at, .. } => Some(self.pointers(storage::format(&page), &page[at])),
            _ => None,
        })
    }

    /// Write `row` as the successor `xid` makes of the version at `at`, and point that version at
    /// it.
    ///
    /// The successor is heap-only when it fits on the old version's page and no field listed in
    /// `indexed`, the key fields of every index over this heap's constructor, changes.
    pub fn update<R: Runtime>(
        &self,
        pager: &Pager<R>,
        at: impl Into<WriteTarget>,
        xid: u64,
        row: &[Value],
        indexed: &[usize],
    ) -> Result<Updated, Error> {
        self.update_pruning(pager, at, xid, row, indexed, None)
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
