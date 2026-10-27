use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::key::{Tid, Value};
use crate::runtime::Runtime;
use crate::storage::{self, PageError, Pager, Pid};

use super::dir::{Above, Space, category, max_fanout, need_category};
use super::large::LargeStore;
use super::page::{self, Arrangement, Slot, align8};
use super::prune::{self, Fate};
use super::row::{FieldType, Layout, POINTER_LEN, Pointer, Stored};
use super::stage::Stage;
use super::version::{Flags, HEADER_LEN, Header};
use super::vm::Vm;
use super::{fixed, slotted};

/// Converts a row of the previous format into a row of this one.
pub type Upgrade = Box<dyn Fn(Vec<Value>) -> Vec<Value> + Send + Sync>;

struct Format {
    layout: Layout,
    arrangement: Arrangement,
    upgrade: Option<Upgrade>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Page(PageError),
    /// A version of `len` bytes is longer than the `limit` a page holds, even with its large
    /// values out of line.
    TooLarge {
        len: usize,
        limit: usize,
    },
}

impl From<PageError> for Error {
    fn from(e: PageError) -> Error {
        Error::Page(e)
    }
}

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

/// One version of a chain, as [`walk`] finds it.
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

/// Decides the fate of a version from its header, for pruning.
pub type Judge<'a, R> = dyn FnMut(&Pager<R>, &Header) -> Result<Fate, PageError> + 'a;

/// What converting a page to the newest format made of it.
#[derive(Debug, Default)]
struct Converted {
    page: Vec<u8>,
    /// Pointers the converted versions hold that they did not before.
    gained: Vec<Pointer>,
    /// Pointers the versions held before and no longer do.
    dropped: Vec<Pointer>,
}

/// Returns `a` and `b` in the order their latches are taken, once each.
fn latch_order(a: Pid, b: Pid) -> Vec<Pid> {
    let mut out = vec![a.min(b), a.max(b)];
    out.dedup();
    out
}

/// Finish a change whose heap pages `stage` holds: clear the all-visible bit of each page of
/// `marks` that has it, the map's bit in an update of its own written first, and set the
/// directory category of the page of `dir` to its free space. Returns the directory entries to
/// settle once `stage` is written.
fn finish<'p, R: Runtime>(
    stage: &mut Stage<'p, R>,
    pager: &'p Pager<R>,
    dir: Option<(&Heap, Pid)>,
    marks: &[(&Heap, Pid)],
) -> Result<Above, PageError> {
    for &(h, pid) in marks {
        let page = stage.page(pager, pid)?;
        if !page::all_visible(page) {
            continue;
        }
        page::set_all_visible(page, false);
        h.vm(stage, pager)?.clear(pager, pid)?;
    }
    let Some((h, pid)) = dir else {
        return Ok(Above::default());
    };
    let page = stage.read(pager, pid)?;
    let cat = category(page::free(&page), h.page_size);
    h.space
        .set_category(stage, pager, pid, page::position(&page), cat)
}

/// A page's conversion to the newest format, made from `source`, a copy of the page.
struct Preconverted {
    source: Vec<u8>,
    done: Converted,
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
    id: u32,
    page_size: usize,
    space: Space,
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

fn tid(pid: Pid, slot: usize) -> Tid {
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
        let space = Space::create(pager, id, fanout, Some(vm.root()))?;
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

    fn format(&self, version: u16) -> &Format {
        &self.formats[usize::from(version) - 1]
    }

    fn current(&self) -> &Format {
        self.formats.last().unwrap()
    }

    /// Returns every page of the heap, in directory order.
    pub fn pages<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pid>, PageError> {
        self.space.pages(pager)
    }

    /// Returns the row format, flags and bytes of the version at `tid`, if a version is there.
    fn raw<R: Runtime>(
        &self,
        pager: &Pager<R>,
        tid: Tid,
    ) -> Result<Option<(u16, Flags, Vec<u8>)>, PageError> {
        let page = pager.read(Pid(tid.page()))?;
        assert_eq!(
            self.id,
            storage::owner(&page),
            "{tid:?} is not in heap {}",
            self.id
        );
        Ok(match page::get(&page, usize::from(tid.slot())) {
            Slot::Normal { flags, at } => Some((storage::format(&page), flags, page[at].to_vec())),
            _ => None,
        })
    }

    fn upgrade(&self, from: u16, mut row: Vec<Value>) -> Vec<Value> {
        for f in &self.formats[usize::from(from)..] {
            row = (f.upgrade.as_ref().unwrap())(row);
        }
        row
    }

    /// Returns the row of `version` in format `format`, and the pointers of its out-of-line
    /// values.
    fn decode<R: Runtime>(
        &self,
        pager: &Pager<R>,
        format: u16,
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

    fn pointers(&self, format: u16, version: &[u8]) -> Vec<Pointer> {
        let (_, stored) = self.format(format).layout.split(&version[HEADER_LEN..]);
        stored
            .into_iter()
            .filter_map(|s| match s {
                Stored::Out(p) => Some(p),
                Stored::Inline(_) => None,
            })
            .collect()
    }

    /// Returns the pointers the version at `tid` holds, if a version of this heap is there.
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

    /// Returns the version at `tid`, if a version is there. Its out-of-line values are read
    /// while its page is held under a shared latch.
    ///
    /// Panics if `tid` names a page of another heap.
    pub fn read<R: Runtime>(
        &self,
        pager: &Pager<R>,
        tid: Tid,
    ) -> Result<Option<Version>, PageError> {
        let page = pager.read(Pid(tid.page()))?;
        assert_eq!(
            self.id,
            storage::owner(&page),
            "{tid:?} is not in heap {}",
            self.id
        );
        let Slot::Normal { flags, at } = page::get(&page, usize::from(tid.slot())) else {
            return Ok(None);
        };
        let format = storage::format(&page);
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
    /// order and then slot order, as [`Heap::scan`] does. A page no longer of the heap is skipped.
    pub fn scan_pages<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pages: &[Pid],
        mut f: impl FnMut(Version),
    ) -> Result<(), PageError> {
        for &pid in pages {
            let mut found = Vec::new();
            {
                let page = pager.read(pid)?;
                if !page::is_heap(&page) || storage::owner(&page) != self.id {
                    continue;
                }
                let format = storage::format(&page);
                for slot in 0..page::slot_count(&page) {
                    let Slot::Normal { flags, at } = page::get(&page, slot) else {
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
            }
            found.into_iter().for_each(&mut f);
        }
        Ok(())
    }

    /// Returns the body of `row` in the newest format, its large values out of line and written
    /// by `xid`, and the pointers it holds.
    ///
    /// A variable-width field equal to a value one of `reuse` points at keeps that pointer. Others
    /// move out of line, largest first, while the version exceeds a quarter of a page.
    fn body<R: Runtime>(
        &self,
        pager: &Pager<R>,
        xid: u64,
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
        let max = match f.arrangement {
            Arrangement::Fixed { width } => width,
            Arrangement::Slotted => slotted::max_version(self.page_size),
        };
        let least = HEADER_LEN
            + f.layout.least_body_len()
            + stored
                .iter()
                .map(|s| s.len().min(POINTER_LEN))
                .sum::<usize>();
        if least > max {
            return Err(Error::TooLarge {
                len: least,
                limit: max,
            });
        }
        let limit = self.page_size / 4;
        while HEADER_LEN + f.layout.body_len(&stored) > limit {
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

    /// Returns `page` with every version on it in the newest format, at the same slots, with the
    /// pointers the versions gained and dropped, or `None` if they would not fit. Values that move
    /// out of line are written by `xid`.
    fn convert<R: Runtime>(
        &self,
        pager: &Pager<R>,
        xid: u64,
        page: &[u8],
    ) -> Result<Option<Converted>, Error> {
        let from = storage::format(page);
        let target = match page::arrangement(page) {
            Arrangement::Slotted => Arrangement::Slotted,
            Arrangement::Fixed { .. } => self.current().arrangement,
        };
        let mut out = page.to_vec();
        let header = storage::header_edit(
            self.newest(),
            target.kind(),
            storage::owner(page),
            storage::flags(page),
        );
        out[storage::FIRST_EDITABLE..storage::HEADER_LEN].copy_from_slice(&header);
        target.init(&mut out, page::position(page));
        let mut done = Converted::default();
        for slot in 0..page::slot_count(page) {
            let content = page::get(page, slot);
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
            if !page::put(&mut out, slot, &content, &version) {
                return Ok(None);
            }
        }
        done.page = out;
        Ok(Some(done))
    }

    /// Returns whether `page`, read under a latch as page `pid`, is a page of this heap that its
    /// directory names.
    fn ours<R: Runtime>(&self, pager: &Pager<R>, pid: Pid, page: &[u8]) -> Result<bool, PageError> {
        if !page::is_heap(page) || storage::owner(page) != self.id {
            return Ok(false);
        }
        self.space.names(pager, page::position(page), pid)
    }

    /// Returns page `pid` converted to the newest format, the values that move out of line written
    /// by `xid` while no page is latched, or `None` if the page is in the newest format, is not a
    /// page of this heap, or its versions would not fit.
    fn preconvert<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        xid: u64,
    ) -> Result<Option<Preconverted>, Error> {
        let source = pager.read(pid)?.to_vec();
        if !page::is_heap(&source)
            || storage::owner(&source) != self.id
            || storage::format(&source) == self.newest()
        {
            return Ok(None);
        }
        let done = self.convert(pager, xid, &source)?;
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
        let mut page = stage.read(pager, pid)?;
        if !self.ours(pager, pid, &page)? {
            return Ok(Put::Full);
        }
        let mut converted = Converted::default();
        if storage::format(&page) != self.newest() {
            match pre {
                Some(p) if p.source == page => {
                    page = p.done.page.clone();
                    converted = p.done;
                }
                Some(_) => return Ok(Put::Stale),
                None => return Ok(Put::Full),
            }
        }
        let Some(slot) = page::insert(&mut page, version, flags) else {
            return Ok(Put::Full);
        };
        stage.put(pager, pid, page)?;
        Ok(Put::Done(tid(pid, slot), converted))
    }

    /// Count one more version pointing at each of `held` and `gained`, in updates of their own.
    fn retain_all<R: Runtime>(
        &self,
        pager: &Pager<R>,
        held: &[Pointer],
        gained: &[Pointer],
    ) -> Result<(), PageError> {
        self.large.retain_apart(pager, &[held, gained].concat())
    }

    /// Correct the directory entry of `pid`, which could not take a version: an old-format page
    /// gets category 0, any other page of the heap its actual category.
    fn refused<R: Runtime>(&self, pager: &Pager<R>, pid: Pid) -> Result<(), PageError> {
        if self.hint.load(Ordering::Relaxed) == pid.0 {
            self.hint.store(0, Ordering::Relaxed);
        }
        let mut stage = Stage::new();
        let page = stage.page(pager, pid)?.clone();
        if !self.ours(pager, pid, &page)? {
            return Ok(());
        }
        let cat = if storage::format(&page) == self.newest() {
            category(page::free(&page), self.page_size)
        } else {
            0
        };
        let above = self
            .space
            .set_category(&mut stage, pager, pid, page::position(&page), cat)?;
        stage.commit(pager)?;
        above.settle(pager)
    }

    /// Returns a page that may take a version needing `need` bytes: the last page that took an
    /// insert, a page whose entry promises the room, or a new page.
    fn candidate<R: Runtime>(&self, pager: &Pager<R>, need: usize) -> Result<Pid, PageError> {
        let hint = self.hint.load(Ordering::Relaxed);
        if hint != 0 {
            return Ok(Pid(hint));
        }
        let f = self.current();
        let need = need_category(f.arrangement.need(need), self.page_size);
        let (arrangement, newest) = (f.arrangement, self.newest());
        match self
            .space
            .find(&Stage::new(), pager, need, arrangement, newest)?
        {
            Some(pid) => Ok(pid),
            None => self.space.append(pager, arrangement, newest),
        }
    }

    /// Set `xmax` and `next` of the version at `at`, whose page `stage` holds, and replace its
    /// successor flags by `successor`.
    fn supersede<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        at: Tid,
        xid: u64,
        next: Option<Tid>,
        successor: Flags,
    ) -> Result<(), PageError> {
        let page = stage.page(pager, Pid(at.page()))?;
        let slot = usize::from(at.slot());
        let Slot::Normal { flags, at: range } = page::get(page, slot) else {
            panic!("{at:?} holds no version");
        };
        let mut header = Header::read(&page[range.clone()]);
        header.xmax = xid;
        header.next = next;
        header.write(&mut page[range]);
        let flags = flags
            .without(Flags::MOVED)
            .without(Flags::HOT_UPDATED)
            .with(successor);
        page::set_flags(page, slot, flags);
        Ok(())
    }

    /// Returns the TID, header and flags of every version of the heap, in directory order and
    /// then slot order.
    pub fn headers<R: Runtime>(
        &self,
        pager: &Pager<R>,
    ) -> Result<Vec<(Tid, Header, Flags)>, PageError> {
        let mut out = Vec::new();
        for pid in self.space.pages(pager)? {
            let page = pager.read(pid)?;
            if !page::is_heap(&page) || !page::check(&page).is_empty() {
                continue;
            }
            for slot in 0..page::slot_count(&page) {
                if let Slot::Normal { flags, at } = page::get(&page, slot) {
                    out.push((tid(pid, slot), Header::read(&page[at]), flags));
                }
            }
        }
        Ok(out)
    }

    /// Returns the pointer of every out-of-line value a version of the heap holds, or `None` if
    /// a page of the heap has broken slots.
    pub fn out_of_line<R: Runtime>(
        &self,
        pager: &Pager<R>,
    ) -> Result<Option<Vec<Pointer>>, PageError> {
        let mut out = Vec::new();
        for pid in self.space.pages(pager)? {
            let page = pager.read(pid)?;
            if !page::is_heap(&page) || !page::check(&page).is_empty() {
                return Ok(None);
            }
            let format = storage::format(&page);
            for slot in 0..page::slot_count(&page) {
                if let Slot::Normal { at, .. } = page::get(&page, slot) {
                    out.extend(self.pointers(format, &page[at]));
                }
            }
        }
        Ok(Some(out))
    }

    fn vm<'p, R: Runtime>(
        &self,
        stage: &Stage<'p, R>,
        pager: &'p Pager<R>,
    ) -> Result<Vm, PageError> {
        let root = self.space.vm(stage, pager)?;
        Ok(Vm::open(
            root.expect("a heap's directory names its visibility map"),
            self.id,
        ))
    }

    /// Returns whether the visibility map has the bit of page `pid` set: every version on it is
    /// visible to every snapshot.
    pub fn all_visible<R: Runtime>(&self, pager: &Pager<R>, pid: Pid) -> Result<bool, PageError> {
        self.vm(&Stage::new(), pager)?.get(pager, pid)
    }

    /// Returns whether `tid` names a slot of this heap holding a version, a dead mark or a
    /// redirect.
    pub fn holds<R: Runtime>(&self, pager: &Pager<R>, tid: Tid) -> Result<bool, PageError> {
        let page = pager.read(Pid(tid.page()))?;
        Ok(page::is_heap(&page)
            && storage::owner(&page) == self.id
            && page::get(&page, usize::from(tid.slot())) != Slot::Unused)
    }

    /// Returns the slots of page `pid` whose index entries must be deleted before pruning can
    /// free them: those named by index entries whose chain on the page holds no version `judge`
    /// keeps. A page no longer of this heap has none.
    pub fn dead_slots<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        mut judge: impl FnMut(&Pager<R>, &Header) -> Result<Fate, PageError>,
    ) -> Result<Vec<u16>, PageError> {
        let page = pager.read(pid)?.to_vec();
        if !self.ours(pager, pid, &page)? {
            return Ok(Vec::new());
        }
        let fates = prune::judge(&page, |h| judge(pager, h))?;
        let plan = prune::plan(&page, pid.0, &fates, &|_| false);
        Ok(plan.dead.into_iter().map(|s| s as u16).collect())
    }

    /// Prune page `pid` under its exclusive latch, judging each version by `judge`, in one update.
    ///
    /// Versions `judge` removes are freed when no index entry names them and left as dead slots
    /// otherwise; the slots in `cleaned`, whose index entries are gone, are freed. A chain of
    /// heap-only versions whose leading versions are removed keeps its first slot as a redirect.
    /// Versions whose deleter aborted have `xmax`, `next` and successor flags cleared. When `mark`
    /// holds, the page's all-visible bit is set if every version left is judged all-visible and
    /// no dead slot is left. The directory entry takes the page's free space when the page is in
    /// the newest format or pruning freed space on it. A page left empty leaves the directory and
    /// is freed in the same pager update. Each out-of-line value a freed version pointed at is
    /// counted one version fewer afterwards. A page no longer of this heap is left alone.
    ///
    /// Panics if a slot in `cleaned` still leads to a version `judge` keeps.
    pub fn prune<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        mut judge: impl FnMut(&Pager<R>, &Header) -> Result<Fate, PageError>,
        cleaned: &[u16],
        mark: bool,
    ) -> Result<Pruned, PageError> {
        if mark {
            self.vm(&Stage::new(), pager)?.extend(pager, pid)?;
        }
        let mut stage = Stage::new();
        let page = stage.page(pager, pid)?.clone();
        if !self.ours(pager, pid, &page)? {
            return Ok(Pruned::default());
        }
        let fates = prune::judge(&page, |h| judge(pager, h))?;
        let plan = prune::plan(&page, pid.0, &fates, &|s| cleaned.contains(&(s as u16)));
        let mut out = page.clone();
        prune::apply(&mut out, &plan);
        let format = storage::format(&page);
        let mut released = Vec::new();
        for (s, _) in &plan.release {
            if let Slot::Normal { at, .. } = page::get(&page, *s) {
                released.extend(self.pointers(format, &page[at]));
            }
        }
        let mut done = Pruned {
            freed: plan
                .release
                .iter()
                .filter(|(_, c)| *c == Slot::Unused)
                .count(),
            cleared: plan.clear.len(),
            ..Pruned::default()
        };
        let flagged = page::all_visible(&page);
        let position = page::position(&page);
        let vm = self.vm(&stage, pager)?;
        let bit = vm.get(pager, pid)?;
        if page::is_empty(&out) {
            page::set_all_visible(&mut out, false);
            if bit {
                vm.clear(pager, pid)?;
            }
            stage.put(pager, pid, out)?;
            let above = self.space.vacate(&mut stage, pager, pid, position)?;
            let mut gone = pager.reserve();
            gone.free(pid);
            stage.commit_with(pager, gone)?;
            above.settle(pager)?;
            self.hint
                .compare_exchange(pid.0, 0, Ordering::Relaxed, Ordering::Relaxed)
                .ok();
            done.removed = true;
        } else {
            let changed = out != page;
            let visible = if mark {
                plan.all_visible
            } else {
                flagged && !changed
            };
            page::set_all_visible(&mut out, visible);
            if bit && !visible {
                vm.clear(pager, pid)?;
            }
            let roomier = page::free(&out) > page::free(&page);
            let current = storage::format(&out) == self.newest();
            let cat = category(page::free(&out), self.page_size);
            stage.put(pager, pid, out)?;
            let above = if roomier || current {
                self.space
                    .set_category(&mut stage, pager, pid, position, cat)?
            } else {
                Above::default()
            };
            if visible && !bit {
                vm.put(&mut stage, pager, pid, true)?;
            }
            stage.commit(pager)?;
            above.settle(pager)?;
            done.all_visible = visible;
        }
        for p in released {
            self.large.release(pager, p)?;
        }
        Ok(done)
    }

    /// Insert `row` as a version written by `xid`. Returns its TID.
    ///
    /// Panics if `row` is not a row of the newest format.
    pub fn insert<R: Runtime>(
        &self,
        pager: &Pager<R>,
        xid: u64,
        row: &[Value],
    ) -> Result<Tid, Error> {
        self.insert_pruning(pager, xid, row, None)
    }

    /// Insert `row` as [`Heap::insert`] does, pruning by `judge` a page found without room
    /// before looking further.
    pub fn insert_pruning<R: Runtime>(
        &self,
        pager: &Pager<R>,
        xid: u64,
        row: &[Value],
        judge: Option<&mut Judge<'_, R>>,
    ) -> Result<Tid, Error> {
        self.put_version(pager, Header::new(xid), row, judge)
    }

    /// Insert a copy of `v`, a version of `from` read at `v.tid`, with no successor, and flag the
    /// original kept, both in one update. Returns the copy's TID, or `None`, copying nothing, when
    /// `from` no longer holds that version.
    ///
    /// Panics if `from` is this heap or `v.row` is not a row of the newest format.
    pub fn copy_version<R: Runtime>(
        &self,
        pager: &Pager<R>,
        from: &Heap,
        v: &Version,
    ) -> Result<Option<Tid>, Error> {
        assert_ne!(self.id, from.id, "a version is copied to another heap");
        let header = Header {
            next: None,
            ..v.header
        };
        let (body, held) = self.body(pager, header.xmin, &v.row, &[])?;
        let version = version_bytes(header, &body);
        let o = Pid(v.tid.page());
        loop {
            let n = self.candidate(pager, version.len())?;
            let pre = self.preconvert(pager, n, header.xmin)?;
            let mut stage = Stage::new();
            for p in latch_order(o, n) {
                stage.page(pager, p)?;
            }
            if !from.mark_kept(&mut stage, pager, v)? {
                return Ok(None);
            }
            match self.try_put(&mut stage, pager, n, &version, Flags::NONE, pre)? {
                Put::Done(t, c) => {
                    self.retain_all(pager, &held, &c.gained)?;
                    let above = finish(&mut stage, pager, Some((self, n)), &[(self, n)])?;
                    stage.commit(pager)?;
                    above.settle(pager)?;
                    self.hint.store(n.0, Ordering::Relaxed);
                    self.release_all(pager, &c.dropped)?;
                    return Ok(Some(t));
                }
                Put::Stale => continue,
                Put::Full => {}
            }
            drop(stage);
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
        let page = stage.page(pager, Pid(v.tid.page()))?;
        if !page::is_heap(page) || storage::owner(page) != self.id {
            return Ok(false);
        }
        let slot = usize::from(v.tid.slot());
        let Slot::Normal { flags, at } = page::get(page, slot) else {
            return Ok(false);
        };
        let h = Header::read(&page[at]);
        if (h.xmin, h.xmax) != (v.header.xmin, v.header.xmax) {
            return Ok(false);
        }
        page::set_flags(page, slot, flags.with(Flags::KEPT));
        Ok(true)
    }

    fn put_version<R: Runtime>(
        &self,
        pager: &Pager<R>,
        header: Header,
        row: &[Value],
        mut judge: Option<&mut Judge<'_, R>>,
    ) -> Result<Tid, Error> {
        let xid = header.xmin;
        let (body, held) = self.body(pager, xid, row, &[])?;
        let version = version_bytes(header, &body);
        loop {
            let pid = self.candidate(pager, version.len())?;
            let pre = self.preconvert(pager, pid, xid)?;
            let mut stage = Stage::new();
            stage.page(pager, pid)?;
            match self.try_put(&mut stage, pager, pid, &version, Flags::NONE, pre)? {
                Put::Done(t, c) => {
                    self.retain_all(pager, &held, &c.gained)?;
                    let above = finish(&mut stage, pager, Some((self, pid)), &[(self, pid)])?;
                    stage.commit(pager)?;
                    above.settle(pager)?;
                    self.hint.store(pid.0, Ordering::Relaxed);
                    self.release_all(pager, &c.dropped)?;
                    return Ok(t);
                }
                Put::Stale => continue,
                Put::Full => {}
            }
            drop(stage);
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
        judge: Option<&mut Judge<'_, R>>,
    ) -> Result<bool, PageError> {
        let Some(judge) = judge else {
            return Ok(false);
        };
        let page = pager.read(pid)?.to_vec();
        if !self.ours(pager, pid, &page)? || storage::format(&page) != self.newest() {
            return Ok(false);
        }
        let done = self.prune(pager, pid, |p, h| judge(p, h), &[], false)?;
        Ok(done.freed > 0 && !done.removed)
    }

    fn release_all<R: Runtime>(
        &self,
        pager: &Pager<R>,
        dropped: &[Pointer],
    ) -> Result<(), PageError> {
        for &p in dropped {
            self.large.release(pager, p)?;
        }
        Ok(())
    }

    /// Write `row` as the successor `xid` makes of the version at `at`, and point that version at
    /// it.
    ///
    /// The successor is heap-only when it fits on the old version's page and no field listed in
    /// `indexed`, the key fields of every index over this heap's constructor, changes.
    pub fn update<R: Runtime>(
        &self,
        pager: &Pager<R>,
        at: Tid,
        xid: u64,
        row: &[Value],
        indexed: &[usize],
    ) -> Result<Updated, Error> {
        self.update_pruning(pager, at, xid, row, indexed, None)
    }

    /// Update as [`Heap::update`] does, pruning by `judge` a page found without room before
    /// looking further.
    pub fn update_pruning<R: Runtime>(
        &self,
        pager: &Pager<R>,
        at: Tid,
        xid: u64,
        row: &[Value],
        indexed: &[usize],
        mut judge: Option<&mut Judge<'_, R>>,
    ) -> Result<Updated, Error> {
        let (format, _, old) = self
            .raw(pager, at)?
            .expect("update of a slot with no version");
        let (old_row, pointers) = self.decode(pager, format, &old)?;
        let old_row = self.upgrade(format, old_row);
        let keys_kept = indexed.iter().all(|&i| old_row[i] == row[i]);
        let (body, held) = self.body(pager, xid, row, &pointers)?;
        let version = version_bytes(Header::new(xid), &body);
        let o = Pid(at.page());
        let mut tries = 0;
        while keys_kept && tries < 2 {
            let pre = self.preconvert(pager, o, xid)?;
            let mut stage = Stage::new();
            stage.page(pager, o)?;
            match self.try_put(&mut stage, pager, o, &version, Flags::HEAP_ONLY, pre)? {
                Put::Done(t, c) => {
                    self.supersede(&mut stage, pager, at, xid, Some(t), Flags::HOT_UPDATED)?;
                    self.retain_all(pager, &held, &c.gained)?;
                    let above = finish(&mut stage, pager, Some((self, o)), &[(self, o)])?;
                    stage.commit(pager)?;
                    above.settle(pager)?;
                    self.release_all(pager, &c.dropped)?;
                    return Ok(Updated {
                        tid: t,
                        heap_only: true,
                    });
                }
                Put::Stale => continue,
                Put::Full => tries += 1,
            }
            drop(stage);
            if !self.prune_full(pager, o, judge.as_deref_mut())? {
                break;
            }
        }
        loop {
            let n = self.candidate(pager, version.len())?;
            let pre = self.preconvert(pager, n, xid)?;
            let mut stage = Stage::new();
            for p in latch_order(o, n) {
                stage.page(pager, p)?;
            }
            match self.try_put(&mut stage, pager, n, &version, Flags::NONE, pre)? {
                Put::Done(t, c) => {
                    self.supersede(&mut stage, pager, at, xid, Some(t), Flags::NONE)?;
                    self.retain_all(pager, &held, &c.gained)?;
                    let marks = [(self, o), (self, n)];
                    let above = finish(&mut stage, pager, Some((self, n)), &marks)?;
                    stage.commit(pager)?;
                    above.settle(pager)?;
                    self.hint.store(n.0, Ordering::Relaxed);
                    self.release_all(pager, &c.dropped)?;
                    return Ok(Updated {
                        tid: t,
                        heap_only: false,
                    });
                }
                Put::Stale => continue,
                Put::Full => {}
            }
            drop(stage);
            if n != o && self.prune_full(pager, n, judge.as_deref_mut())? {
                continue;
            }
            self.refused(pager, n)?;
        }
    }

    /// Mark the version at `at` deleted by `xid`.
    pub fn delete<R: Runtime>(&self, pager: &Pager<R>, at: Tid, xid: u64) -> Result<(), Error> {
        let o = Pid(at.page());
        let mut stage = Stage::new();
        self.supersede(&mut stage, pager, at, xid, None, Flags::NONE)?;
        let above = finish(&mut stage, pager, None, &[(self, o)])?;
        stage.commit(pager)?;
        above.settle(pager)?;
        Ok(())
    }

    /// Clear the deleter of the version at `at`: its `xmax`, `next` and successor flags.
    ///
    /// Preconditions: the version's deleter is the caller's running transaction, and the
    /// successor it wrote, if any, is deleted by that transaction too.
    pub fn restore<R: Runtime>(&self, pager: &Pager<R>, at: Tid) -> Result<(), Error> {
        let mut stage = Stage::new();
        self.supersede(&mut stage, pager, at, 0, None, Flags::NONE)?;
        stage.commit(pager)?;
        Ok(())
    }

    /// Move the row whose version is at `at` into heap `to` as `row`, written by `xid`. Returns
    /// the new version's TID.
    ///
    /// The new version has the retagged-in flag; the old one gets `xmax = xid`, the moved flag
    /// and `next` naming the new one. Out-of-line values `row` shares with the old version keep
    /// their pointers.
    ///
    /// Panics if `to` is this heap or belongs to another sum.
    pub fn retag<R: Runtime>(
        &self,
        pager: &Pager<R>,
        at: Tid,
        xid: u64,
        to: &Heap,
        row: &[Value],
    ) -> Result<Tid, Error> {
        assert_ne!(self.id, to.id, "a retag moves a row to another heap");
        assert_eq!(self.large, to.large, "a retag stays within one sum");
        let (format, _, old) = self
            .raw(pager, at)?
            .expect("retag of a slot with no version");
        let pointers = self.pointers(format, &old);
        let (body, held) = to.body(pager, xid, row, &pointers)?;
        let version = version_bytes(Header::new(xid), &body);
        let o = Pid(at.page());
        loop {
            let n = to.candidate(pager, version.len())?;
            let pre = to.preconvert(pager, n, xid)?;
            let mut stage = Stage::new();
            for p in latch_order(o, n) {
                stage.page(pager, p)?;
            }
            match to.try_put(&mut stage, pager, n, &version, Flags::RETAGGED_IN, pre)? {
                Put::Done(t, c) => {
                    self.supersede(&mut stage, pager, at, xid, Some(t), Flags::MOVED)?;
                    self.retain_all(pager, &held, &c.gained)?;
                    let marks = [(self, o), (to, n)];
                    let above = finish(&mut stage, pager, Some((to, n)), &marks)?;
                    stage.commit(pager)?;
                    above.settle(pager)?;
                    to.hint.store(n.0, Ordering::Relaxed);
                    to.release_all(pager, &c.dropped)?;
                    return Ok(t);
                }
                Put::Stale => continue,
                Put::Full => {}
            }
            drop(stage);
            to.refused(pager, n)?;
        }
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

    /// Returns each directory page with an entry that promises less than its child holds, with
    /// what it promises too little. A crash may leave them; [`Heap::settle_directory`] mends them.
    pub fn unsettled<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<(Pid, String)>, PageError> {
        Ok(self.space.check(pager)?.unsettled)
    }

    /// Set every inner directory entry to what its child holds, each in an update of its own.
    pub fn settle_directory<R: Runtime>(&self, pager: &Pager<R>) -> Result<(), PageError> {
        self.space.settle(pager)
    }

    /// Returns each page of the heap, its directory or its visibility map found wrong, with what
    /// is wrong with it, leaving out [`Heap::unsettled`] directory entries.
    pub fn check<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<(Pid, String)>, PageError> {
        let check = self.space.check(pager)?;
        let mut problems = check.problems;
        let vm = self.vm(&Stage::new(), pager)?;
        let mut marked: BTreeSet<Pid> = vm.marked(pager)?.into_iter().collect();
        for (position, leaf) in check.leaves.into_iter().enumerate() {
            let Some((pid, cat)) = leaf else {
                continue;
            };
            let page = pager.read(pid)?.to_vec();
            if !page::is_heap(&page) || storage::owner(&page) != self.id {
                problems.push((pid, format!("is not a page of heap {}", self.id)));
                continue;
            }
            let layout = page::check(&page);
            if !layout.is_empty() {
                problems.extend(layout.into_iter().map(|what| (pid, what)));
                continue;
            }
            if page::position(&page) as usize != position {
                problems.push((pid, format!("records position {}", page::position(&page))));
            }
            let actual = category(page::free(&page), self.page_size);
            let current = storage::format(&page) == self.newest();
            if cat > actual || current && cat != actual {
                problems.push((
                    pid,
                    format!("has category {cat} in the directory, {actual} on the page"),
                ));
            }
            if marked.remove(&pid) && !page::all_visible(&page) {
                problems.push((
                    pid,
                    "is all-visible in the visibility map but not on the page".into(),
                ));
            }
            for slot in 0..page::slot_count(&page) {
                match page::get(&page, slot) {
                    Slot::Redirect(to)
                        if !matches!(page::get(&page, usize::from(to)), Slot::Normal { .. }) =>
                    {
                        problems.push((pid, format!("slot {slot} redirects to an empty slot")));
                    }
                    Slot::Dead if page::all_visible(&page) => {
                        problems.push((pid, format!("is all-visible and has dead slot {slot}")));
                    }
                    Slot::Normal { flags, at } if flags.contains(Flags::HOT_UPDATED) => {
                        let header = Header::read(&page[at]);
                        let successor = header
                            .next
                            .filter(|n| n.page() == pid.0)
                            .map(|n| page::get(&page, usize::from(n.slot())));
                        let linked = match successor {
                            Some(Slot::Normal { flags, at }) => {
                                flags.contains(Flags::HEAP_ONLY)
                                    && Header::read(&page[at]).xmin == header.xmax
                            }
                            _ => false,
                        };
                        if !linked {
                            problems.push((
                                pid,
                                format!(
                                    "slot {slot} is hot-updated but names no heap-only successor"
                                ),
                            ));
                        }
                    }
                    _ => {}
                }
            }
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
    /// Panics if the heap has a page left.
    pub fn drop_empty<R: Runtime>(self, pager: &Pager<R>) -> Result<(), PageError> {
        assert!(self.is_empty(pager)?, "heap {} has pages left", self.id);
        let mut gone = pager.reserve();
        for pid in self.owned_pages(pager)? {
            gone.free(pid);
        }
        pager.update_with(gone, &[], |_| {})?;
        Ok(())
    }

    /// Returns every page the heap owns: its directory's, its own and its visibility map's.
    pub fn owned_pages<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pid>, PageError> {
        let mut out = self.space.nodes(pager)?;
        out.extend(self.space.pages(pager)?);
        out.extend(self.vm(&Stage::new(), pager)?.pages(pager)?);
        Ok(out)
    }
}

/// Returns the header of the version at `tid`, or `None` when its page is not a heap page or
/// its slot holds no version.
pub fn header_at<R: Runtime>(pager: &Pager<R>, tid: Tid) -> Result<Option<Header>, PageError> {
    let page = pager.read(Pid(tid.page()))?;
    if !page::is_heap(&page) || !page::check(&page).is_empty() {
        return Ok(None);
    }
    Ok(match page::get(&page, usize::from(tid.slot())) {
        Slot::Normal { at, .. } => Some(Header::read(&page[at])),
        _ => None,
    })
}

/// Follow the chain from the version at `start`, in heap `heap`, through each `next`, across
/// heaps, and return the first version `stop` accepts. Returns `None` when the chain ends first.
///
/// The chain ends at a slot holding no version, at a page that is not a heap page, at a version
/// whose `xmin` is not the `xmax` of the version before it, and at a version on a page of the
/// wrong heap: the first must be in `heap`, the successor of a moved version in another heap and
/// flagged retagged in, and any other successor in its predecessor's heap. The versions of one
/// stretch of the chain on one page are read from one copy of that page.
pub fn walk<R: Runtime>(
    pager: &Pager<R>,
    heap: HeapNo,
    start: Tid,
    mut stop: impl FnMut(&Link) -> bool,
) -> Result<Option<Link>, PageError> {
    let mut at = start;
    let mut prior: Option<Link> = None;
    let mut copy: Option<(u64, Vec<u8>)> = None;
    loop {
        let page = match copy {
            Some((pid, ref page)) if pid == at.page() => page,
            _ => {
                &copy
                    .insert((at.page(), pager.read(Pid(at.page()))?.to_vec()))
                    .1
            }
        };
        if !page::is_heap(page) {
            return Ok(None);
        }
        match page::get(page, usize::from(at.slot())) {
            Slot::Normal { flags, at: range } => {
                let link = Link {
                    tid: at,
                    heap: HeapNo(storage::owner(page)),
                    header: Header::read(&page[range]),
                    flags,
                };
                if !follows(prior.as_ref(), heap, &link) {
                    return Ok(None);
                }
                if stop(&link) {
                    return Ok(Some(link));
                }
                match link.header.next {
                    Some(next) => at = next,
                    None => return Ok(None),
                }
                prior = Some(link);
            }
            Slot::Redirect(slot) => at = tid(Pid(at.page()), usize::from(slot)),
            Slot::Unused | Slot::Dead => return Ok(None),
        }
    }
}

/// Returns whether `link` can be the version after `prior` in a chain starting in heap `heap`.
fn follows(prior: Option<&Link>, heap: HeapNo, link: &Link) -> bool {
    let Some(p) = prior else {
        return link.heap == heap;
    };
    let placed = if p.flags.contains(Flags::MOVED) {
        link.heap != p.heap && link.flags.contains(Flags::RETAGGED_IN)
    } else {
        link.heap == p.heap
    };
    placed && link.header.xmin == p.header.xmax
}
