use std::cmp::{Ordering, Reverse};

use crate::faults::{self, Step};
use crate::runtime::Runtime;
use crate::storage::{PageError, Pager, Pid, Reservation};

use super::dir::{Above, Fit};
use super::handle::Heap;
use super::large::LargeStore;
use super::page::{HeapPage, HeapPageMut, PageCopy};
use super::row::Pointer;
use super::stage::Stage;
use super::{Error, Refused};

/// A latch's place in the one order every thread takes latches in, shared or exclusive: heap
/// pages by page id, then large-value store pages by page id, then directory pages by level, the
/// highest first, and by page id within a level, then visibility map pages by page id. The
/// order is checked by a stage in debug builds and under `latch-checks`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Rank {
    Heap(Pid),
    Store(Pid),
    Dir { level: Level, pid: Pid },
    Vm(Pid),
}

/// The level of a directory page as the latch order sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Level {
    /// The directory's root, above every other level.
    Root,
    At(u32),
}

impl Level {
    fn key(self) -> (u8, Reverse<u32>) {
        match self {
            Level::Root => (0, Reverse(0)),
            Level::At(l) => (1, Reverse(l)),
        }
    }
}

impl Rank {
    /// Returns the rank of directory page `pid` at level `level`.
    pub fn dir(level: u32, pid: Pid) -> Rank {
        Rank::Dir {
            level: Level::At(level),
            pid,
        }
    }

    pub fn pid(self) -> Pid {
        match self {
            Rank::Heap(pid) | Rank::Store(pid) | Rank::Dir { pid, .. } | Rank::Vm(pid) => pid,
        }
    }

    fn key(self) -> (u8, (u8, Reverse<u32>), Pid) {
        match self {
            Rank::Heap(pid) => (0, (0, Reverse(0)), pid),
            Rank::Store(pid) => (1, (0, Reverse(0)), pid),
            Rank::Dir { level, pid } => (2, level.key(), pid),
            Rank::Vm(pid) => (3, (0, Reverse(0)), pid),
        }
    }
}

impl Ord for Rank {
    fn cmp(&self, other: &Rank) -> Ordering {
        self.key().cmp(&other.key())
    }
}

impl PartialOrd for Rank {
    fn partial_cmp(&self, other: &Rank) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The page that took a version in a [`HeapWrite`].
#[derive(Clone, Copy)]
pub(super) enum Took<'h> {
    /// A heap-only successor on its predecessor's page.
    HeapOnly(&'h Heap, Pid),
    /// A version on a page found for it, which the heap tries first for its next insert.
    Found(&'h Heap, Pid),
}

/// The failure of a heap write's commit, and whether it came after the update that wrote the
/// change's pages.
#[derive(Debug)]
pub(super) struct Failed {
    refused: Refused,
    written: bool,
}

impl Failed {
    /// Returns whether the failure came after the update that wrote the change's pages.
    pub(super) fn written(&self) -> bool {
        self.written
    }
}

impl From<Failed> for Refused {
    fn from(f: Failed) -> Refused {
        f.refused
    }
}

impl From<Failed> for Error {
    fn from(f: Failed) -> Error {
        f.refused.into()
    }
}

/// What is left of a change once its update wrote its pages.
struct After<'a, T> {
    /// The values the change no longer points at, and their store.
    released: Option<(LargeStore, &'a [Pointer])>,
    above: Above,
    done: T,
}

impl After<'static, ()> {
    /// Returns what is left of a change that released nothing: settling `above`.
    fn settling(above: Above) -> After<'static, ()> {
        After {
            released: None,
            above,
            done: (),
        }
    }
}

/// Count one version fewer pointing at each value the change whose update `update` reports
/// released, each in an update of its own, then settle the directory entries above the changed
/// ones; a step that fails stops none of the others. Returns the change's result, or the first
/// failure, its update's included.
#[track_caller]
fn finish<R: Runtime, T>(
    pager: &Pager<R>,
    update: Result<After<'_, T>, Refused>,
) -> Result<T, Failed> {
    let After {
        released,
        above,
        done,
    } = update.map_err(|refused| Failed {
        refused,
        written: false,
    })?;
    let released = match released {
        Some((store, pointers)) => release_each(pager, &store, pointers),
        None => Ok(()),
    };
    let settled = released.and(settle(pager, above).map_err(Refused::from));
    settled.map(|()| done).map_err(|refused| Failed {
        refused,
        written: true,
    })
}

/// One change to heap pages and to the store, directory and map pages that describe them. Its
/// exclusive latches are taken in [`Rank`] order.
pub(super) struct HeapWrite<'p, 'h, R: Runtime> {
    pager: &'p Pager<R>,
    stage: Stage<'p, R>,
    /// The heap of each heap page [`HeapWrite::latch`] latched.
    latched: Vec<(&'h Heap, Pid)>,
}

impl<'p, 'h, R: Runtime> HeapWrite<'p, 'h, R> {
    pub fn new(pager: &'p Pager<R>) -> HeapWrite<'p, 'h, R> {
        HeapWrite::above(pager, None)
    }

    /// Returns an empty write made while latches up to `floor` are held.
    pub fn above(pager: &'p Pager<R>, floor: Option<Rank>) -> HeapWrite<'p, 'h, R> {
        HeapWrite {
            pager,
            stage: Stage::above(floor),
            latched: Vec::new(),
        }
    }

    pub fn stage(&mut self) -> &mut Stage<'p, R> {
        &mut self.stage
    }

    /// Latch the heap pages `pages` names, in page id order.
    ///
    /// Postcondition: a commit clears the all-visible flag and the map bit of each staged heap
    /// page whose staged copy differs from the page as latched.
    pub fn latch(&mut self, pages: &[(&'h Heap, Pid)]) -> Result<(), PageError> {
        let mut sorted = pages.to_vec();
        sorted.sort_by_key(|&(_, pid)| pid);
        sorted.dedup_by_key(|&mut (_, pid)| pid);
        for (heap, pid) in sorted {
            self.stage.page(self.pager, Rank::Heap(pid))?;
            self.latched.push((heap, pid));
        }
        Ok(())
    }

    /// Write the change: count one more version pointing at each of `retain` in the store of the
    /// heap `took` names; clear the all-visible flag and the map bit of every staged heap page the
    /// change wrote; set the level-0 directory entry of the page `took` names from its staged
    /// copy; write every staged page in one update; point the heap's next insert at a page found
    /// for the version. The counts and the map bits change in updates of their own. A failure
    /// before the update counts each of `retain` it counted off again. After the update, count
    /// one version fewer pointing at each of `released` in that store and settle the directory
    /// entries above, as `finish` does.
    ///
    /// Panic if a staged heap page the change wrote was not latched through [`HeapWrite::latch`].
    #[track_caller]
    pub fn commit(
        self,
        took: Took<'h>,
        retain: &[Pointer],
        released: &[Pointer],
    ) -> Result<(), Failed> {
        let pager = self.pager;
        let (Took::HeapOnly(heap, pid) | Took::Found(heap, pid)) = took;
        let store = heap.large();
        let mut retained = store.retained(pager);
        let update = retained
            .retain_apart(self.stage.top(), retain)
            .and_then(|()| self.write_pages(Some((heap, pid))));
        let update = match update {
            Ok(above) => {
                retained.written();
                if let Took::Found(..) = took {
                    heap.hint_at(pid);
                }
                Ok(After {
                    released: Some((store, released)),
                    above,
                    done: (),
                })
            }
            Err(e) => {
                // The write, and every latch it held, is gone: the counts raised go back.
                drop(retained);
                Err(e)
            }
        };
        finish(pager, update)
    }

    /// Write a change to versions in place: clear the all-visible flag and the map bit of every
    /// staged heap page it wrote, and write every staged page in one update.
    ///
    /// Panic if a staged heap page the change wrote was not latched through [`HeapWrite::latch`].
    #[track_caller]
    pub fn commit_in_place(self) -> Result<(), Failed> {
        let pager = self.pager;
        finish(pager, self.write_pages(None).map(After::settling))
    }

    /// Clear the map bits of the change, set the level-0 directory entry of the heap page `took`
    /// names, and write every staged page in one update. Returns the entries above to settle.
    ///
    /// Postcondition: the latches of the write are released when this returns.
    fn write_pages(mut self, took: Option<(&Heap, Pid)>) -> Result<Above, Refused> {
        let pager = self.pager;
        self.clear_written()?;
        let above = match took {
            Some((heap, pid)) => {
                fault(Step::Reflect)?;
                let newest = heap.newest();
                heap.space
                    .reflect(&mut self.stage, pager, pid, newest, Fit::Took)?
            }
            None => Above::default(),
        };
        fault(Step::Write)?;
        self.stage.commit(pager)?;
        Ok(above)
    }

    /// Clear the all-visible flag of each staged heap page whose staged copy changed and has it,
    /// and its map bit in an update of its own.
    fn clear_written(&mut self) -> Result<(), Refused> {
        let pager = self.pager;
        for pid in self.stage.heap_pages() {
            if !self.stage.changed(pid) {
                continue;
            }
            let heap = self
                .latched
                .iter()
                .find_map(|&(heap, p)| (p == pid).then_some(heap))
                .unwrap_or_else(|| panic!("heap page {pid:?} was written but not latched"));
            let copy = self.stage.page(pager, Rank::Heap(pid))?;
            let mut page = HeapPageMut::any(copy).expect("a written heap page is a heap page");
            if !page.view().all_visible() {
                continue;
            }
            page.set_all_visible(false);
            self.clear_bit(heap, pid)?;
        }
        Ok(())
    }

    /// Clear the map bit of `pid`, a page of `heap`, in an update of its own.
    fn clear_bit(&mut self, heap: &Heap, pid: Pid) -> Result<(), Refused> {
        let vm = heap.vm(&mut self.stage, self.pager)?;
        let mut bit = Stage::above(self.stage.top());
        vm.put(&mut bit, self.pager, pid, false)?;
        fault(Step::ClearBit)?;
        Ok(bit.commit(self.pager)?)
    }

    /// Set the level-0 directory entry of `pid`, a latched page of `heap` that could not take a
    /// version, from its staged copy, write it and settle the entries above.
    #[track_caller]
    pub fn commit_refused(self, heap: &Heap, pid: Pid) -> Result<(), Failed> {
        let pager = self.pager;
        finish(pager, self.write_refused(heap, pid).map(After::settling))
    }

    fn write_refused(mut self, heap: &Heap, pid: Pid) -> Result<Above, Refused> {
        let pager = self.pager;
        let newest = heap.newest();
        let above = heap
            .space
            .reflect(&mut self.stage, pager, pid, newest, Fit::Refused)?;
        self.stage.commit(pager)?;
        Ok(above)
    }

    /// Write `pruned` as page `pid` of `heap`, latched, with its all-visible flag and map bit set
    /// when `all_visible` holds and cleared otherwise. A page left empty leaves the directory and
    /// is freed in the same update; any other gets its directory entry from its staged copy as
    /// [`Fit::Pruned`] says. After the update, count one version fewer pointing at each of
    /// `released` and settle the entries above, as `finish` does. Returns whether the page was
    /// freed.
    ///
    /// Precondition: the map has the path to `pid` when `all_visible` holds.
    #[track_caller]
    pub fn commit_pruned(
        self,
        heap: &Heap,
        pid: Pid,
        pruned: PageCopy,
        all_visible: bool,
        released: &[Pointer],
    ) -> Result<bool, Failed> {
        let pager = self.pager;
        let update = self.write_pruned(heap, pid, pruned, all_visible);
        let update = update.map(|(above, removed)| After {
            released: Some((heap.large(), released)),
            above,
            done: removed,
        });
        finish(pager, update)
    }

    /// Write the update [`HeapWrite::commit_pruned`] makes. Returns the entries above to settle
    /// and whether the page was freed.
    fn write_pruned(
        mut self,
        heap: &Heap,
        pid: Pid,
        mut pruned: PageCopy,
        all_visible: bool,
    ) -> Result<(Above, bool), Refused> {
        let pager = self.pager;
        let copy = self.stage.read(pager, Rank::Heap(pid))?;
        let page = HeapPage::any(&copy).expect("a page pruned is a heap page");
        let vm = heap.vm(&mut self.stage, pager)?;
        let bit = vm.get(&mut self.stage, pager, pid)?;
        let mut after = HeapPageMut::any(&mut pruned).expect("a pruned page is a heap page");
        let removed = after.view().is_empty();
        let visible = all_visible && !removed;
        after.set_all_visible(visible);
        if bit && !visible {
            self.clear_bit(heap, pid)?;
        }
        let roomier = after.view().free() > page.free();
        self.stage
            .put(pager, Rank::Heap(pid), pruned.into_bytes())?;
        let mut gone = pager.reserve();
        let above = if removed {
            gone.free(pid);
            heap.space
                .vacate(&mut self.stage, pager, pid, page.position())?
        } else {
            let fit = Fit::Pruned { roomier };
            heap.space
                .reflect(&mut self.stage, pager, pid, heap.newest(), fit)?
        };
        if visible && !bit {
            vm.put(&mut self.stage, pager, pid, true)?;
        }
        self.stage.commit_with(pager, gone)?;
        if removed {
            heap.forget_hint(pid);
        }
        Ok((above, removed))
    }

    /// Write a change to large-value store pages in one update, allocating and freeing the pages
    /// of `new`, then settle `above`.
    #[track_caller]
    pub fn commit_store(self, new: Reservation<'_, R>, above: Above) -> Result<(), Failed> {
        let update = self.stage.commit_with(self.pager, new).map(|()| above);
        finish(
            self.pager,
            update.map(After::settling).map_err(Refused::from),
        )
    }
}

/// Write `stage` in one update, allocating and freeing the pages of `new`, then settle `above`.
#[track_caller]
pub(super) fn commit_settling<R: Runtime>(
    stage: Stage<'_, R>,
    pager: &Pager<R>,
    new: Reservation<'_, R>,
    above: Above,
) -> Result<(), PageError> {
    stage.commit_with(pager, new)?;
    settle(pager, above)
}

/// Count one version fewer pointing at each of `pointers` in `store`, each in an update of its
/// own. Returns the first failure once every release ran.
#[track_caller]
pub(super) fn release_each<R: Runtime>(
    pager: &Pager<R>,
    store: &LargeStore,
    pointers: &[Pointer],
) -> Result<(), Refused> {
    let mut first = Ok(());
    for &p in pointers {
        let released = match faults::injected(Step::Release) {
            Some(e) => Err(Refused::Page(e)),
            None => store.release(pager, p),
        };
        first = first.and(released);
    }
    first
}

#[track_caller]
fn fault(step: Step) -> Result<(), Refused> {
    faults::injected(step).map_or(Ok(()), |e| Err(Refused::Page(e)))
}

#[track_caller]
fn settle<R: Runtime>(pager: &Pager<R>, above: Above) -> Result<(), PageError> {
    if let Some(e) = faults::injected(Step::Settle) {
        return Err(e);
    }
    above.settle(pager)
}
