use std::collections::BTreeSet;

use crate::runtime::Runtime;
use crate::storage::{self, PageError, PageKind, Pager, Pid, Shape, Stopped};

use super::Refused;
use super::blame::{Blame, Holds, unchanged};
use super::page::{Arrangement, CONTENTS, HeapPage, Kinded, Owned, PageCopy, u16_at, u64_at};
use super::stage::Stage;
use super::vm;
use super::write::{Level, Rank};

const LEVEL: usize = CONTENTS;
const COUNT: usize = CONTENTS + 2;
const CAPACITY: usize = CONTENTS + 4;
const VM: usize = CONTENTS + 8;
const ENTRIES: usize = CONTENTS + 16;
const ENTRY: usize = 8;
const PID_MASK: u64 = (1 << 48) - 1;
/// The positions of a space: a position is a `u32`.
const POSITIONS: u64 = 1 << 32;
const TOUCHED: u64 = 1 << 56;

/// A level-0 entry naming no page: its page was removed and its position awaits a new one.
const HOLE: Pid = Pid(0);
/// A hole's category: every search for room finds it.
const HOLE_CATEGORY: u8 = 255;

/// Returns the most entries a directory page of `page_size` bytes holds.
pub fn max_fanout(page_size: usize) -> u16 {
    ((page_size - ENTRIES) / ENTRY).min(usize::from(u16::MAX)) as u16
}

fn level(node: &[u8]) -> u32 {
    u32::from(u16_at(node, LEVEL))
}

fn count(node: &[u8]) -> usize {
    usize::from(u16_at(node, COUNT))
}

fn capacity(node: &[u8]) -> u64 {
    u64::from(u16_at(node, CAPACITY))
}

fn entry(node: &[u8], i: usize) -> (Pid, u8) {
    let word = u64_at(node, ENTRIES + ENTRY * i);
    (Pid(word & PID_MASK), (word >> 48) as u8)
}

fn touched(node: &[u8], i: usize) -> bool {
    u64_at(node, ENTRIES + ENTRY * i) & TOUCHED != 0
}

fn any_touched(node: &[u8]) -> bool {
    (0..count(node)).any(|i| touched(node, i))
}

fn set_word(node: &mut [u8], i: usize, pid: Pid, category: u8, touched: bool) {
    let word = pid.0 | u64::from(category) << 48 | if touched { TOUCHED } else { 0 };
    let at = ENTRIES + ENTRY * i;
    node[at..at + ENTRY].copy_from_slice(&word.to_le_bytes());
}

fn push(node: &mut [u8], pid: Pid, category: u8) {
    let n = count(node);
    assert!((n as u64) < capacity(node), "directory node overflows");
    set_word(node, n, pid, category, false);
    node[COUNT..COUNT + 2].copy_from_slice(&(n as u16 + 1).to_le_bytes());
}

/// Drop the holes that end level-0 node `node` when it is not full.
///
/// Only the last node of a level is not full, so the positions dropped are the last of the space.
fn trim(node: &mut [u8]) {
    let mut n = count(node);
    if level(node) != 0 || n as u64 >= capacity(node) {
        return;
    }
    while n > 0 && entry(node, n - 1).0 == HOLE {
        n -= 1;
        set_word(node, n, HOLE, 0, false);
    }
    node[COUNT..COUNT + 2].copy_from_slice(&(n as u16).to_le_bytes());
}

fn init(node: &mut [u8], level: u32, capacity: u64) {
    let vm = u64_at(node, VM);
    node[CONTENTS..].fill(0);
    node[LEVEL..LEVEL + 2].copy_from_slice(&(level as u16).to_le_bytes());
    node[CAPACITY..CAPACITY + 2].copy_from_slice(&(capacity as u16).to_le_bytes());
    node[VM..VM + 8].copy_from_slice(&vm.to_le_bytes());
}

pub(super) fn vm_root(node: &[u8]) -> Option<Pid> {
    let pid = u64_at(node, VM);
    (pid != 0).then_some(Pid(pid))
}

fn max_category(node: &[u8]) -> u8 {
    (0..count(node))
        .map(|i| entry(node, i).1)
        .max()
        .unwrap_or(0)
}

/// The page, level and capacity of a directory page: its children are one level below it and
/// share its capacity.
type Parent = Option<(Pid, u32, u64)>;

fn parent_of(pid: Pid, node: &[u8]) -> Parent {
    Some((pid, level(node), capacity(node)))
}

/// Returns how `node` is out of shape as a directory page of `owner` whose entries fit its page,
/// a child of `parent` when there is one, at a level `l` with `capacity^l` below [`POSITIONS`].
fn misshapen(node: &impl Owned, owner: u32, page_size: usize, parent: Parent) -> Option<Shape> {
    if storage::kind(node) != PageKind::FreeSpaceMap as u16 {
        return Some(storage::kind_fault(node));
    }
    if node.owner() != owner {
        return Some(Shape::Owner);
    }
    let cap = capacity(node);
    let framed = (2..=u64::from(max_fanout(page_size))).contains(&cap)
        && count(node) as u64 <= cap
        && cap.checked_pow(level(node)).is_some_and(|s| s < POSITIONS)
        && parent.is_none_or(|(_, l, c)| l == level(node) + 1 && c == cap);
    (!framed).then_some(Shape::Frame)
}

/// Returns whether `node`, a directory or visibility map page, names `pid`.
pub(super) fn names_page(node: &[u8], pid: Pid) -> bool {
    match storage::kind(node) {
        k if k == PageKind::FreeSpaceMap as u16 => {
            let n = count(node).min(usize::from(max_fanout(node.len())));
            vm_root(node) == Some(pid) || (0..n).any(|i| entry(node, i).0 == pid)
        }
        k if k == PageKind::VisibilityMap as u16 => vm::names(node, pid),
        _ => false,
    }
}

/// Returns whether `node` is a level-0 directory page whose entry `i` names `pid`.
pub(super) fn names_at(node: &[u8], i: usize, pid: Pid) -> bool {
    storage::kind(node) == PageKind::FreeSpaceMap as u16
        && level(node) == 0
        && i < count(node).min(usize::from(max_fanout(node.len())))
        && entry(node, i).0 == pid
}

/// Why a directory or map page could not be read.
enum Fault {
    Page(PageError),
    /// A page is not a directory or map page where the entry naming it puts it.
    Shape(Blame),
}

/// Returns what a walk failing with `walked` found, or `None` when a page changed under it and the
/// walk runs again.
fn again<T>(walked: Result<T, Option<PageError>>) -> Result<Option<T>, PageError> {
    match walked {
        Ok(t) => Ok(Some(t)),
        Err(e) => e.map_or(Ok(None), Err),
    }
}

fn stopped(fault: Fault) -> Stopped {
    match fault {
        Fault::Page(e) => Stopped::from(e),
        Fault::Shape(_) => Stopped::Shape,
    }
}

/// The directory pages and the pages of a space, in walk order, and the visibility map its root
/// names.
#[derive(Default)]
struct Walked {
    nodes: Vec<Pid>,
    entries: Vec<Entry>,
    vm: Option<Pid>,
}

impl Walked {
    fn pages(&self) -> impl Iterator<Item = Pid> + '_ {
        self.entries.iter().map(|e| e.page).filter(|&p| p != HOLE)
    }
}

/// A level-0 entry: its position, the directory page holding it, its index there and the page it
/// names.
#[derive(Clone, Copy)]
struct Entry {
    position: u32,
    leaf: Pid,
    i: usize,
    page: Pid,
}

/// What [`Space::misnamed`] finds of a page an entry names.
enum Named {
    /// The page is a page of the space at the entry's position.
    Held,
    /// The entry changed while it was checked.
    Changed,
    /// The page to refuse.
    Blamed(Blame),
}

/// Returns the free space category of a page with `free` bytes free.
pub fn category(free: usize, page_size: usize) -> u8 {
    (free * 256 / page_size).min(255) as u8
}

/// Returns the free space category of `page`, a page of `page_size` bytes.
pub fn page_category(page: HeapPage<'_>, page_size: usize) -> u8 {
    category(page.free(), page_size)
}

/// What a write did to a page, which decides its level-0 directory entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// The page took a version: its entry gets the page's category.
    Took,
    /// The page took a chunk of a large value: its entry gets the page's category and is marked
    /// touched.
    Chunk,
    /// The page could not take a version: its entry gets the page's category in the newest
    /// format and 0 in an older one.
    Refused,
    /// The page was pruned: its entry gets the page's category when the page is in the newest
    /// format or has more room than before, and is left alone otherwise.
    Pruned { roomier: bool },
}

/// Returns the least category of a page certain to have `need` bytes free.
pub fn need_category(need: usize, page_size: usize) -> u16 {
    (need * 256).div_ceil(page_size).max(1) as u16
}

/// What [`Space::check`] found.
#[derive(Debug, Default)]
pub struct Check {
    /// Each page found wrong, with what is wrong with it.
    pub problems: Vec<(Pid, String)>,
    /// Each position of the space with its page and its entry's category, or `None` for a hole.
    pub leaves: Vec<Option<(Pid, u8)>>,
    /// Each directory page with an entry that promises less than its child holds, with what it
    /// promises too little.
    pub unsettled: Vec<(Pid, String)>,
    /// Each page whose level-0 entry is touched, in position order.
    pub touched: Vec<Pid>,
}

#[derive(Clone, Copy)]
struct Want {
    level: u32,
    capacity: u64,
    rightmost: bool,
}

fn digit(position: u64, level: u32, capacity: u64) -> usize {
    (position / capacity.pow(level) % capacity) as usize
}

/// The pages of a heap or a large-value store, named by a directory tree whose root page never
/// moves.
///
/// Invariants:
///
/// - Directory pages have kind free space map and the space's owner. A node at level 0 lists
///   pages of the space; a node at level `l` lists nodes at level `l - 1`.
/// - Pages are numbered by position in the order they were added. Every node but the last at
///   each level is full, so the path to position `p` is the digits of `p` in base `capacity`.
/// - Each page records its own position.
/// - A level-0 entry's category never exceeds the category of its page's free space, and
///   changes only under its page's exclusive latch.
/// - An inner entry's category is at least the greatest category of its child, and it is touched
///   when an entry of its child is, except while a change below it is settled: a crash then may
///   leave the entries above that change promising too little until [`Space::settle`].
/// - The last directory page at a level may be empty.
/// - A space opened at page 0 has no directory: it holds no page and takes none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Space {
    owner: u32,
    root: Option<Pid>,
}

impl Space {
    /// Create an empty space whose directory pages hold `fanout` entries each, and whose root
    /// names the visibility map rooted at `vm`, if any.
    pub fn create<R: Runtime>(
        pager: &Pager<R>,
        owner: u32,
        fanout: u16,
        vm: Option<Pid>,
    ) -> Result<Space, PageError> {
        assert!(
            fanout >= 2 && fanout <= max_fanout(pager.page_size()),
            "directory fanout {fanout} is out of range"
        );
        let mut new = pager.reserve();
        let root = new.page(1, PageKind::FreeSpaceMap, owner)?;
        pager.update_with(new, &[root], |p| {
            p[0][VM..VM + 8].copy_from_slice(&vm.map_or(0, |v| v.0).to_le_bytes());
            init(p[0], 0, u64::from(fanout));
        })?;
        Ok(Space {
            owner,
            root: Some(root),
        })
    }

    pub fn open(owner: u32, root: Pid) -> Space {
        Space {
            owner,
            root: (root != HOLE).then_some(root),
        }
    }

    pub fn owner(&self) -> u32 {
        self.owner
    }

    /// Returns the root page of the directory, page 0 for a space with none.
    pub fn root(&self) -> Pid {
        self.root.unwrap_or(HOLE)
    }

    /// Returns whether the space was opened at page 0, with no directory.
    pub fn is_placeholder(&self) -> bool {
        self.root.is_none()
    }

    /// Returns the root page of the directory and its contents as `stage` holds them, `None` for
    /// a space with no directory.
    fn top<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
    ) -> Result<Option<(Pid, Vec<u8>)>, Refused> {
        let Some(root) = self.root else {
            return Ok(None);
        };
        loop {
            if let Some(node) = self.node(stage, pager, root, None)? {
                return Ok(Some((root, node)));
            }
        }
    }

    /// Returns the root of the visibility map the directory's root names. Refuse the directory's
    /// root as out of shape when it names a page that is not a map page of the space.
    pub fn vm<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
    ) -> Result<Option<Pid>, Refused> {
        loop {
            let Some(map) = self.top(stage, pager)?.and_then(|(_, root)| vm_root(&root)) else {
                return Ok(None);
            };
            match self.map_page(stage, pager, map, None) {
                Ok(_) => return Ok(Some(map)),
                Err(Fault::Page(e)) => return Err(e.into()),
                Err(Fault::Shape(blame)) => {
                    if let Some(e) = blame.refuse(stage, pager) {
                        return Err(e);
                    }
                }
            }
        }
    }

    /// Refuse the directory's root, which names no visibility map, as out of frame, as
    /// [`Blame::refuse`] does. Returns `None` when, under its exclusive latch, it names one: the
    /// caller reads it again.
    ///
    /// Preconditions: the space has a directory, and `stage` holds no changed copy of its root.
    pub fn refuse_root<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
    ) -> Option<Refused> {
        let blame = Blame::new(self.rank(self.root(), 0), Shape::Frame, Holds::NoMap);
        blame.refuse(stage, pager)
    }

    /// Returns the rank of directory page `pid`, at `level` when it is not the root.
    fn rank(&self, pid: Pid, level: u32) -> Rank {
        if self.root == Some(pid) {
            Rank::Dir {
                level: Level::Root,
                pid,
            }
        } else {
            Rank::dir(level, pid)
        }
    }

    /// Returns `pid` as `stage` holds it, or as the pager does when it is not staged, read without
    /// latching it. Fail on a page that is not a directory page of the space one level below
    /// `parent`.
    ///
    /// Every walk of the directory reads its pages through this or [`Space::node_mut`].
    fn read<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        pid: Pid,
        parent: Parent,
    ) -> Result<Vec<u8>, Fault> {
        let level = parent.map_or(Level::Root, |(_, l, _)| Level::At(l.saturating_sub(1)));
        let node = stage.read_any(pager, Rank::Dir { level, pid });
        let node = node.map_err(Fault::Page)?;
        match misshapen(&node, self.owner, pager.page_size(), parent) {
            None => Ok(node.into_bytes()),
            Some(shape) => {
                let rank = self.rank(pid, parent.map_or(0, |(_, l, _)| l.saturating_sub(1)));
                let named_by = parent.map(|(p, l, _)| self.rank(p, l));
                Err(Fault::Shape(Blame::of(
                    rank,
                    named_by,
                    shape,
                    node.into_bytes(),
                )))
            }
        }
    }

    /// Returns visibility map page `pid` as `stage` holds it, or as the pager does, read without
    /// latching it. Fail on a page that is not a map page of the space one level below `parent`,
    /// the map page naming it and its level, blaming that page, or the directory's root for the
    /// map's root.
    fn map_page<'p, R: Runtime>(
        &self,
        stage: &Stage<'p, R>,
        pager: &'p Pager<R>,
        pid: Pid,
        parent: Option<(Pid, u32)>,
    ) -> Result<Vec<u8>, Fault> {
        let node = stage.read_any(pager, Rank::Vm(pid));
        let node = node.map_err(Fault::Page)?;
        match vm::misshapen(&node, self.owner, parent.map(|(_, l)| l)) {
            None => Ok(node.into_bytes()),
            Some(shape) => Err(Fault::Shape(Blame::new(
                parent.map_or(self.rank(self.root(), 0), |(p, _)| Rank::Vm(p)),
                shape,
                Holds::Names(pid),
            ))),
        }
    }

    /// Returns `fault` as the error of a read that refuses a page out of shape, `None` when the
    /// page changed before its refusal and the read runs again.
    fn fail<R: Runtime>(&self, pager: &Pager<R>, fault: Fault) -> Option<PageError> {
        match fault {
            Fault::Page(e) => Some(e),
            Fault::Shape(blame) => blame.refuse_unchanged(&mut Stage::new(), pager),
        }
    }

    /// Returns `pid` as [`Space::read`] does. Refuse as out of shape the directory page whose
    /// entry names a page that is not a directory page where the entry puts it, or the root when
    /// it is not one. Returns `None` when the page naming `pid` changed before its refusal: the
    /// caller reads again from the root.
    fn node<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        pid: Pid,
        parent: Parent,
    ) -> Result<Option<Vec<u8>>, Refused> {
        loop {
            match self.read(stage, pager, pid, parent) {
                Ok(node) => return Ok(Some(node)),
                Err(Fault::Page(e)) => return Err(e.into()),
                Err(Fault::Shape(blame)) => {
                    let named = blame.pid() != pid;
                    if let Some(e) = blame.refuse(stage, pager) {
                        return Err(e);
                    }
                    if named {
                        return Ok(None);
                    }
                }
            }
        }
    }

    /// Returns the staged copy of directory page `pid`, expected at `level`, latching and staging
    /// it first if needed. Refuse it as out of shape under that latch as [`Space::node`] does.
    fn node_mut<'s, 'p, R: Runtime>(
        &self,
        stage: &'s mut Stage<'p, R>,
        pager: &'p Pager<R>,
        pid: Pid,
        level: u32,
    ) -> Result<&'s mut PageCopy, PageError> {
        let rank = Rank::dir(level, pid);
        let node = stage.page(pager, rank)?;
        match misshapen(node, self.owner, pager.page_size(), None) {
            None => stage.page(pager, rank),
            Some(shape) => Err(stage.refuse(pid, shape)),
        }
    }

    /// Returns the directory pages, the root first, and the pages of the space in position order,
    /// failing with what `fault` makes of a directory page that cannot be read or has an entry at a
    /// position of [`POSITIONS`] or more.
    fn walk<R: Runtime, E>(
        &self,
        pager: &Pager<R>,
        fault: impl Fn(Fault) -> E,
    ) -> Result<Walked, E> {
        let mut out = Walked::default();
        let Some(root) = self.root else {
            return Ok(out);
        };
        let mut todo = vec![(root, None, 0)];
        while let Some((pid, parent, base)) = todo.pop() {
            let node = self
                .read(&mut Stage::new(), pager, pid, parent)
                .map_err(&fault)?;
            out.nodes.push(pid);
            if parent.is_none() {
                out.vm = vm_root(&node);
            }
            let entries = (0..count(&node)).map(|i| (i, entry(&node, i).0));
            let span = capacity(&node).pow(level(&node));
            let at = |i: usize| base + i as u64 * span;
            if count(&node)
                .checked_sub(1)
                .is_some_and(|i| at(i) >= POSITIONS)
            {
                let rank = self.rank(pid, level(&node));
                let blame = Blame::of(rank, None, Shape::Frame, node.clone());
                return Err(fault(Fault::Shape(blame)));
            }
            if level(&node) == 0 {
                out.entries.extend(entries.map(|(i, page)| Entry {
                    position: at(i) as u32,
                    leaf: pid,
                    i,
                    page,
                }));
            } else {
                todo.extend(
                    entries
                        .rev()
                        .map(|(i, c)| (c, parent_of(pid, &node), at(i))),
                );
            }
        }
        Ok(out)
    }

    /// Returns every page of the space, in position order. A page named at two positions is
    /// refused as [`Space::refuse_misnamed`] refuses it, or by its second entry's directory page
    /// when each entry finds it held.
    ///
    /// Preconditions: the caller holds no page latch.
    pub fn pages<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pid>, PageError> {
        loop {
            let Some(walked) = again(self.walk(pager, |f| self.fail(pager, f)))? else {
                continue;
            };
            let mut seen = BTreeSet::new();
            let Some(twice) = walked.pages().find(|&p| !seen.insert(p)) else {
                return Ok(walked.pages().collect());
            };
            if self
                .refuse_entries(pager, &walked.entries, twice)?
                .is_some()
            {
                let mut named = walked.entries.iter().filter(|e| e.page == twice);
                let first = named.next().expect("a page named twice has a first entry");
                let second = named.next().expect("a page named twice has a second entry");
                let holds = Holds::Twice {
                    page: twice,
                    at: second.i,
                    first: first.leaf,
                    first_at: first.i,
                };
                let blame = Blame::new(self.rank(second.leaf, 0), Shape::Frame, holds);
                if let Some(e) = blame.refuse_unchanged(&mut Stage::new(), pager) {
                    return Err(e);
                }
            }
        }
    }

    /// Refuse page `pid`, read as no page of the space, at each position a level-0 entry names it
    /// where it is not a page of the space: record as damaged `pid` when its header claims that
    /// position, the directory page holding the entry otherwise. Returns whether an entry names
    /// `pid`, from a walk whose every entry naming `pid` was checked unchanged.
    ///
    /// Retry while an entry changes under its check. A try holds one latch at a time and none
    /// across tries.
    ///
    /// Preconditions: the caller holds no page latch.
    pub fn refuse_misnamed<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
    ) -> Result<bool, PageError> {
        loop {
            let Some(walked) = again(self.walk(pager, |f| self.fail(pager, f)))? else {
                continue;
            };
            if let Some(named) = self.refuse_entries(pager, &walked.entries, pid)? {
                return Ok(named);
            }
        }
    }

    /// Refuse `pid` at each of `entries` naming it as [`Space::refuse_misnamed`] does. Returns
    /// whether one names it, or `None` when one changed under its check.
    fn refuse_entries<R: Runtime>(
        &self,
        pager: &Pager<R>,
        entries: &[Entry],
        pid: Pid,
    ) -> Result<Option<bool>, PageError> {
        let mut named = false;
        for e in entries.iter().filter(|e| e.page == pid) {
            named = true;
            match self.misnamed(pager, e.leaf, e.i, pid, e.position)? {
                Named::Held => {}
                Named::Changed => return Ok(None),
                Named::Blamed(blame) => {
                    return blame
                        .refuse_unchanged(&mut Stage::new(), pager)
                        .map_or(Ok(None), Err);
                }
            }
        }
        Ok(Some(named))
    }

    /// Returns every directory page of the space, then every page of it, then every page of the
    /// visibility map its root names, failing with what `fault` makes of a page that cannot be
    /// read.
    fn all<R: Runtime, E>(
        &self,
        pager: &Pager<R>,
        fault: impl Fn(Fault) -> E,
    ) -> Result<Vec<Pid>, E> {
        let walked = self.walk(pager, &fault)?;
        let pages: Vec<Pid> = walked.pages().collect();
        let mut out = walked.nodes;
        out.extend(pages);
        if let Some(map) = walked.vm {
            let stage = Stage::new();
            let read = |pid, parent| self.map_page(&stage, pager, pid, parent).map_err(&fault);
            out.extend(vm::walk(map, read)?);
        }
        Ok(out)
    }

    /// Returns the pages of [`Space::all`]. Record as damaged the page naming a directory or map
    /// page out of shape.
    pub fn owned<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pid>, PageError> {
        loop {
            if let Some(pages) = again(self.all(pager, |f| self.fail(pager, f)))? {
                return Ok(pages);
            }
        }
    }

    /// Returns the pages of [`Space::all`], for a walk mapping damage to its holder. Stop on a
    /// directory or map page out of shape, recording no damage.
    pub fn held<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pid>, Stopped> {
        self.all(pager, stopped)
    }

    /// Returns what is wrong with the directory, and its level-0 entries in position order, a hole
    /// as `None`.
    pub fn check<R: Runtime>(&self, pager: &Pager<R>) -> Result<Check, PageError> {
        let mut found = Check::default();
        let Some(pid) = self.root else {
            return Ok(found);
        };
        let root = pager.read_formatted(pid)?.to_vec();
        let at = Want {
            level: level(&root),
            capacity: capacity(&root),
            rightmost: true,
        };
        self.check_node(pager, pid, at, &mut found)?;
        Ok(found)
    }

    fn check_node<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        want: Want,
        found: &mut Check,
    ) -> Result<(u8, bool), PageError> {
        let problems = &mut found.problems;
        let node = PageCopy::of(&pager.read_formatted(pid)?);
        if storage::kind(&node) != PageKind::FreeSpaceMap as u16 || node.owner() != self.owner {
            problems.push((pid, format!("is not a directory page of {}", self.owner)));
            return Ok((0, false));
        }
        if level(&node) != want.level || capacity(&node) != want.capacity {
            problems.push((pid, "has the wrong level or capacity".into()));
            return Ok((0, false));
        }
        let n = count(&node);
        if !want.rightmost && n as u64 != want.capacity {
            problems.push((pid, "is not full but is not the last at its level".into()));
        }
        if want.level > 0 && n == 0 && !want.rightmost {
            problems.push((pid, "is an empty inner directory page".into()));
        }
        for i in 0..n {
            let (child, cat) = entry(&node, i);
            if want.level == 0 {
                found.leaves.push((child != HOLE).then_some((child, cat)));
                if touched(&node, i) {
                    found.touched.push(child);
                }
                if child == HOLE && cat != HOLE_CATEGORY {
                    found
                        .problems
                        .push((pid, format!("entry {i} is a hole of category {cat}")));
                }
                continue;
            }
            let below = Want {
                level: want.level - 1,
                capacity: want.capacity,
                rightmost: want.rightmost && i == n - 1,
            };
            let (top, any) = self.check_node(pager, child, below, found)?;
            if top > cat {
                found.unsettled.push((
                    pid,
                    format!("entry {i} has category {cat}, its child's greatest is {top}"),
                ));
            }
            if any && !touched(&node, i) {
                found.unsettled.push((
                    pid,
                    format!("entry {i} is not touched, an entry of its child is"),
                ));
            }
        }
        Ok((max_category(&node), any_touched(&node)))
    }

    /// Returns a page whose entry has category `need` or more, if there is one.
    ///
    /// A hole found first gets a new empty page in `arrangement` and row format `format`, written
    /// with its entry in one update of its own. An inner entry found to promise more than its
    /// child holds is set to its child's greatest category, in one update of its own. When an
    /// entry still names a page that is not a page of the space at the entry's position, the
    /// directory page holding the entry is refused as damaged.
    fn find<R: Runtime>(
        &self,
        pager: &Pager<R>,
        need: u16,
        arrangement: Arrangement,
        format: u16,
    ) -> Result<Option<Pid>, PageError> {
        let stage = &mut Stage::new();
        'search: loop {
            let Some((mut here, mut node)) = self.top(stage, pager).map_err(unchanged)? else {
                return Ok(None);
            };
            let cap = capacity(&node);
            let mut position = 0;
            let mut above = None;
            loop {
                let Some(i) = (0..count(&node)).find(|&i| u16::from(entry(&node, i).1) >= need)
                else {
                    let Some((parent, j)) = above else {
                        return Ok(None);
                    };
                    self.repair(pager, parent, j, here)?;
                    continue 'search;
                };
                position += i as u64 * cap.pow(level(&node));
                let child = entry(&node, i).0;
                if level(&node) > 0 {
                    above = Some(((here, level(&node)), i));
                    let below = self
                        .node(stage, pager, child, parent_of(here, &node))
                        .map_err(unchanged)?;
                    let Some(below) = below else {
                        continue 'search;
                    };
                    node = below;
                    here = child;
                } else if child == HOLE {
                    match self.fill(pager, position as u32, arrangement, format)? {
                        Some(pid) => return Ok(Some(pid)),
                        None => continue 'search,
                    }
                } else if self.holds(pager, child, position as u32)? {
                    return Ok(Some(child));
                } else if let Named::Blamed(blame) =
                    self.misnamed(pager, here, i, child, position as u32)?
                {
                    if let Some(e) = blame.refuse_unchanged(stage, pager) {
                        return Err(e);
                    }
                    continue 'search;
                } else {
                    continue 'search;
                }
            }
        }
    }

    /// Returns whether `pid` is a page of the space at `position`.
    fn holds<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        position: u32,
    ) -> Result<bool, PageError> {
        Ok(self.at(&pager.read(pid)?, position))
    }

    fn at(&self, page: &impl Owned, position: u32) -> bool {
        self.claimed(page, position)
            .is_some_and(|k| k.framed().is_some())
    }

    /// Returns `page` when its header names a heap kind, the space's owner and `position`.
    fn claimed<'g>(&self, page: &'g impl Owned, position: u32) -> Option<Kinded<'g>> {
        Kinded::of(page).filter(|k| k.owner() == self.owner && k.position() == position)
    }

    /// Returns what entry `i` of level-0 directory page `leaf`, naming `pid` at `position`, finds:
    /// [`Named::Blamed`] holds `pid` when its header claims that position, `leaf` otherwise. Holds
    /// one latch at a time: `leaf` is read optimistically and validated after `pid` is read under
    /// its shared latch.
    ///
    /// Precondition: `i` is below the most entries a directory page holds.
    fn misnamed<R: Runtime>(
        &self,
        pager: &Pager<R>,
        leaf: Pid,
        i: usize,
        pid: Pid,
        position: u32,
    ) -> Result<Named, PageError> {
        let seen = pager.optimistic(leaf)?;
        if !seen.read(|node| level(node) == 0 && i < count(node) && entry(node, i).0 == pid) {
            return Ok(Named::Changed);
        }
        let page = pager.read(pid)?;
        let (held, claimed) = (
            self.at(&page, position),
            self.claimed(&page, position).is_some(),
        );
        let (misfit, bytes) = (self.misfit(&page), page.to_vec());
        drop(page);
        Ok(match (held, seen.validate(), claimed) {
            (true, _, _) => Named::Held,
            (false, false, _) => Named::Changed,
            (false, true, true) => {
                Named::Blamed(Blame::of(Rank::Heap(pid), None, Shape::Frame, bytes))
            }
            (false, true, false) => {
                let parent = Some(self.rank(leaf, 0));
                Named::Blamed(Blame::of(Rank::Heap(pid), parent, misfit, bytes))
            }
        })
    }

    /// Returns how `page`, which is not a page of the space where an entry names it, is out of
    /// shape.
    fn misfit(&self, page: &impl Owned) -> Shape {
        match Kinded::of(page) {
            None => storage::kind_fault(page),
            Some(k) if k.owner() != self.owner => Shape::Owner,
            Some(_) => Shape::Frame,
        }
    }

    /// Set entry `i` of directory page `parent`, at level `l`, to the greatest category and the
    /// touched mark of `child`, in an update of its own, if the entry still names `child`.
    /// Returns whether it did.
    fn repair<R: Runtime>(
        &self,
        pager: &Pager<R>,
        (parent, l): (Pid, u32),
        i: usize,
        child: Pid,
    ) -> Result<bool, PageError> {
        let mut stage = Stage::new();
        let node = self.node_mut(&mut stage, pager, parent, l)?.clone();
        if level(&node) == 0 || i >= count(&node) || entry(&node, i).0 != child {
            return Ok(false);
        }
        let Some(below) = self
            .node(&mut stage, pager, child, parent_of(parent, &node))
            .map_err(unchanged)?
        else {
            return Ok(false);
        };
        let (top, any) = (max_category(&below), any_touched(&below));
        let node = stage.page(pager, Rank::dir(l, parent))?;
        if entry(node, i).1 != top || touched(node, i) != any {
            set_word(node, i, child, top, any);
            stage.commit(pager)?;
        }
        Ok(true)
    }

    /// Returns the directory pages on the path to `position`, the root first and the level-0 page
    /// last, with the directory's capacity, read without holding a latch across pages. Returns
    /// `None` if an inner page on the way has no entry for `position`.
    fn path<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        position: u32,
    ) -> Result<Option<(Vec<Pid>, u64)>, Refused> {
        'path: loop {
            let Some((mut pid, mut node)) = self.top(stage, pager)? else {
                return Ok(None);
            };
            let cap = capacity(&node);
            let mut path = vec![pid];
            while level(&node) > 0 {
                let i = digit(u64::from(position), level(&node), cap);
                if i >= count(&node) {
                    return Ok(None);
                }
                let child = entry(&node, i).0;
                path.push(child);
                let Some(below) = self.node(stage, pager, child, parent_of(pid, &node))? else {
                    continue 'path;
                };
                (pid, node) = (child, below);
            }
            return Ok(Some((path, cap)));
        }
    }

    /// Returns the level-0 directory page holding the entry at `position`, if the path to it has
    /// one.
    #[cfg(test)]
    pub fn leaf_of<R: Runtime>(
        &self,
        pager: &Pager<R>,
        position: u32,
    ) -> Result<Option<Pid>, PageError> {
        let path = self
            .path(&mut Stage::new(), pager, position)
            .map_err(unchanged)?;
        Ok(path.map(|(path, _)| *path.last().unwrap()))
    }

    /// Returns whether the entry at `position` names `pid`, read without holding a latch across
    /// pages, while the caller holds latches up to `floor`.
    pub fn names<R: Runtime>(
        &self,
        pager: &Pager<R>,
        position: u32,
        pid: Pid,
        floor: Option<Rank>,
    ) -> Result<bool, PageError> {
        let stage = &mut Stage::above(floor);
        'names: loop {
            let Some((mut at, mut node)) = self.top(stage, pager).map_err(unchanged)? else {
                return Ok(false);
            };
            let cap = capacity(&node);
            for l in (0..=level(&node)).rev() {
                let i = digit(u64::from(position), l, cap);
                if level(&node) != l || i >= count(&node) {
                    return Ok(false);
                }
                let child = entry(&node, i).0;
                if l == 0 {
                    return Ok(child == pid);
                }
                let Some(below) = self
                    .node(stage, pager, child, parent_of(at, &node))
                    .map_err(unchanged)?
                else {
                    continue 'names;
                };
                (at, node) = (child, below);
            }
            return Ok(false);
        }
    }

    /// Give the hole at `position` a new empty page. Returns `None`, changing nothing, if another
    /// writer filled it first.
    fn fill<R: Runtime>(
        &self,
        pager: &Pager<R>,
        position: u32,
        arrangement: Arrangement,
        format: u16,
    ) -> Result<Option<Pid>, PageError> {
        let mut new = pager.reserve();
        let page = new.page(format, arrangement.kind(), self.owner)?;
        let mut stage = Stage::new();
        let image = stage.put_reserved(&new, page, new.image(page));
        arrangement.init(image, position);
        let fresh = HeapPage::any(image).expect("a page formatted empty is a heap page");
        let cat = page_category(fresh, pager.page_size());
        let fresh = Change {
            page: Some(page),
            category: Some(cat),
            mark: None,
        };
        let changed = self
            .change(&mut stage, pager, position, HOLE, fresh)
            .map_err(unchanged)?;
        let Some((above, _)) = changed else {
            return Ok(None);
        };
        stage.commit_with(pager, new)?;
        above.settle(pager)?;
        Ok(Some(page))
    }

    /// Replace the entry at `position`, which names `pid`, by a hole, in `stage`. The page it named
    /// is no longer part of the space. When the level-0 directory page holding the entry is not
    /// full, the holes that end it leave it.
    ///
    /// Precondition: `stage` holds `pid` and no directory page.
    pub fn vacate<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        pid: Pid,
        position: u32,
    ) -> Result<Above, Refused> {
        let hole = Change {
            page: Some(HOLE),
            category: Some(HOLE_CATEGORY),
            mark: Some(false),
        };
        let Some((changed, leaf)) = self.change(stage, pager, position, pid, hole)? else {
            return Ok(Above::default());
        };
        trim(stage.page(pager, Rank::dir(0, leaf))?);
        Ok(changed)
    }

    /// Returns the number of positions, holes included.
    fn len<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
    ) -> Result<u64, Refused> {
        'len: loop {
            let Some((mut at, mut node)) = self.top(stage, pager)? else {
                return Ok(0);
            };
            let cap = capacity(&node);
            let mut total = 0;
            while level(&node) > 0 && count(&node) > 0 {
                let n = count(&node) as u64;
                total += (n - 1) * cap.pow(level(&node));
                let last = entry(&node, count(&node) - 1).0;
                let Some(below) = self.node(stage, pager, last, parent_of(at, &node))? else {
                    continue 'len;
                };
                (at, node) = (last, below);
            }
            return Ok(total + count(&node) as u64);
        }
    }

    /// Set the level-0 entry of `pid`, at the position its staged copy records, in `stage`, as
    /// `fit` says from the staged copy, `newest` being the newest row format of the space's
    /// pages. An entry that names another page is left alone.
    ///
    /// Precondition: `stage` holds `pid` and no directory page.
    pub fn reflect<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        pid: Pid,
        newest: u16,
        fit: Fit,
    ) -> Result<Above, Refused> {
        let copy = stage.read(pager, Rank::Heap(pid))?;
        let page = HeapPage::any(&copy).expect("a page a write reflects is a heap page");
        let category = page_category(page, pager.page_size());
        let current = storage::format(&page) == newest;
        let (category, mark) = match fit {
            Fit::Took => (Some(category), None),
            Fit::Chunk => (Some(category), Some(true)),
            Fit::Refused => (Some(if current { category } else { 0 }), None),
            Fit::Pruned { roomier } => ((roomier || current).then_some(category), None),
        };
        if category.is_none() {
            return Ok(Above::default());
        }
        self.set_entry(stage, pager, pid, page.position(), category, mark)
    }

    /// Mark or unmark the entry at `position`, which names `page`, as touched in `stage`. An entry
    /// that names another page is left alone.
    ///
    /// Precondition: `stage` holds `page` and no directory page.
    pub fn touch<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        page: Pid,
        position: u32,
        on: bool,
    ) -> Result<Above, Refused> {
        self.set_entry(stage, pager, page, position, None, Some(on))
    }

    /// Set the category and the touched mark of the entry at `position`, which names `page`, in
    /// `stage`, each kept as it is when `None`. An entry that names another page is left alone.
    ///
    /// Precondition: `stage` holds `page` and no directory page.
    fn set_entry<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        page: Pid,
        position: u32,
        category: Option<u8>,
        mark: Option<bool>,
    ) -> Result<Above, Refused> {
        let to = Change {
            page: None,
            category,
            mark,
        };
        let changed = self.change(stage, pager, position, page, to)?;
        Ok(changed.map(|(above, _)| above).unwrap_or_default())
    }

    /// Set the entry at `position`, which names `expect`, in `stage` as `to` says, latching only
    /// the level-0 directory page holding it. Returns the entries above it to settle once `stage`
    /// is written and the level-0 page it latched, or `None`, staging nothing, if the entry does
    /// not name `expect`.
    ///
    /// Precondition: `stage` holds no directory page, checked in debug builds.
    fn change<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        position: u32,
        expect: Pid,
        to: Change,
    ) -> Result<Option<(Above, Pid)>, Refused> {
        stage.holds_no_directory();
        let Change {
            page,
            category,
            mark,
        } = to;
        loop {
            let Some((path, cap)) = self.path(stage, pager, position)? else {
                return Ok(None);
            };
            let leaf = *path.last().unwrap();
            let i = digit(u64::from(position), 0, cap);
            let fresh = !stage.holds(leaf);
            let node = self.node_mut(stage, pager, leaf, 0)?;
            if level(node) != 0 {
                if fresh {
                    stage.unstage(leaf);
                }
                continue;
            }
            if i >= count(node) || entry(node, i).0 != expect {
                if fresh {
                    stage.unstage(leaf);
                }
                return Ok(None);
            }
            let (_, old) = entry(node, i);
            let was = touched(node, i);
            let cat = category.unwrap_or(old);
            let on = mark.unwrap_or(was);
            set_word(node, i, page.unwrap_or(expect), cat, on);
            let rise = cat > old || on && !was;
            let fall = !on && was;
            let above = Above {
                at: (rise || fall).then_some((*self, position)),
            };
            return Ok(Some((above, leaf)));
        }
    }

    /// Set each entry above the one at `position` to the greatest category and the touched mark
    /// of its child, the lowest first, each in an update of its own.
    fn settle_above<R: Runtime>(&self, pager: &Pager<R>, position: u32) -> Result<(), PageError> {
        'walk: loop {
            let Some((path, cap)) = self
                .path(&mut Stage::new(), pager, position)
                .map_err(unchanged)?
            else {
                return Ok(());
            };
            for k in (0..path.len() - 1).rev() {
                let l = (path.len() - 1 - k) as u32;
                let i = digit(u64::from(position), l, cap);
                if !self.repair(pager, (path[k], l), i, path[k + 1])? {
                    continue 'walk;
                }
            }
            return Ok(());
        }
    }

    /// Set every inner entry to the greatest category and the touched mark of its child, the
    /// deepest first, each in an update of its own.
    pub fn settle<R: Runtime>(&self, pager: &Pager<R>) -> Result<(), PageError> {
        let Some(root) = self.root else {
            return Ok(());
        };
        while !self.settle_node(pager, root, None)? {}
        Ok(())
    }

    /// Settle the entries below `pid`. Returns false when a page naming one changed under the
    /// walk, which runs again from the root.
    fn settle_node<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        parent: Parent,
    ) -> Result<bool, PageError> {
        let Some(node) = self
            .node(&mut Stage::new(), pager, pid, parent)
            .map_err(unchanged)?
        else {
            return Ok(false);
        };
        if level(&node) == 0 {
            return Ok(true);
        }
        for i in 0..count(&node) {
            let child = entry(&node, i).0;
            if !self.settle_node(pager, child, parent_of(pid, &node))? {
                return Ok(false);
            }
            self.repair(pager, (pid, level(&node)), i, child)?;
        }
        Ok(true)
    }

    /// Returns every page whose entry is touched, with its position, in position order.
    pub fn touched<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<(u32, Pid)>, PageError> {
        loop {
            if let Some(out) = self.touched_once(pager)? {
                return Ok(out);
            }
        }
    }

    /// Returns what [`Space::touched`] returns, or `None` when a page naming another changed
    /// under the walk.
    fn touched_once<R: Runtime>(
        &self,
        pager: &Pager<R>,
    ) -> Result<Option<Vec<(u32, Pid)>>, PageError> {
        let mut out = Vec::new();
        let Some((pid, root)) = self.top(&mut Stage::new(), pager).map_err(unchanged)? else {
            return Ok(Some(out));
        };
        let cap = capacity(&root);
        let mut todo = vec![(pid, 0u64, None)];
        while let Some((pid, base, parent)) = todo.pop() {
            let Some(node) = self
                .node(&mut Stage::new(), pager, pid, parent)
                .map_err(unchanged)?
            else {
                return Ok(None);
            };
            let l = level(&node);
            for i in (0..count(&node)).rev() {
                if !touched(&node, i) {
                    continue;
                }
                let at = base + i as u64 * cap.pow(l);
                let (child, _) = entry(&node, i);
                if l == 0 {
                    out.push((at as u32, child));
                } else {
                    todo.push((child, at, parent_of(pid, &node)));
                }
            }
        }
        out.sort();
        Ok(Some(out))
    }

    /// Returns a page whose entry has category `need` or more as [`Space::find`] does, or else a
    /// new one [`Space::append`] adds. Returns `None` for a space with no directory.
    pub fn room<R: Runtime>(
        &self,
        pager: &Pager<R>,
        need: u16,
        arrangement: Arrangement,
        format: u16,
    ) -> Result<Option<Pid>, PageError> {
        let Some(root) = self.root else {
            return Ok(None);
        };
        match self.find(pager, need, arrangement, format)? {
            Some(pid) => Ok(Some(pid)),
            None => self.append(pager, root, arrangement, format).map(Some),
        }
    }

    /// Add a new page in `arrangement` and row format `format`, empty, at the next position of
    /// the space whose directory's root is `root`.
    ///
    /// Each step is an update of its own: growing the root, adding each directory page the
    /// position needs, then the new page with its entry, latching one level-0 directory page;
    /// the entries above it are settled afterwards.
    fn append<R: Runtime>(
        &self,
        pager: &Pager<R>,
        root: Pid,
        arrangement: Arrangement,
        format: u16,
    ) -> Result<Pid, PageError> {
        'append: loop {
            let stage = &mut Stage::new();
            let Some(top) = self.node(stage, pager, root, None).map_err(unchanged)? else {
                continue;
            };
            let (mut at, mut node) = (root, top);
            let (cap, height) = (capacity(&node), level(&node));
            let position = self.len(stage, pager).map_err(unchanged)?;
            if position == cap.pow(height + 1) {
                self.grow(pager, at, height)?;
                continue;
            }
            let position = u32::try_from(position).expect("a space holds fewer than 2^32 pages");
            while level(&node) > 0 {
                let i = digit(u64::from(position), level(&node), cap);
                if i >= count(&node) {
                    break;
                }
                let child = entry(&node, i).0;
                let Some(below) = self
                    .node(stage, pager, child, parent_of(at, &node))
                    .map_err(unchanged)?
                else {
                    continue 'append;
                };
                (at, node) = (child, below);
            }
            let l = level(&node);
            if l > 0 {
                self.add_node(pager, at, l, digit(u64::from(position), l, cap))?;
                continue;
            }
            if let Some(pid) = self.add_page(pager, at, position, arrangement, format)? {
                return Ok(pid);
            }
        }
    }

    /// Move the entries of root `pid` to a new directory page and make the root list it alone, one
    /// level up, if the root is still at `height` and full, in an update of its own.
    fn grow<R: Runtime>(&self, pager: &Pager<R>, pid: Pid, height: u32) -> Result<(), PageError> {
        let mut stage = Stage::new();
        let old = self.node_mut(&mut stage, pager, pid, height)?.clone();
        let cap = capacity(&old);
        if level(&old) != height
            || self.len(&mut stage, pager).map_err(unchanged)? != cap.pow(height + 1)
        {
            return Ok(());
        }
        let mut new = pager.reserve();
        let moved = new.page(1, PageKind::FreeSpaceMap, self.owner)?;
        stage.put_reserved(&new, moved, new.image(moved));
        let copy = stage.page(pager, Rank::dir(height, moved))?;
        copy[CONTENTS..].copy_from_slice(&old[CONTENTS..]);
        copy[VM..VM + 8].fill(0);
        let root = stage.page(pager, Rank::dir(height, pid))?;
        init(root, height + 1, cap);
        push(root, moved, 0);
        set_word(root, 0, moved, max_category(&old), any_touched(&old));
        stage.commit_with(pager, new)
    }

    /// Add an empty directory page one level below `parent` as its entry `i`, if `parent` is still
    /// at level `l` with `i` entries, in an update of its own.
    fn add_node<R: Runtime>(
        &self,
        pager: &Pager<R>,
        parent: Pid,
        l: u32,
        i: usize,
    ) -> Result<(), PageError> {
        let mut stage = Stage::new();
        let node = self.node_mut(&mut stage, pager, parent, l)?;
        if level(node) != l || count(node) != i {
            return Ok(());
        }
        let cap = capacity(node);
        let mut new = pager.reserve();
        let child = new.page(1, PageKind::FreeSpaceMap, self.owner)?;
        stage.put_reserved(&new, child, new.image(child));
        let below = Rank::dir(l - 1, child);
        init(stage.page(pager, below)?, l - 1, cap);
        push(stage.page(pager, Rank::dir(l, parent))?, child, 0);
        stage.commit_with(pager, new)
    }

    /// Add a new empty page at `position` as the next entry of level-0 directory page `leaf`, in
    /// an update of its own, and settle the entries above it. Returns `None`, changing nothing, if
    /// `leaf` no longer ends just before `position`.
    fn add_page<R: Runtime>(
        &self,
        pager: &Pager<R>,
        leaf: Pid,
        position: u32,
        arrangement: Arrangement,
        format: u16,
    ) -> Result<Option<Pid>, PageError> {
        let mut new = pager.reserve();
        let page = new.page(format, arrangement.kind(), self.owner)?;
        let mut stage = Stage::new();
        let image = stage.put_reserved(&new, page, new.image(page));
        arrangement.init(image, position);
        let fresh = HeapPage::any(image).expect("a page formatted empty is a heap page");
        let cat = page_category(fresh, pager.page_size());
        let node = self.node_mut(&mut stage, pager, leaf, 0)?;
        let i = digit(u64::from(position), 0, capacity(node));
        if level(node) != 0 || count(node) != i {
            return Ok(None);
        }
        push(node, page, cat);
        stage.commit_with(pager, new)?;
        self.settle_above(pager, position)?;
        Ok(Some(page))
    }
}

/// What [`Space::change`] makes of an entry: the page it names, its category and its touched
/// mark, each kept as it is when `None`.
struct Change {
    page: Option<Pid>,
    category: Option<u8>,
    mark: Option<bool>,
}

/// The entries above a changed directory entry, to settle once the change is written.
#[must_use = "a raised or remarked entry leaves the entries above it to settle"]
#[derive(Debug, Default)]
pub struct Above {
    at: Option<(Space, u32)>,
}

impl Above {
    /// Settle the entries above the changed one, each in an update of its own.
    ///
    /// Precondition: the change is written.
    pub fn settle<R: Runtime>(self, pager: &Pager<R>) -> Result<(), PageError> {
        match self.at {
            Some((space, position)) => space.settle_above(pager, position),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
#[path = "dir_tests.rs"]
mod tests;
