use std::collections::BTreeSet;

use crate::key::{Tid, Value};
use crate::runtime::Runtime;
use crate::storage::{self, PageError, Pager, Pid};

use super::dir::{Space, category, max_fanout, need_category};
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
    /// A version of `len` bytes does not fit a page even with its large values out of line.
    TooLarge {
        len: usize,
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

/// One version of a chain, as [`walk`] finds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Link {
    pub tid: Tid,
    /// The heap id of the heap holding the version.
    pub heap: u32,
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

/// The row versions of one constructor relation.
pub struct Heap {
    id: u32,
    page_size: usize,
    space: Space,
    large: LargeStore,
    formats: Vec<Format>,
    /// The page that took the last insert.
    hint: Option<Pid>,
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

fn arrangement_of(layout: &Layout, page_size: usize) -> Result<Arrangement, Error> {
    if !layout.is_fixed() {
        return Ok(Arrangement::Slotted);
    }
    let width = HEADER_LEN + align8(layout.fixed_width());
    if fixed::capacity(page_size, width) == 0 {
        return Err(Error::TooLarge { len: width });
    }
    Ok(Arrangement::Fixed { width })
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
            hint: None,
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

    pub fn id(&self) -> u32 {
        self.id
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
        let page = &pager.read(Pid(tid.page()))?.to_vec();
        assert_eq!(
            self.id,
            storage::owner(page),
            "{tid:?} is not in heap {}",
            self.id
        );
        Ok(match page::get(page, usize::from(tid.slot())) {
            Slot::Normal { flags, at } => Some((storage::format(page), flags, page[at].to_vec())),
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

    /// Returns the version at `tid`, if a version is there.
    ///
    /// Panics if `tid` names a page of another heap.
    pub fn read<R: Runtime>(
        &self,
        pager: &Pager<R>,
        tid: Tid,
    ) -> Result<Option<Version>, PageError> {
        let Some((format, flags, bytes)) = self.raw(pager, tid)? else {
            return Ok(None);
        };
        let (row, _) = self.decode(pager, format, &bytes)?;
        Ok(Some(Version {
            tid,
            header: Header::read(&bytes),
            flags,
            row: self.upgrade(format, row),
        }))
    }

    /// Call `f` on every version of the heap, in directory order and then slot order.
    pub fn scan<R: Runtime>(
        &self,
        pager: &Pager<R>,
        mut f: impl FnMut(Version),
    ) -> Result<(), PageError> {
        for pid in self.space.pages(pager)? {
            let page = pager.read(pid)?.to_vec();
            let format = storage::format(&page);
            for slot in 0..page::slot_count(&page) {
                let Slot::Normal { flags, at } = page::get(&page, slot) else {
                    continue;
                };
                let bytes = &page[at];
                let (row, _) = self.decode(pager, format, bytes)?;
                f(Version {
                    tid: tid(pid, slot),
                    header: Header::read(bytes),
                    flags,
                    row: self.upgrade(format, row),
                });
            }
        }
        Ok(())
    }

    /// Returns the body of `row` in the newest format, its large values out of line.
    ///
    /// A variable-width field equal to a value one of `reuse` points at keeps that pointer. Others
    /// move out of line, largest first, while the version exceeds a quarter of a page.
    fn body<R: Runtime>(
        &self,
        pager: &Pager<R>,
        row: &[Value],
        reuse: &[Pointer],
    ) -> Result<Vec<u8>, Error> {
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
            stored[j] = Stored::Out(self.large.write(pager, bytes)?);
        }
        let len = HEADER_LEN + f.layout.body_len(&stored);
        let max = match f.arrangement {
            Arrangement::Fixed { width } => width,
            Arrangement::Slotted => slotted::max_version(self.page_size),
        };
        if len > max {
            return Err(Error::TooLarge { len });
        }
        Ok(f.layout.assemble(&fixed_part, &stored))
    }

    /// Returns `page` with every version on it in the newest format, at the same slots, or
    /// `None` if they would not fit.
    fn convert<R: Runtime>(&self, pager: &Pager<R>, page: &[u8]) -> Result<Option<Vec<u8>>, Error> {
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
        for slot in 0..page::slot_count(page) {
            let content = page::get(page, slot);
            let version = match &content {
                Slot::Normal { at, .. } => {
                    let old = &page[at.clone()];
                    let (row, pointers) = self.decode(pager, from, old)?;
                    let row = self.upgrade(from, row);
                    match self.body(pager, &row, &pointers) {
                        Ok(body) => version_bytes(Header::read(old), &body),
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
        Ok(Some(out))
    }

    /// Insert `version` on page `pid` in `stage`, converting the page to the newest format first.
    /// Returns its TID, or `None`, staging nothing, if the page cannot take it.
    fn try_put<R: Runtime>(
        &self,
        stage: &mut Stage,
        pager: &Pager<R>,
        pid: Pid,
        version: &[u8],
        flags: Flags,
    ) -> Result<Option<Tid>, Error> {
        let mut page = stage.read(pager, pid)?;
        if storage::format(&page) != self.newest() {
            match self.convert(pager, &page)? {
                Some(converted) => page = converted,
                None => return Ok(None),
            }
        }
        let Some(slot) = page::insert(&mut page, version, flags) else {
            return Ok(None);
        };
        let position = page::position(&page);
        let cat = category(page::free(&page), self.page_size);
        stage.put(pid, page);
        self.unmark(stage, pager, pid)?;
        self.space.set_category(stage, pager, position, cat)?;
        Ok(Some(tid(pid, slot)))
    }

    /// Insert `version` on some page of the heap, in `stage`.
    ///
    /// Precondition: `stage` holds no page of the heap's directory.
    fn place<R: Runtime>(
        &mut self,
        stage: &mut Stage,
        pager: &Pager<R>,
        version: &[u8],
        flags: Flags,
    ) -> Result<Tid, Error> {
        if let Some(hint) = self.hint.take()
            && let Some(t) = self.try_put(stage, pager, hint, version, flags)?
        {
            self.hint = Some(hint);
            return Ok(t);
        }
        let f = self.current();
        let need = need_category(f.arrangement.need(version.len()), self.page_size);
        let (arrangement, newest) = (f.arrangement, self.newest());
        while let Some(pid) = self.space.find(stage, pager, need, arrangement, newest)? {
            if let Some(t) = self.try_put(stage, pager, pid, version, flags)? {
                self.hint = Some(pid);
                return Ok(t);
            }
            let page = &pager.read(pid)?.to_vec();
            assert_ne!(
                newest,
                storage::format(page),
                "{pid:?} has less room than its directory entry says"
            );
            let position = page::position(page);
            let mut full = Stage::new();
            self.space.set_category(&mut full, pager, position, 0)?;
            full.commit(pager)?;
        }
        let pid = self.space.append(pager, arrangement, newest)?;
        self.hint = Some(pid);
        let t = self.try_put(stage, pager, pid, version, flags)?;
        Ok(t.expect("an empty page takes any version that fits a page"))
    }

    /// Set `xmax` and `next` of the version at `at` in `stage`, and replace its successor flags
    /// by `successor`.
    fn supersede<R: Runtime>(
        &self,
        stage: &mut Stage,
        pager: &Pager<R>,
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
        self.unmark(stage, pager, Pid(at.page()))
    }

    fn vm<R: Runtime>(&self, stage: &Stage, pager: &Pager<R>) -> Result<Vm, PageError> {
        let root = self.space.vm(stage, pager)?;
        Ok(Vm::open(
            root.expect("a heap's directory names its visibility map"),
            self.id,
        ))
    }

    /// Clear the all-visible flag of the staged page `pid` and its bit in the visibility map, in
    /// `stage`.
    fn unmark<R: Runtime>(
        &self,
        stage: &mut Stage,
        pager: &Pager<R>,
        pid: Pid,
    ) -> Result<(), PageError> {
        let page = stage.page(pager, pid)?;
        if !page::all_visible(page) {
            return Ok(());
        }
        page::set_all_visible(page, false);
        self.vm(stage, pager)?.put(stage, pager, pid, false)
    }

    /// Returns whether the visibility map has the bit of page `pid` set: every version on it is
    /// visible to every snapshot.
    pub fn all_visible<R: Runtime>(&self, pager: &Pager<R>, pid: Pid) -> Result<bool, PageError> {
        self.vm(&Stage::new(), pager)?.get(pager, pid)
    }

    /// Returns the bytes of heap page `pid`.
    ///
    /// Panics if `pid` is not a page of this heap.
    fn own_page<R: Runtime>(&self, pager: &Pager<R>, pid: Pid) -> Result<Vec<u8>, PageError> {
        let page = pager.read(pid)?.to_vec();
        assert_eq!(
            self.id,
            storage::owner(&page),
            "{pid:?} is not in heap {}",
            self.id
        );
        Ok(page)
    }

    /// Returns whether `tid` names a slot of this heap holding a version, a dead mark or a
    /// redirect.
    pub fn holds<R: Runtime>(&self, pager: &Pager<R>, tid: Tid) -> Result<bool, PageError> {
        let page = &pager.read(Pid(tid.page()))?.to_vec();
        Ok(page::is_heap(page)
            && storage::owner(page) == self.id
            && page::get(page, usize::from(tid.slot())) != Slot::Unused)
    }

    /// Returns the slots of page `pid` whose index entries must be deleted before pruning can
    /// free them: those named by index entries whose chain on the page holds no version `judge`
    /// keeps.
    pub fn dead_slots<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        mut judge: impl FnMut(&Pager<R>, &Header) -> Result<Fate, PageError>,
    ) -> Result<Vec<u16>, PageError> {
        let page = self.own_page(pager, pid)?;
        let fates = prune::judge(&page, |h| judge(pager, h))?;
        let plan = prune::plan(&page, pid.0, &fates, &|_| false);
        Ok(plan.dead.into_iter().map(|s| s as u16).collect())
    }

    /// Prune page `pid`, judging each version by `judge`, in one update.
    ///
    /// Versions `judge` removes are freed when no index entry names them and left as dead slots
    /// otherwise; the slots in `cleaned`, whose index entries are gone, are freed. A chain of
    /// heap-only versions whose leading versions are removed keeps its first slot as a redirect.
    /// Versions whose deleter aborted have `xmax`, `next` and successor flags cleared. When `mark`
    /// holds, the page's all-visible bit is set if every version left is judged all-visible and
    /// no dead slot is left. The directory entry takes the page's free space when the page is in
    /// the newest format or pruning freed space on it. A page left empty leaves the directory and
    /// is freed in the same pager update.
    ///
    /// Panics if `pid` is not a page of this heap, or if a slot in `cleaned` still leads to a
    /// version `judge` keeps.
    pub fn prune<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        pid: Pid,
        mut judge: impl FnMut(&Pager<R>, &Header) -> Result<Fate, PageError>,
        cleaned: &[u16],
        mark: bool,
    ) -> Result<Pruned, PageError> {
        let page = self.own_page(pager, pid)?;
        let fates = prune::judge(&page, |h| judge(pager, h))?;
        let plan = prune::plan(&page, pid.0, &fates, &|s| cleaned.contains(&(s as u16)));
        let mut out = page.clone();
        prune::apply(&mut out, &plan);
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
        let mut stage = Stage::new();
        if page::is_empty(&out) {
            page::set_all_visible(&mut out, false);
            stage.put(pid, out);
            if flagged {
                self.vm(&stage, pager)?.put(&mut stage, pager, pid, false)?;
            }
            self.space.vacate(&mut stage, pager, position)?;
            let mut gone = pager.reserve();
            gone.free(pid);
            stage.commit_with(pager, gone)?;
            if self.hint == Some(pid) {
                self.hint = None;
            }
            done.removed = true;
            return Ok(done);
        }
        let changed = out != page;
        let visible = if mark {
            plan.all_visible
        } else {
            flagged && !changed
        };
        page::set_all_visible(&mut out, visible);
        let roomier = page::free(&out) > page::free(&page);
        let current = storage::format(&out) == self.newest();
        let cat = category(page::free(&out), self.page_size);
        if out != page {
            stage.put(pid, out);
        }
        if visible != flagged {
            self.vm(&stage, pager)?
                .put(&mut stage, pager, pid, visible)?;
        }
        if roomier || current {
            self.space.set_category(&mut stage, pager, position, cat)?;
        }
        stage.commit(pager)?;
        done.all_visible = visible;
        Ok(done)
    }

    /// Returns the pointer of every out-of-line value a version of the heap holds.
    pub fn out_of_line<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pointer>, PageError> {
        let mut out = Vec::new();
        for pid in self.space.pages(pager)? {
            let page = pager.read(pid)?.to_vec();
            let format = storage::format(&page);
            for slot in 0..page::slot_count(&page) {
                if let Slot::Normal { at, .. } = page::get(&page, slot) {
                    out.extend(self.pointers(format, &page[at]));
                }
            }
        }
        Ok(out)
    }

    /// Insert `row` as a version written by `xid`. Returns its TID.
    ///
    /// Panics if `row` is not a row of the newest format.
    pub fn insert<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        xid: u64,
        row: &[Value],
    ) -> Result<Tid, Error> {
        let body = self.body(pager, row, &[])?;
        let version = version_bytes(Header::new(xid), &body);
        let mut stage = Stage::new();
        let t = self.place(&mut stage, pager, &version, Flags::NONE)?;
        stage.commit(pager)?;
        Ok(t)
    }

    /// Write `row` as the successor `xid` makes of the version at `at`, and point that version at
    /// it.
    ///
    /// The successor is heap-only when it fits on the old version's page and no field listed in
    /// `indexed`, the key fields of every index over this heap's constructor, changes.
    pub fn update<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        at: Tid,
        xid: u64,
        row: &[Value],
        indexed: &[usize],
    ) -> Result<Updated, Error> {
        let (format, _, old) = self
            .raw(pager, at)?
            .expect("update of a slot with no version");
        let (old_row, pointers) = self.decode(pager, format, &old)?;
        let old_row = self.upgrade(format, old_row);
        let keys_kept = indexed.iter().all(|&i| old_row[i] == row[i]);
        let body = self.body(pager, row, &pointers)?;
        let version = version_bytes(Header::new(xid), &body);
        let mut stage = Stage::new();
        let pid = Pid(at.page());
        let hot = if keys_kept {
            self.try_put(&mut stage, pager, pid, &version, Flags::HEAP_ONLY)?
        } else {
            None
        };
        let (tid, heap_only) = match hot {
            Some(t) => (t, true),
            None => (self.place(&mut stage, pager, &version, Flags::NONE)?, false),
        };
        let successor = if heap_only {
            Flags::HOT_UPDATED
        } else {
            Flags::NONE
        };
        self.supersede(&mut stage, pager, at, xid, Some(tid), successor)?;
        stage.commit(pager)?;
        Ok(Updated { tid, heap_only })
    }

    /// Mark the version at `at` deleted by `xid`.
    pub fn delete<R: Runtime>(&mut self, pager: &Pager<R>, at: Tid, xid: u64) -> Result<(), Error> {
        let mut stage = Stage::new();
        self.supersede(&mut stage, pager, at, xid, None, Flags::NONE)?;
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
        &mut self,
        pager: &Pager<R>,
        at: Tid,
        xid: u64,
        to: &mut Heap,
        row: &[Value],
    ) -> Result<Tid, Error> {
        assert_ne!(self.id, to.id, "a retag moves a row to another heap");
        assert_eq!(self.large, to.large, "a retag stays within one sum");
        let (format, _, old) = self
            .raw(pager, at)?
            .expect("retag of a slot with no version");
        let pointers = self.pointers(format, &old);
        let body = to.body(pager, row, &pointers)?;
        let version = version_bytes(Header::new(xid), &body);
        let mut stage = Stage::new();
        let tid = to.place(&mut stage, pager, &version, Flags::RETAGGED_IN)?;
        self.supersede(&mut stage, pager, at, xid, Some(tid), Flags::MOVED)?;
        stage.commit(pager)?;
        Ok(tid)
    }

    /// Returns what is wrong with the heap's pages, directory and visibility map.
    pub fn verify<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<String>, PageError> {
        Ok(self
            .check(pager)?
            .into_iter()
            .map(|(pid, what)| format!("{pid:?} {what}"))
            .collect())
    }

    /// Returns each page of the heap, its directory or its visibility map found wrong, with what
    /// is wrong with it.
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
            if page::all_visible(&page) != marked.remove(&pid) {
                problems.push((
                    pid,
                    "disagrees with the visibility map on being all-visible".into(),
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

    /// Returns every page the heap owns: its directory's, its own and its visibility map's.
    pub fn owned_pages<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pid>, PageError> {
        let mut out = self.space.nodes(pager)?;
        out.extend(self.space.pages(pager)?);
        out.extend(self.vm(&Stage::new(), pager)?.pages(pager)?);
        Ok(out)
    }
}

/// Follow the chain from the version at `start` through each `next`, across heaps, and return
/// the first version `stop` accepts. Returns `None` when the chain ends first.
///
/// The chain ends at a slot holding no version, at a page that is not a heap page, or at a
/// version whose `xmin` is not the `xmax` of the version before it.
pub fn walk<R: Runtime>(
    pager: &Pager<R>,
    start: Tid,
    mut stop: impl FnMut(&Link) -> bool,
) -> Result<Option<Link>, PageError> {
    let mut at = start;
    let mut prior = None;
    loop {
        let page = &pager.read(Pid(at.page()))?.to_vec();
        if !page::is_heap(page) {
            return Ok(None);
        }
        match page::get(page, usize::from(at.slot())) {
            Slot::Normal { flags, at: range } => {
                let link = Link {
                    tid: at,
                    heap: storage::owner(page),
                    header: Header::read(&page[range]),
                    flags,
                };
                if prior.is_some_and(|x| x != link.header.xmin) {
                    return Ok(None);
                }
                if stop(&link) {
                    return Ok(Some(link));
                }
                prior = Some(link.header.xmax);
                match link.header.next {
                    Some(next) => at = next,
                    None => return Ok(None),
                }
            }
            Slot::Redirect(slot) => at = tid(Pid(at.page()), usize::from(slot)),
            Slot::Unused | Slot::Dead => return Ok(None),
        }
    }
}
