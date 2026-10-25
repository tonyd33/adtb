use crate::key::{Tid, Value};
use crate::runtime::Runtime;
use crate::storage::{self, PageError, Pager, Pid};

use super::dir::{Space, category, max_fanout, need_category};
use super::large::LargeStore;
use super::page::{self, Arrangement, Slot, align8};
use super::row::{FieldType, Layout, POINTER_LEN, Pointer, Stored};
use super::stage::Stage;
use super::version::{Flags, HEADER_LEN, Header};
use super::{fixed, slotted};

/// Converts a row of the previous format into a row of this one.
pub type Upgrade = Box<dyn Fn(Vec<Value>) -> Vec<Value>>;

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
        pager: &mut Pager<R>,
        id: u32,
        row: Vec<FieldType>,
        large: LargeStore,
    ) -> Result<Heap, Error> {
        let fanout = max_fanout(pager.page_size());
        Heap::create_with_fanout(pager, id, row, large, fanout)
    }

    /// Create an empty heap whose directory pages hold `fanout` entries each.
    pub fn create_with_fanout<R: Runtime>(
        pager: &mut Pager<R>,
        id: u32,
        row: Vec<FieldType>,
        large: LargeStore,
        fanout: u16,
    ) -> Result<Heap, Error> {
        let layout = Layout::new(row);
        let arrangement = arrangement_of(&layout, pager.page_size())?;
        let space = Space::create(pager, id, fanout)?;
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
    pub fn pages<R: Runtime>(&self, pager: &mut Pager<R>) -> Result<Vec<Pid>, PageError> {
        self.space.pages(pager)
    }

    /// Returns the row format, flags and bytes of the version at `tid`, if a version is there.
    fn raw<R: Runtime>(
        &self,
        pager: &mut Pager<R>,
        tid: Tid,
    ) -> Result<Option<(u16, Flags, Vec<u8>)>, PageError> {
        let page = pager.read(Pid(tid.page()))?;
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
        pager: &mut Pager<R>,
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
        pager: &mut Pager<R>,
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
        pager: &mut Pager<R>,
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
        pager: &mut Pager<R>,
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
    fn convert<R: Runtime>(
        &self,
        pager: &mut Pager<R>,
        page: &[u8],
    ) -> Result<Option<Vec<u8>>, Error> {
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
        pager: &mut Pager<R>,
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
        self.space.set_category(stage, pager, position, cat)?;
        Ok(Some(tid(pid, slot)))
    }

    /// Insert `version` on some page of the heap, in `stage`.
    ///
    /// Precondition: `stage` holds no page of the heap's directory.
    fn place<R: Runtime>(
        &mut self,
        stage: &mut Stage,
        pager: &mut Pager<R>,
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
        while let Some(pid) = self.space.find(stage, pager, need)? {
            if let Some(t) = self.try_put(stage, pager, pid, version, flags)? {
                self.hint = Some(pid);
                return Ok(t);
            }
            let page = pager.read(pid)?;
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
        pager: &mut Pager<R>,
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

    /// Insert `row` as a version written by `xid`. Returns its TID.
    ///
    /// Panics if `row` is not a row of the newest format.
    pub fn insert<R: Runtime>(
        &mut self,
        pager: &mut Pager<R>,
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
        pager: &mut Pager<R>,
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
    pub fn delete<R: Runtime>(
        &mut self,
        pager: &mut Pager<R>,
        at: Tid,
        xid: u64,
    ) -> Result<(), Error> {
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
        pager: &mut Pager<R>,
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

    /// Returns what is wrong with the heap's pages and directory.
    pub fn verify<R: Runtime>(&self, pager: &mut Pager<R>) -> Result<Vec<String>, PageError> {
        let check = self.space.check(pager)?;
        let mut problems = check.problems;
        for (position, (pid, cat)) in check.leaves.into_iter().enumerate() {
            let page = pager.read(pid)?;
            if storage::owner(page) != self.id {
                problems.push(format!("{pid:?} is not owned by heap {}", self.id));
                continue;
            }
            if page::position(page) as usize != position {
                problems.push(format!("{pid:?} records position {}", page::position(page)));
            }
            let actual = category(page::free(page), self.page_size);
            let current = storage::format(page) == self.newest();
            if cat > actual || current && cat != actual {
                problems.push(format!(
                    "{pid:?} has category {cat} in the directory, {actual} on the page"
                ));
            }
            for slot in 0..page::slot_count(page) {
                if let Slot::Redirect(to) = page::get(page, slot)
                    && !matches!(page::get(page, usize::from(to)), Slot::Normal { .. })
                {
                    problems.push(format!("{pid:?} slot {slot} redirects to an empty slot"));
                }
            }
        }
        Ok(problems)
    }
}

/// Follow the chain from the version at `start` through each `next`, across heaps, and return
/// the first version `stop` accepts. Returns `None` when the chain ends first.
pub fn walk<R: Runtime>(
    pager: &mut Pager<R>,
    start: Tid,
    mut stop: impl FnMut(&Link) -> bool,
) -> Result<Option<Link>, PageError> {
    let mut at = start;
    loop {
        let page = pager.read(Pid(at.page()))?;
        match page::get(page, usize::from(at.slot())) {
            Slot::Normal { flags, at: range } => {
                let link = Link {
                    tid: at,
                    heap: storage::owner(page),
                    header: Header::read(&page[range]),
                    flags,
                };
                if stop(&link) {
                    return Ok(Some(link));
                }
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
