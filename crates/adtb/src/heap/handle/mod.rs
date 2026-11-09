use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::key::{Tid, Value};
use crate::mvcc::{Fate, Writers, Xid};
use crate::runtime::Runtime;
use crate::storage::{self, Damage, PageError, Pager, Pid, Stopped};

use super::dir::{Space, max_fanout, need_category, page_category};
use super::large::LargeStore;
use super::page::{Arrangement, HeapPage, HeapPageMut, Kinded, Owned, PageCopy, Slot, align8};
use super::row::{FieldType, Layout, POINTER_LEN, Pointer, Stored};
use super::stage::Stage;
use super::version::{Flags, HEADER_LEN, Header};
use super::vm::Vm;
use super::write::{HeapWrite, Rank, Took};
use super::{Error, WriteTarget, fixed, slotted};
use super::{chain, prune};

#[cfg(any(test, feature = "testing"))]
mod observe;

/// Converts a row of the previous format into a row of this one.
pub type Upgrade = Box<dyn Fn(Vec<Value>) -> Vec<Value> + Send + Sync>;

struct Format {
    layout: Layout,
    arrangement: Arrangement,
    upgrade: Option<Upgrade>,
}

/// A row format of a heap, read from a page whose format byte names one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FormatNo(u16);

/// A row version as a heap returns it, its row in the heap's newest format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub tid: Tid,
    pub header: Header,
    pub flags: Flags,
    pub row: Vec<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Updated {
    pub tid: Tid,
    /// The new version is on the old one's page and needs no index entries.
    pub heap_only: bool,
}

/// The number a heap stamps on its pages as their owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HeapNo(pub u32);

impl std::fmt::Display for HeapNo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// One version of a chain, as [`super::walk`] finds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Link {
    pub tid: Tid,
    /// The heap holding the version.
    pub heap: HeapNo,
    pub header: Header,
    pub flags: Flags,
}

/// What [`Heap::prune`] did to a page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pruned {
    /// Slots emptied: versions freed, and dead slots and redirects no longer needed.
    pub freed: usize,
    /// Versions whose aborted deleter was cleared.
    pub cleared: usize,
    /// The page's all-visible bit is set.
    pub all_visible: bool,
    /// The page held nothing and left the heap.
    pub removed: bool,
}

/// Returns the fate of a version from its writers, for pruning.
pub type Judge<'a> = dyn FnMut(&Writers) -> Result<Fate, PageError> + 'a;

/// What converting a page to the newest format made of it.
#[derive(Debug, Default)]
struct Converted {
    page: Vec<u8>,
    /// Pointers the converted versions hold that they did not before.
    gained: Vec<Pointer>,
    /// Pointers the versions held before and no longer do.
    dropped: Vec<Pointer>,
}

/// A page's conversion to the newest format, made from `source`, a copy of the page.
struct Preconverted {
    source: PageCopy,
    done: Converted,
}

/// A page of a heap that its directory names at the position the page records.
#[derive(Clone, Copy, Debug)]
struct Listed<'g>(HeapPage<'g>);

impl<'g> std::ops::Deref for Listed<'g> {
    type Target = HeapPage<'g>;

    fn deref(&self) -> &HeapPage<'g> {
        &self.0
    }
}

/// What [`Heap::try_put`] made of a page.
enum Put {
    Done(Tid, Converted),
    /// The page cannot take the version or is no longer a page of the heap.
    Full,
    /// The page changed since its conversion was made.
    Stale,
}

/// The row versions of one constructor relation.
pub struct Heap {
    pub(super) id: u32,
    page_size: usize,
    pub(super) space: Space,
    large: LargeStore,
    formats: Vec<Format>,
    /// The page that took the last insert, 0 for none.
    hint: AtomicU64,
}

impl std::fmt::Debug for Heap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Heap")
            .field("id", &self.id)
            .field("root", &self.space.root())
            .field("formats", &self.formats.len())
            .finish_non_exhaustive()
    }
}

/// Returns the arrangement of the pages of a heap of `layout`, or `TooLarge` when even its
/// smallest version does not fit a page of `page_size` bytes.
fn arrangement_of(layout: &Layout, page_size: usize) -> Result<Arrangement, Error> {
    if !layout.is_fixed() {
        let len = HEADER_LEN + layout.least_body_len();
        let limit = slotted::max_version(page_size);
        if len > limit {
            return Err(Error::TooLarge { len, limit });
        }
        return Ok(Arrangement::Slotted);
    }
    let width = HEADER_LEN + align8(layout.fixed_width());
    if fixed::capacity(page_size, width) == 0 {
        return Err(Error::TooLarge {
            len: width,
            limit: fixed::max_width(page_size),
        });
    }
    Ok(Arrangement::Fixed { width })
}

/// Refuses with `TooLarge` a heap of fields `row` whose smallest version does not fit a page
/// of `page_size` bytes.
pub fn fits(page_size: usize, row: Vec<FieldType>) -> Result<(), Error> {
    arrangement_of(&Layout::new(row), page_size).map(|_| ())
}

fn version_bytes(header: Header, body: &[u8]) -> Vec<u8> {
    let mut out = vec![0; HEADER_LEN];
    header.write(&mut out);
    out.extend_from_slice(body);
    out
}

pub(super) fn tid(pid: Pid, slot: usize) -> Tid {
    Tid::new(pid.0, slot as u16).expect("48-bit page id")
}

impl Heap {
    /// Create an empty heap with id `id` whose rows have fields `row`, out-of-line values in
    /// `large`.
    pub fn create<R: Runtime>(
        pager: &Pager<R>,
        id: u32,
        row: Vec<FieldType>,
        large: LargeStore,
    ) -> Result<Heap, Error> {
        let fanout = max_fanout(pager.page_size());
        Heap::create_with_fanout(pager, id, row, large, fanout)
    }

    /// Create an empty heap whose directory pages hold `fanout` entries each.
    pub fn create_with_fanout<R: Runtime>(
        pager: &Pager<R>,
        id: u32,
        row: Vec<FieldType>,
        large: LargeStore,
        fanout: u16,
    ) -> Result<Heap, Error> {
        let layout = Layout::new(row);
        let arrangement = arrangement_of(&layout, pager.page_size())?;
        let vm = Vm::create(pager, id)?;
        let space = Space::create(pager, id, fanout, Some(vm))?;
        Ok(Heap::assemble(pager, id, space, large, layout, arrangement))
    }

    /// Open the heap `id` whose directory root is `root`, with its first row format.
    pub fn open<R: Runtime>(
        pager: &Pager<R>,
        id: u32,
        root: Pid,
        row: Vec<FieldType>,
        large: LargeStore,
    ) -> Result<Heap, Error> {
        let layout = Layout::new(row);
        let arrangement = arrangement_of(&layout, pager.page_size())?;
        Ok(Heap::assemble(
            pager,
            id,
            Space::open(id, root),
            large,
            layout,
            arrangement,
        ))
    }

    fn assemble<R: Runtime>(
        pager: &Pager<R>,
        id: u32,
        space: Space,
        large: LargeStore,
        layout: Layout,
        arrangement: Arrangement,
    ) -> Heap {
        Heap {
            id,
            page_size: pager.page_size(),
            space,
            large,
            formats: vec![Format {
                layout,
                arrangement,
                upgrade: None,
            }],
            hint: AtomicU64::new(0),
        }
    }

    pub fn id(&self) -> HeapNo {
        HeapNo(self.id)
    }

    /// Returns the root page of the heap's directory, which never changes.
    pub fn root(&self) -> Pid {
        self.space.root()
    }

    pub fn large(&self) -> LargeStore {
        self.large
    }

    /// Returns the newest row format version.
    pub fn newest(&self) -> u16 {
        self.formats.len() as u16
    }

    fn format(&self, version: FormatNo) -> &Format {
        &self.formats[usize::from(version.0) - 1]
    }

    /// Returns the row format of `page`, or `None` when its format byte names no format of the
    /// heap.
    pub(super) fn format_of(&self, page: HeapPage<'_>) -> Option<FormatNo> {
        let n = storage::format(&page);
        (1..=self.newest()).contains(&n).then_some(FormatNo(n))
    }

    /// Returns the newest row format.
    fn newest_format(&self) -> FormatNo {
        FormatNo(self.newest())
    }

    /// Release `held`, then record page `pid`, a page of the heap whose format byte names no
    /// format of it, as damaged.
    ///
    /// Precondition: `held` is every latch the caller holds on `pid`.
    pub(super) fn refuse_format<R: Runtime>(
        pager: &Pager<R>,
        pid: Pid,
        held: impl Sized,
    ) -> PageError {
        drop(held);
        pager.damage(pid, Damage::Checksum)
    }

    fn current(&self) -> &Format {
        self.formats.last().unwrap()
    }

    /// Returns every page of the heap, in directory order.
    pub fn pages<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pid>, PageError> {
        self.space.pages(pager)
    }

    /// Returns the row format, flags and bytes of the version at `tid`, if a version of this heap
    /// is there. Refuse its page when its format byte names no format of the heap.
    fn raw<R: Runtime>(
        &self,
        pager: &Pager<R>,
        tid: Tid,
    ) -> Result<Option<(FormatNo, Flags, Vec<u8>)>, PageError> {
        let pid = Pid(tid.page());
        let guard = pager.read(pid)?;
        let Some(page) = self.page(&guard) else {
            return Ok(None);
        };
        let Some(format) = self.format_of(page) else {
            return Err(Heap::refuse_format(pager, pid, guard));
        };
        let Slot::Normal { flags, at } = page.get(usize::from(tid.slot())) else {
            return Ok(None);
        };
        Ok(Some((format, flags, page[at].to_vec())))
    }

    pub(super) fn upgrade(&self, from: FormatNo, mut row: Vec<Value>) -> Vec<Value> {
        for f in &self.formats[usize::from(from.0)..] {
            row = (f.upgrade.as_ref().unwrap())(row);
        }
        row
    }

    /// Returns the row of `version` in format `format`, and the pointers of its out-of-line
    /// values.
    pub(super) fn decode<R: Runtime>(
        &self,
        pager: &Pager<R>,
        format: FormatNo,
        version: &[u8],
    ) -> Result<(Vec<Value>, Vec<Pointer>), PageError> {
        let layout = &self.format(format).layout;
        let (fixed, stored) = layout.split(&version[HEADER_LEN..]);
        let mut vars = Vec::with_capacity(stored.len());
        let mut pointers = Vec::new();
        for s in stored {
            match s {
                Stored::Inline(b) => vars.push(b),
                Stored::Out(p) => {
                    pointers.push(p);
                    vars.push(self.large.read(pager, p)?);
                }
            }
        }
        Ok((layout.decode(fixed, &vars), pointers))
    }

    fn pointers(&self, format: FormatNo, version: &[u8]) -> Vec<Pointer> {
        let (_, stored) = self.format(format).layout.split(&version[HEADER_LEN..]);
        stored
            .into_iter()
            .filter_map(|s| match s {
                Stored::Out(p) => Some(p),
                Stored::Inline(_) => None,
            })
            .collect()
    }

    /// Returns the version at `tid`, if a version of this heap is there. Its out-of-line values
    /// are read while its page is held under a shared latch.
    pub fn read<R: Runtime>(
        &self,
        pager: &Pager<R>,
        tid: Tid,
    ) -> Result<Option<Version>, PageError> {
        let pid = Pid(tid.page());
        let guard = pager.read_formatted(pid)?;
        let Some(page) = self.page(&guard) else {
            return Ok(None);
        };
        let Some(format) = self.format_of(page) else {
            return Err(Heap::refuse_format(pager, pid, guard));
        };
        let Slot::Normal { flags, at } = page.get(usize::from(tid.slot())) else {
            return Ok(None);
        };
        let bytes = &page[at];
        let (row, _) = self.decode(pager, format, bytes)?;
        Ok(Some(Version {
            tid,
            header: Header::read(bytes),
            flags,
            row: self.upgrade(format, row),
        }))
    }

    /// Call `f` on every version of the heap, in directory order and then slot order. Each page's
    /// versions are decoded while it is held under a shared latch, and `f` sees them after the
    /// latch is released.
    pub fn scan<R: Runtime>(
        &self,
        pager: &Pager<R>,
        f: impl FnMut(Version),
    ) -> Result<(), PageError> {
        self.scan_pages(pager, &self.pages(pager)?, f)
    }

    /// Call `f` on every version on `pages`, a range of what [`Heap::pages`] returned, in their
    /// order and then slot order, as [`Heap::scan`] does. A page no longer of the heap is skipped,
    /// and refused while the directory still names it.
    pub fn scan_pages<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pages: &[Pid],
        mut f: impl FnMut(Version),
    ) -> Result<(), PageError> {
        for &pid in pages {
            let found;
            {
                let guard = pager.read_formatted(pid)?;
                let Some(page) = self.page(&guard) else {
                    drop(guard);
                    self.space.refuse_misnamed(pager, pid)?;
                    continue;
                };
                let Some(format) = self.format_of(page) else {
                    return Err(Heap::refuse_format(pager, pid, guard));
                };
                found = self.versions_on(pager, pid, page, format)?;
            }
            found.into_iter().for_each(&mut f);
        }
        Ok(())
    }

    /// Returns `page` as a heap page of this heap.
    pub(super) fn page<'g>(&self, page: &'g impl Owned) -> Option<HeapPage<'g>> {
        HeapPage::any(page).filter(|p| p.owner() == self.id)
    }

    /// Returns `page` as a heap page of this heap to change.
    fn page_mut<'g>(&self, page: &'g mut PageCopy) -> Option<HeapPageMut<'g>> {
        HeapPageMut::any(page).filter(|p| p.view().owner() == self.id)
    }

    /// Returns the versions on `page`, page `pid` of the heap in row format `format`, in slot
    /// order.
    pub(super) fn versions_on<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        page: HeapPage<'_>,
        format: FormatNo,
    ) -> Result<Vec<Version>, PageError> {
        let mut found = Vec::new();
        for slot in 0..page.slot_count() {
            let Slot::Normal { flags, at } = page.get(slot) else {
                continue;
            };
            let bytes = &page[at];
            let (row, _) = self.decode(pager, format, bytes)?;
            found.push(Version {
                tid: tid(pid, slot),
                header: Header::read(bytes),
                flags,
                row: self.upgrade(format, row),
            });
        }
        Ok(found)
    }

    /// Returns the body of `row` in the newest format, its large values out of line and written
    /// by `xid`, and the pointers it holds.
    ///
    /// A variable-width field equal to a value one of `reuse` points at keeps that pointer. Others
    /// move out of line, largest first, while the version exceeds a quarter of a page.
    fn body<R: Runtime>(
        &self,
        pager: &Pager<R>,
        xid: Xid,
        row: &[Value],
        reuse: &[Pointer],
    ) -> Result<(Vec<u8>, Vec<Pointer>), Error> {
        let f = self.current();
        let (fixed_part, vars) = f.layout.encode(row);
        let mut known: Vec<Option<Vec<u8>>> = vec![None; reuse.len()];
        let mut stored = Vec::with_capacity(vars.len());
        for bytes in vars {
            let mut s = None;
            for (k, p) in reuse.iter().enumerate() {
                if p.len != bytes.len() as u64 {
                    continue;
                }
                if known[k].is_none() {
                    known[k] = Some(self.large.read(pager, *p)?);
                }
                if known[k].as_deref() == Some(&bytes[..]) {
                    s = Some(Stored::Out(*p));
                    break;
                }
            }
            stored.push(s.unwrap_or(Stored::Inline(bytes)));
        }
        self.check_lengths(stored.iter().map(Stored::len))?;
        while HEADER_LEN + f.layout.body_len(&stored) > self.page_size / 4 {
            let largest = stored
                .iter()
                .enumerate()
                .filter_map(|(j, s)| match s {
                    Stored::Inline(b) if b.len() > POINTER_LEN => Some((b.len(), j)),
                    _ => None,
                })
                .max();
            let Some((_, j)) = largest else { break };
            let Stored::Inline(bytes) = &stored[j] else {
                unreachable!()
            };
            stored[j] = Stored::Out(self.large.write(pager, xid, bytes)?);
        }
        let held = stored
            .iter()
            .filter_map(|s| match s {
                Stored::Out(p) => Some(*p),
                Stored::Inline(_) => None,
            })
            .collect();
        Ok((f.layout.assemble(&fixed_part, &stored), held))
    }

    pub fn check_row(&self, row: &[Value]) -> Result<(), Error> {
        let (_, vars) = self.current().layout.encode(row);
        self.check_lengths(vars.iter().map(Vec::len))
    }

    fn check_lengths(&self, lengths: impl Iterator<Item = usize>) -> Result<(), Error> {
        let limit = match self.current().arrangement {
            Arrangement::Fixed { width } => width,
            Arrangement::Slotted => slotted::max_version(self.page_size),
        };
        let base = HEADER_LEN + self.current().layout.least_body_len();
        let len = base + lengths.map(|len| len.min(POINTER_LEN)).sum::<usize>();
        if len > limit {
            return Err(Error::TooLarge { len, limit });
        }
        Ok(())
    }

    /// Returns `page`, in row format `from`, with every version on it in the newest format, at the
    /// same slots, with the pointers the versions gained and dropped, or `None` if they would not
    /// fit. Values that move out of line are written by `xid`.
    fn convert<R: Runtime>(
        &self,
        pager: &Pager<R>,
        xid: Xid,
        page: HeapPage<'_>,
        from: FormatNo,
    ) -> Result<Option<Converted>, Error> {
        let target = match page.arrangement() {
            Arrangement::Slotted => Arrangement::Slotted,
            Arrangement::Fixed { .. } => self.current().arrangement,
        };
        let mut copy = page.reformat(self.newest(), target);
        let mut out = HeapPageMut::any(&mut copy).expect("a page formatted empty is a heap page");
        let mut done = Converted::default();
        for slot in 0..page.slot_count() {
            let content = page.get(slot);
            let version = match &content {
                Slot::Normal { at, .. } => {
                    let old = &page[at.clone()];
                    let (row, pointers) = self.decode(pager, from, old)?;
                    let row = self.upgrade(from, row);
                    match self.body(pager, xid, &row, &pointers) {
                        Ok((body, held)) => {
                            done.gained
                                .extend(held.iter().filter(|p| !pointers.contains(p)));
                            done.dropped
                                .extend(pointers.iter().filter(|p| !held.contains(p)));
                            version_bytes(Header::read(old), &body)
                        }
                        Err(Error::TooLarge { .. }) => return Ok(None),
                        Err(e) => return Err(e),
                    }
                }
                _ => Vec::new(),
            };
            if !out.put(slot, &content, &version) {
                return Ok(None);
            }
        }
        done.page = copy.into_bytes();
        Ok(Some(done))
    }

    /// Returns `page`, read under a latch as page `pid`, as a page of this heap that its directory
    /// names, the directory read while the caller holds latches up to `floor`.
    fn listed<'g, R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        page: &'g impl Owned,
        floor: Option<Rank>,
    ) -> Result<Option<Listed<'g>>, PageError> {
        let Some(page) = self.page(page) else {
            return Ok(None);
        };
        let named = self.space.names(pager, page.position(), pid, floor)?;
        Ok(named.then_some(Listed(page)))
    }

    /// Returns page `pid` converted to the newest format, the values that move out of line written
    /// by `xid` while no page is latched, or `None` if the page is in the newest format, is not a
    /// page of this heap, or its versions would not fit.
    fn preconvert<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        xid: Xid,
    ) -> Result<Option<Preconverted>, Error> {
        let source = PageCopy::of(&pager.read(pid)?);
        let Some(page) = self.page(&source) else {
            return Ok(None);
        };
        let Some(from) = self.format_of(page) else {
            return Err(Heap::refuse_format(pager, pid, ()).into());
        };
        if from == self.newest_format() {
            return Ok(None);
        }
        let done = self.convert(pager, xid, page, from)?;
        Ok(done.map(|done| Preconverted { source, done }))
    }

    /// Insert `version` on page `pid`, which `stage` holds, taking `pre` as the page in the newest
    /// format when it is in an older one.
    fn try_put<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        pid: Pid,
        version: &[u8],
        flags: Flags,
        pre: Option<Preconverted>,
    ) -> Result<Put, Error> {
        let mut page = stage.read(pager, Rank::Heap(pid))?;
        if self.listed(pager, pid, &page, stage.top())?.is_none() {
            return Ok(Put::Full);
        }
        let mut converted = Converted::default();
        if storage::format(&page) != self.newest() {
            match pre {
                Some(p) if p.source == page => {
                    page.replace(p.done.page.clone());
                    converted = p.done;
                }
                Some(_) => return Ok(Put::Stale),
                None => return Ok(Put::Full),
            }
        }
        let mut heap_page = HeapPageMut::any(&mut page).expect("a listed page is a heap page");
        let Some(slot) = heap_page.insert(version, flags) else {
            return Ok(Put::Full);
        };
        stage.put(pager, Rank::Heap(pid), page.into_bytes())?;
        Ok(Put::Done(tid(pid, slot), converted))
    }

    /// Make `pid` the page the heap's next insert tries first.
    pub(super) fn hint_at(&self, pid: Pid) {
        self.hint.store(pid.0, Ordering::Relaxed);
    }

    /// Stop the heap's next insert trying `pid` first.
    pub(super) fn forget_hint(&self, pid: Pid) {
        self.hint
            .compare_exchange(pid.0, 0, Ordering::Relaxed, Ordering::Relaxed)
            .ok();
    }

    /// Correct the directory entry of `pid`, which could not take a version: an old-format page
    /// gets category 0, any other page of the heap its actual category.
    fn refused<R: Runtime>(&self, pager: &Pager<R>, pid: Pid) -> Result<(), PageError> {
        self.forget_hint(pid);
        drop(pager.read_formatted(pid)?);
        let mut write = HeapWrite::new(pager);
        write.latch(&[(self, pid)])?;
        let page = write.stage().read(pager, Rank::Heap(pid))?;
        if self
            .listed(pager, pid, &page, write.stage().top())?
            .is_none()
        {
            return Ok(());
        }
        write.commit_refused(self, pid)
    }

    /// Returns a page that may take a version needing `need` bytes: the last page that took an
    /// insert, a page whose entry promises the room, or a new page.
    fn candidate<R: Runtime>(&self, pager: &Pager<R>, need: usize) -> Result<Pid, Error> {
        let hint = self.hint.load(Ordering::Relaxed);
        if hint != 0 {
            return Ok(Pid(hint));
        }
        let f = self.current();
        let need = need_category(f.arrangement.need(need), self.page_size);
        let (arrangement, newest) = (f.arrangement, self.newest());
        let room = self.space.room(pager, need, arrangement, newest)?;
        room.ok_or(Error::NoDirectory)
    }

    /// Replace the writers of the version at `at`, whose page `stage` holds, by `writers` applied
    /// to them; set its `next` and replace its successor flags by `successor`.
    fn supersede<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        at: impl Into<WriteTarget>,
        writers: impl FnOnce(Writers) -> Writers,
        next: Option<Tid>,
        successor: Flags,
    ) -> Result<(), Error> {
        let target = at.into();
        let at = target.tid;
        let copy = stage.page(pager, Rank::Heap(Pid(at.page())))?;
        let slot = usize::from(at.slot());
        let held = self.page_mut(copy).and_then(|page| {
            let content = page.view().get(slot);
            match content {
                Slot::Normal { flags, at } => Some((page, flags, at)),
                _ => None,
            }
        });
        let Some((mut page, flags, range)) = held else {
            target.check(None)?;
            panic!("{at:?} holds no version of heap {}", self.id);
        };
        let mut header = Header::read(&page.view()[range.clone()]);
        target.check(Some(header))?;
        header.writers = writers(header.writers);
        header.next = next;
        header.write(page.version_mut(range));
        let flags = flags
            .without(Flags::MOVED)
            .without(Flags::HOT_UPDATED)
            .with(successor);
        page.set_flags(slot, flags);
        Ok(())
    }

    /// Returns the TID, header and flags of every version of the heap, in directory order and
    /// then slot order. A page that is not a page of this heap with sound slots is skipped.
    pub fn headers<R: Runtime>(
        &self,
        pager: &Pager<R>,
    ) -> Result<Vec<(Tid, Header, Flags)>, PageError> {
        let mut out = Vec::new();
        for pid in self.space.pages(pager)? {
            let guard = pager.read(pid)?;
            let Some(page) = self.page(&guard).filter(|p| p.check().is_empty()) else {
                continue;
            };
            for slot in 0..page.slot_count() {
                if let Slot::Normal { flags, at } = page.get(slot) {
                    out.push((tid(pid, slot), Header::read(&page[at]), flags));
                }
            }
        }
        Ok(out)
    }

    /// Returns the pointer of every out-of-line value a version of the heap holds, or `None` if
    /// a page the directory names is not a page of this heap with sound slots. Refuse a page
    /// whose format byte names no format of the heap.
    pub fn out_of_line<R: Runtime>(
        &self,
        pager: &Pager<R>,
    ) -> Result<Option<Vec<Pointer>>, PageError> {
        let mut out = Vec::new();
        for pid in self.space.pages(pager)? {
            let guard = pager.read(pid)?;
            let Some(page) = self.page(&guard).filter(|p| p.check().is_empty()) else {
                return Ok(None);
            };
            let Some(format) = self.format_of(page) else {
                return Err(Heap::refuse_format(pager, pid, guard));
            };
            for slot in 0..page.slot_count() {
                if let Slot::Normal { at, .. } = page.get(slot) {
                    out.extend(self.pointers(format, &page[at]));
                }
            }
        }
        Ok(Some(out))
    }

    pub(super) fn vm<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
    ) -> Result<Vm, PageError> {
        match self.space.vm(stage, pager)? {
            Some(root) => Ok(Vm::open(root, self.id, self.space.root())),
            None => Err(self.space.refuse_root(stage, pager)),
        }
    }

    /// Returns whether the visibility map has the bit of page `pid` set: every version on it is
    /// visible to every snapshot.
    pub fn all_visible<R: Runtime>(&self, pager: &Pager<R>, pid: Pid) -> Result<bool, PageError> {
        let mut stage = Stage::new();
        self.vm(&mut stage, pager)?.get(&mut stage, pager, pid)
    }

    /// Returns whether `tid` names a slot of this heap holding a version, a dead mark or a
    /// redirect.
    pub fn holds<R: Runtime>(&self, pager: &Pager<R>, tid: Tid) -> Result<bool, PageError> {
        let guard = pager.read(Pid(tid.page()))?;
        Ok(self
            .page(&guard)
            .is_some_and(|p| p.get(usize::from(tid.slot())) != Slot::Unused))
    }

    /// Returns the slots of page `pid` whose index entries must be deleted before pruning can free
    /// them: those named by index entries whose chain on the page holds no version `judge` keeps.
    /// A page no longer of this heap has none, refused while the directory still names it.
    pub fn dead_slots<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        judge: impl FnMut(&Writers) -> Result<Fate, PageError>,
    ) -> Result<Vec<u16>, PageError> {
        let copy = PageCopy::of(&pager.read_formatted(pid)?);
        if self.page(&copy).is_none() {
            self.space.refuse_misnamed(pager, pid)?;
        }
        let Some(page) = self.listed(pager, pid, &copy, None)? else {
            return Ok(Vec::new());
        };
        let fates = prune::judge(*page, judge)?;
        let plan = prune::plan(*page, pid.0, &fates, &|_| false);
        Ok(plan.dead.into_iter().map(|s| s as u16).collect())
    }

    /// Prune page `pid` under its exclusive latch, judging each version by `judge`, in one update.
    ///
    /// Versions `judge` removes are freed when no index entry names them and left as dead slots
    /// otherwise; the slots in `cleaned`, whose index entries are gone, are freed. A chain of
    /// heap-only versions whose leading versions are removed keeps its first slot as a redirect.
    /// Versions whose deleter aborted have their deleter, `next` and successor flags cleared.
    /// When `mark` holds, the page's all-visible bit is set if every version left is judged
    /// all-visible and no dead slot is left. The directory entry takes the page's free space when
    /// the page is in the newest format or pruning freed space on it. A page left empty leaves the
    /// directory and is freed in the same pager update. Each out-of-line value a freed version
    /// pointed at is counted one version fewer afterwards. A page no longer of this heap is left
    /// alone.
    ///
    /// Panics if a slot in `cleaned` still leads to a version `judge` keeps.
    pub fn prune<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        judge: impl FnMut(&Writers) -> Result<Fate, PageError>,
        cleaned: &[u16],
        mark: bool,
    ) -> Result<Pruned, PageError> {
        if mark {
            self.vm(&mut Stage::new(), pager)?.extend(pager, pid)?;
        }
        let mut write = HeapWrite::new(pager);
        write.latch(&[(self, pid)])?;
        let copy = write.stage().read(pager, Rank::Heap(pid))?;
        let Some(page) = self.listed(pager, pid, &copy, write.stage().top())? else {
            return Ok(Pruned::default());
        };
        let Some(format) = self.format_of(*page) else {
            return Err(Heap::refuse_format(pager, pid, write));
        };
        let fates = prune::judge(*page, judge)?;
        let plan = prune::plan(*page, pid.0, &fates, &|s| cleaned.contains(&(s as u16)));
        let mut out = page.to_copy();
        prune::apply(
            &mut HeapPageMut::any(&mut out).expect("a copy of a heap page is a heap page"),
            &plan,
        );
        let mut released = Vec::new();
        for (s, _) in &plan.release {
            if let Slot::Normal { at, .. } = page.get(*s) {
                released.extend(self.pointers(format, &page[at]));
            }
        }
        let visible = if mark {
            plan.all_visible
        } else {
            page.all_visible() && out == copy
        };
        let removed = write.commit_pruned(self, pid, out, visible, &released)?;
        Ok(Pruned {
            freed: plan
                .release
                .iter()
                .filter(|(_, c)| *c == Slot::Unused)
                .count(),
            cleared: plan.clear.len(),
            all_visible: visible && !removed,
            removed,
        })
    }

    /// Insert `row` as a version written by `xid`. Returns its TID.
    ///
    /// Panics if `row` is not a row of the newest format.
    pub fn insert<R: Runtime>(
        &self,
        pager: &Pager<R>,
        xid: Xid,
        row: &[Value],
    ) -> Result<Tid, Error> {
        self.insert_pruning(pager, xid, row, None)
    }

    /// Insert `row` as [`Heap::insert`] does, pruning by `judge` a page found without room
    /// before looking further.
    pub fn insert_pruning<R: Runtime>(
        &self,
        pager: &Pager<R>,
        xid: Xid,
        row: &[Value],
        judge: Option<&mut Judge<'_>>,
    ) -> Result<Tid, Error> {
        self.put_version(pager, xid, row, judge)
    }

    /// Insert a copy of `v`, a version of `from` read at `v.tid`, with no successor, and flag the
    /// original kept, both in one update. Returns the copy's TID, or `None`, copying nothing, when
    /// `from` no longer holds that version.
    ///
    /// Panic if `from` is this heap or `v.row` is not a row of the newest format.
    pub fn copy_version<R: Runtime>(
        &self,
        pager: &Pager<R>,
        from: &Heap,
        v: &Version,
    ) -> Result<Option<Tid>, Error> {
        assert_ne!(self.id, from.id, "a version is copied to another heap");
        let mut header = v.header;
        header.next = None;
        let writer = header.writers.copy_writer();
        let (body, held) = self.body(pager, writer, &v.row, &[])?;
        let version = version_bytes(header, &body);
        let o = Pid(v.tid.page());
        loop {
            let n = self.candidate(pager, version.len())?;
            let pre = self.preconvert(pager, n, writer)?;
            let mut write = HeapWrite::new(pager);
            write.latch(&[(from, o), (self, n)])?;
            if !from.mark_kept(write.stage(), pager, v)? {
                return Ok(None);
            }
            match self.try_put(write.stage(), pager, n, &version, Flags::NONE, pre)? {
                Put::Done(t, c) => {
                    let retain = [&held[..], &c.gained].concat();
                    write.commit(Some(Took::Found(self, n)), &retain, &c.dropped)?;
                    return Ok(Some(t));
                }
                Put::Stale => continue,
                Put::Full => {}
            }
            drop(write);
            self.refused(pager, n)?;
        }
    }

    /// Flag the version `v` names kept on its page in `stage`. Returns false, changing nothing,
    /// when the page no longer holds a version there written as `v.header` says.
    fn mark_kept<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        v: &Version,
    ) -> Result<bool, PageError> {
        let copy = stage.page(pager, Rank::Heap(Pid(v.tid.page())))?;
        let Some(mut page) = self.page_mut(copy) else {
            return Ok(false);
        };
        let slot = usize::from(v.tid.slot());
        let Slot::Normal { flags, at } = page.view().get(slot) else {
            return Ok(false);
        };
        let h = Header::read(&page.view()[at]);
        if h.writers != v.header.writers {
            return Ok(false);
        }
        page.set_flags(slot, flags.with(Flags::KEPT));
        Ok(true)
    }

    fn put_version<R: Runtime>(
        &self,
        pager: &Pager<R>,
        xid: Xid,
        row: &[Value],
        mut judge: Option<&mut Judge<'_>>,
    ) -> Result<Tid, Error> {
        let (body, held) = self.body(pager, xid, row, &[])?;
        let version = version_bytes(Header::new(xid), &body);
        loop {
            let pid = self.candidate(pager, version.len())?;
            let pre = self.preconvert(pager, pid, xid)?;
            let mut write = HeapWrite::new(pager);
            write.latch(&[(self, pid)])?;
            match self.try_put(write.stage(), pager, pid, &version, Flags::NONE, pre)? {
                Put::Done(t, c) => {
                    let retain = [&held[..], &c.gained].concat();
                    write.commit(Some(Took::Found(self, pid)), &retain, &c.dropped)?;
                    return Ok(t);
                }
                Put::Stale => continue,
                Put::Full => {}
            }
            drop(write);
            if self.prune_full(pager, pid, judge.as_deref_mut())? {
                continue;
            }
            self.refused(pager, pid)?;
        }
    }

    /// Prune `pid` by `judge`, if there is one and the page is in the newest format. Returns
    /// whether pruning freed a slot.
    fn prune_full<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        judge: Option<&mut Judge<'_>>,
    ) -> Result<bool, PageError> {
        let Some(judge) = judge else {
            return Ok(false);
        };
        let copy = PageCopy::of(&pager.read(pid)?);
        let listed = self.listed(pager, pid, &copy, None)?;
        if listed.is_none_or(|page| storage::format(&page) != self.newest()) {
            return Ok(false);
        }
        let done = self.prune(pager, pid, judge, &[], false)?;
        Ok(done.freed > 0 && !done.removed)
    }

    /// Update as [`Heap::update`] does, pruning by `judge` a page found without room before
    /// looking further.
    pub fn update_pruning<R: Runtime>(
        &self,
        pager: &Pager<R>,
        at: impl Into<WriteTarget>,
        xid: Xid,
        row: &[Value],
        indexed: &[usize],
        mut judge: Option<&mut Judge<'_>>,
    ) -> Result<Updated, Error> {
        let target = at.into();
        let at = target.tid;
        let raw = self.raw(pager, at)?;
        let Some((format, _, old)) = raw else {
            target.check(None)?;
            panic!("{at:?} holds no version of heap {}", self.id);
        };
        target.check(Some(Header::read(&old)))?;
        let (old_row, pointers) = self.decode(pager, format, &old)?;
        let old_row = self.upgrade(format, old_row);
        let keys_kept = indexed.iter().all(|&i| old_row[i] == row[i]);
        let (body, held) = self.body(pager, xid, row, &pointers)?;
        let version = version_bytes(Header::new(xid), &body);
        let o = Pid(at.page());
        let mut tries = 0;
        while keys_kept && tries < 2 {
            let pre = self.preconvert(pager, o, xid)?;
            let mut write = HeapWrite::new(pager);
            write.latch(&[(self, o)])?;
            match self.try_put(write.stage(), pager, o, &version, Flags::HEAP_ONLY, pre)? {
                Put::Done(t, c) => {
                    self.supersede(
                        write.stage(),
                        pager,
                        target,
                        |w| w.superseded_by(xid),
                        Some(t),
                        Flags::HOT_UPDATED,
                    )?;
                    let retain = [&held[..], &c.gained].concat();
                    write.commit(Some(Took::HeapOnly(self, o)), &retain, &c.dropped)?;
                    return Ok(Updated {
                        tid: t,
                        heap_only: true,
                    });
                }
                Put::Stale => continue,
                Put::Full => tries += 1,
            }
            drop(write);
            if !self.prune_full(pager, o, judge.as_deref_mut())? {
                break;
            }
        }
        loop {
            let n = self.candidate(pager, version.len())?;
            let pre = self.preconvert(pager, n, xid)?;
            let mut write = HeapWrite::new(pager);
            write.latch(&[(self, o), (self, n)])?;
            match self.try_put(write.stage(), pager, n, &version, Flags::NONE, pre)? {
                Put::Done(t, c) => {
                    self.supersede(
                        write.stage(),
                        pager,
                        target,
                        |w| w.superseded_by(xid),
                        Some(t),
                        Flags::NONE,
                    )?;
                    let retain = [&held[..], &c.gained].concat();
                    write.commit(Some(Took::Found(self, n)), &retain, &c.dropped)?;
                    return Ok(Updated {
                        tid: t,
                        heap_only: false,
                    });
                }
                Put::Stale => continue,
                Put::Full => {}
            }
            drop(write);
            if n != o && self.prune_full(pager, n, judge.as_deref_mut())? {
                continue;
            }
            self.refused(pager, n)?;
        }
    }

    /// Mark the version at `at` deleted by `xid`.
    pub fn delete<R: Runtime>(
        &self,
        pager: &Pager<R>,
        at: impl Into<WriteTarget>,
        xid: Xid,
    ) -> Result<(), Error> {
        let target = at.into();
        let mut write = HeapWrite::new(pager);
        write.latch(&[(self, Pid(target.tid.page()))])?;
        self.supersede(
            write.stage(),
            pager,
            target,
            |w| w.superseded_by(xid),
            None,
            Flags::NONE,
        )?;
        write.commit(None, &[], &[])?;
        Ok(())
    }

    /// Clear the deleter of the version at `at`, its `next` and its successor flags.
    ///
    /// Preconditions: the version's deleter is the caller's running transaction, and the
    /// successor it wrote, if any, is deleted by that transaction too.
    pub fn restore<R: Runtime>(
        &self,
        pager: &Pager<R>,
        at: impl Into<WriteTarget>,
    ) -> Result<(), Error> {
        let target = at.into();
        let mut write = HeapWrite::new(pager);
        write.latch(&[(self, Pid(target.tid.page()))])?;
        self.supersede(
            write.stage(),
            pager,
            target,
            Writers::cleared,
            None,
            Flags::NONE,
        )?;
        write.commit(None, &[], &[])?;
        Ok(())
    }

    /// Move the row whose version is at `at` into heap `to` as `row`, written by `xid`. Returns
    /// the new version's TID.
    ///
    /// The new version has the retagged-in flag; the old one gets `xid` as its deleter, the moved
    /// flag and `next` naming the new one. Out-of-line values `row` shares with the old version
    /// keep their pointers.
    ///
    /// Panics if `to` is this heap or belongs to another sum.
    pub fn retag<R: Runtime>(
        &self,
        pager: &Pager<R>,
        at: impl Into<WriteTarget>,
        xid: Xid,
        to: &Heap,
        row: &[Value],
    ) -> Result<Tid, Error> {
        assert_ne!(self.id, to.id, "a retag moves a row to another heap");
        assert_eq!(self.large, to.large, "a retag stays within one sum");
        let target = at.into();
        let at = target.tid;
        let raw = self.raw(pager, at)?;
        let Some((format, _, old)) = raw else {
            target.check(None)?;
            panic!("{at:?} holds no version of heap {}", self.id);
        };
        target.check(Some(Header::read(&old)))?;
        let pointers = self.pointers(format, &old);
        let (body, held) = to.body(pager, xid, row, &pointers)?;
        let version = version_bytes(Header::new(xid), &body);
        let o = Pid(at.page());
        loop {
            let n = to.candidate(pager, version.len())?;
            let pre = to.preconvert(pager, n, xid)?;
            let mut write = HeapWrite::new(pager);
            write.latch(&[(self, o), (to, n)])?;
            match to.try_put(write.stage(), pager, n, &version, Flags::RETAGGED_IN, pre)? {
                Put::Done(t, c) => {
                    self.supersede(
                        write.stage(),
                        pager,
                        target,
                        |w| w.superseded_by(xid),
                        Some(t),
                        Flags::MOVED,
                    )?;
                    let retain = [&held[..], &c.gained].concat();
                    write.commit(Some(Took::Found(to, n)), &retain, &c.dropped)?;
                    return Ok(t);
                }
                Put::Stale => continue,
                Put::Full => {}
            }
            drop(write);
            to.refused(pager, n)?;
        }
    }

    /// Returns each directory page with an entry that promises less than its child holds, with
    /// what it promises too little. A crash may leave them; [`Heap::settle_directory`] mends them.
    pub fn unsettled<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<(Pid, String)>, PageError> {
        Ok(self.space.check(pager)?.unsettled)
    }

    /// Returns each page of the heap, its directory or its visibility map found wrong, with what
    /// is wrong with it, leaving out [`Heap::unsettled`] directory entries.
    pub fn check<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<(Pid, String)>, PageError> {
        let check = self.space.check(pager)?;
        let mut problems = check.problems;
        let Some(vm) = self.space.vm(&mut Stage::new(), pager)? else {
            if !self.space.is_placeholder() {
                problems.push((self.root(), "names no visibility map".into()));
            }
            return Ok(problems);
        };
        let vm = Vm::open(vm, self.id, self.space.root());
        let mut marked: BTreeSet<Pid> = vm.marked(pager)?.into_iter().collect();
        for (position, leaf) in check.leaves.into_iter().enumerate() {
            let Some((pid, cat)) = leaf else {
                continue;
            };
            let copy = PageCopy::of(&pager.read(pid)?);
            let Some(kinded) = Kinded::of(&copy).filter(|k| k.owner() == self.id) else {
                problems.push((pid, format!("is not a page of heap {}", self.id)));
                continue;
            };
            let layout = kinded.check();
            if !layout.is_empty() {
                problems.extend(layout.into_iter().map(|what| (pid, what)));
                continue;
            }
            let page = kinded.framed().expect("a page whose slots check is framed");
            if page.position() as usize != position {
                problems.push((pid, format!("records position {}", page.position())));
            }
            let actual = page_category(page, self.page_size);
            let current = storage::format(&page) == self.newest();
            if cat > actual || current && cat != actual {
                problems.push((
                    pid,
                    format!("has category {cat} in the directory, {actual} on the page"),
                ));
            }
            if marked.remove(&pid) && !page.all_visible() {
                problems.push((
                    pid,
                    "is all-visible in the visibility map but not on the page".into(),
                ));
            }
            for slot in 0..page.slot_count() {
                match page.get(slot) {
                    Slot::Redirect(to)
                        if !matches!(page.get(usize::from(to)), Slot::Normal { .. }) =>
                    {
                        problems.push((pid, format!("slot {slot} redirects to an empty slot")));
                    }
                    Slot::Dead if page.all_visible() => {
                        problems.push((pid, format!("is all-visible and has dead slot {slot}")));
                    }
                    _ => {}
                }
            }
            let links = chain::link_faults(page, pid.0);
            problems.extend(links.into_iter().map(|what| (pid, what)));
        }
        for pid in marked {
            problems.push((
                pid,
                format!(
                    "is marked all-visible but is not a page of heap {}",
                    self.id
                ),
            ));
        }
        Ok(problems)
    }

    /// Returns every page the heap owns: its directory's, its own and its visibility map's.
    pub fn owned_pages<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pid>, PageError> {
        self.space.owned(pager)
    }

    /// Returns the pages of [`Heap::owned_pages`]. A directory or map page out of shape stops the
    /// walk.
    pub fn held_pages<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pid>, Stopped> {
        self.space.held(pager)
    }
}
