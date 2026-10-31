use crate::runtime::Runtime;
use crate::storage::{self, PageError, PageKind, Pager, Pid};

use super::page::{self, Arrangement, CONTENTS, u16_at, u64_at};
use super::stage::Stage;

const LEVEL: usize = CONTENTS;
const COUNT: usize = CONTENTS + 2;
const CAPACITY: usize = CONTENTS + 4;
const VM: usize = CONTENTS + 8;
const ENTRIES: usize = CONTENTS + 16;
const ENTRY: usize = 8;
const PID_MASK: u64 = (1 << 48) - 1;
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

fn vm_root(node: &[u8]) -> Option<Pid> {
    let pid = u64_at(node, VM);
    (pid != 0).then_some(Pid(pid))
}

fn max_category(node: &[u8]) -> u8 {
    (0..count(node))
        .map(|i| entry(node, i).1)
        .max()
        .unwrap_or(0)
}

/// Returns the free space category of a page with `free` bytes free.
pub fn category(free: usize, page_size: usize) -> u8 {
    (free * 256 / page_size).min(255) as u8
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Space {
    owner: u32,
    root: Pid,
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
        Ok(Space { owner, root })
    }

    pub fn open(owner: u32, root: Pid) -> Space {
        Space { owner, root }
    }

    pub fn owner(&self) -> u32 {
        self.owner
    }

    pub fn root(&self) -> Pid {
        self.root
    }

    /// Returns the root of the visibility map the directory's root names.
    pub fn vm<'p, R: Runtime>(
        &self,
        stage: &Stage<'p, R>,
        pager: &'p Pager<R>,
    ) -> Result<Option<Pid>, PageError> {
        Ok(vm_root(&stage.read(pager, self.root)?))
    }

    /// Returns every page of the space, in position order.
    pub fn pages<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pid>, PageError> {
        let mut out = Vec::new();
        let mut todo = vec![self.root];
        while let Some(pid) = todo.pop() {
            let node = &pager.read_formatted(pid)?.to_vec();
            let entries = (0..count(node)).map(|i| entry(node, i).0);
            if level(node) == 0 {
                out.extend(entries.filter(|&p| p != HOLE));
            } else {
                let children: Vec<Pid> = entries.collect();
                todo.extend(children.into_iter().rev());
            }
        }
        Ok(out)
    }

    /// Returns every directory page of the space, the root first.
    pub fn nodes<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pid>, PageError> {
        let mut out = Vec::new();
        let mut todo = vec![self.root];
        while let Some(pid) = todo.pop() {
            out.push(pid);
            let node = &pager.read(pid)?.to_vec();
            if level(node) > 0 {
                todo.extend((0..count(node)).rev().map(|i| entry(node, i).0));
            }
        }
        Ok(out)
    }

    /// Returns what is wrong with the directory, and its level-0 entries in position order, a hole
    /// as `None`.
    pub fn check<R: Runtime>(&self, pager: &Pager<R>) -> Result<Check, PageError> {
        let mut found = Check::default();
        let root = pager.read(self.root)?.to_vec();
        let at = Want {
            level: level(&root),
            capacity: capacity(&root),
            rightmost: true,
        };
        self.check_node(pager, self.root, at, &mut found)?;
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
        let node = pager.read(pid)?.to_vec();
        if storage::kind(&node) != PageKind::FreeSpaceMap as u16
            || storage::owner(&node) != self.owner
        {
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
    /// child holds is set to its child's greatest category, in one update of its own.
    ///
    /// Precondition: `stage` holds no page of the directory.
    pub fn find<'p, R: Runtime>(
        &self,
        stage: &Stage<'p, R>,
        pager: &'p Pager<R>,
        need: u16,
        arrangement: Arrangement,
        format: u16,
    ) -> Result<Option<Pid>, PageError> {
        'search: loop {
            let mut here = self.root;
            let mut node = stage.read(pager, here)?;
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
                    above = Some((here, i));
                    here = child;
                    node = stage.read(pager, child)?;
                } else if child == HOLE {
                    match self.fill(pager, position as u32, arrangement, format)? {
                        Some(pid) => return Ok(Some(pid)),
                        None => continue 'search,
                    }
                } else {
                    return Ok(Some(child));
                }
            }
        }
    }

    /// Set entry `i` of directory page `parent` to the greatest category and the touched mark of
    /// `child`, in an update of its own, if the entry still names `child`. Returns whether it did.
    fn repair<R: Runtime>(
        &self,
        pager: &Pager<R>,
        parent: Pid,
        i: usize,
        child: Pid,
    ) -> Result<bool, PageError> {
        let mut stage = Stage::new();
        let node = stage.page(pager, parent)?;
        if level(node) == 0 || i >= count(node) || entry(node, i).0 != child {
            return Ok(false);
        }
        let below = stage.read(pager, child)?;
        let (top, any) = (max_category(&below), any_touched(&below));
        let node = stage.page(pager, parent)?;
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
        stage: &Stage<'p, R>,
        pager: &'p Pager<R>,
        position: u32,
    ) -> Result<Option<(Vec<Pid>, u64)>, PageError> {
        let mut pid = self.root;
        let mut node = stage.read(pager, pid)?;
        let cap = capacity(&node);
        let mut path = vec![pid];
        while level(&node) > 0 {
            let i = digit(u64::from(position), level(&node), cap);
            if i >= count(&node) {
                return Ok(None);
            }
            pid = entry(&node, i).0;
            path.push(pid);
            node = stage.read(pager, pid)?;
        }
        Ok(Some((path, cap)))
    }

    /// Returns whether the entry at `position` names `pid`, read without holding a latch across
    /// pages.
    pub fn names<R: Runtime>(
        &self,
        pager: &Pager<R>,
        position: u32,
        pid: Pid,
    ) -> Result<bool, PageError> {
        let mut node = pager.read(self.root)?.to_vec();
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
            node = pager.read(child)?.to_vec();
        }
        Ok(false)
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
        let mut image = new.image(page);
        arrangement.init(&mut image, position);
        let cat = category(page::free(&image), pager.page_size());
        let mut stage = Stage::new();
        stage.put(pager, page, image)?;
        let fresh = Change {
            page: Some(page),
            category: Some(cat),
            mark: None,
        };
        let changed = self.change(&mut stage, pager, position, HOLE, fresh)?;
        let Some(above) = changed else {
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
    ) -> Result<Above, PageError> {
        let hole = Change {
            page: Some(HOLE),
            category: Some(HOLE_CATEGORY),
            mark: Some(false),
        };
        let Some(changed) = self.change(stage, pager, position, pid, hole)? else {
            return Ok(Above::default());
        };
        if let Some((path, _)) = self.path(stage, pager, position)? {
            trim(stage.page(pager, *path.last().unwrap())?);
        }
        Ok(changed)
    }

    /// Returns the number of positions, holes included.
    fn len<'p, R: Runtime>(
        &self,
        stage: &Stage<'p, R>,
        pager: &'p Pager<R>,
    ) -> Result<u64, PageError> {
        let mut node = stage.read(pager, self.root)?;
        let cap = capacity(&node);
        let mut total = 0;
        while level(&node) > 0 && count(&node) > 0 {
            let n = count(&node) as u64;
            total += (n - 1) * cap.pow(level(&node));
            node = stage.read(pager, entry(&node, count(&node) - 1).0)?;
        }
        Ok(total + count(&node) as u64)
    }

    /// Set the category of the entry at `position`, which names `page`, in `stage`. An entry that
    /// names another page is left alone.
    ///
    /// Precondition: `stage` holds `page` and no directory page.
    pub fn set_category<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        page: Pid,
        position: u32,
        category: u8,
    ) -> Result<Above, PageError> {
        self.set_entry(stage, pager, page, position, Some(category), None)
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
    ) -> Result<Above, PageError> {
        self.set_entry(stage, pager, page, position, None, Some(on))
    }

    /// Set the category and the touched mark of the entry at `position`, which names `page`, in
    /// `stage`, each kept as it is when `None`. An entry that names another page is left alone.
    ///
    /// Precondition: `stage` holds `page` and no directory page.
    pub fn set_entry<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        page: Pid,
        position: u32,
        category: Option<u8>,
        mark: Option<bool>,
    ) -> Result<Above, PageError> {
        let to = Change {
            page: None,
            category,
            mark,
        };
        let changed = self.change(stage, pager, position, page, to)?;
        Ok(changed.unwrap_or_default())
    }

    /// Set the entry at `position`, which names `expect`, in `stage` as `to` says, latching only
    /// the level-0 directory page holding it. Returns the entries above it to settle once `stage`
    /// is written, or `None`, staging nothing, if the entry does not name `expect`.
    fn change<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        position: u32,
        expect: Pid,
        to: Change,
    ) -> Result<Option<Above>, PageError> {
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
            let node = stage.page(pager, leaf)?;
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
            return Ok(Some(Above {
                at: (rise || fall).then_some((*self, position)),
            }));
        }
    }

    /// Set each entry above the one at `position` to the greatest category and the touched mark
    /// of its child, the lowest first, each in an update of its own.
    fn settle_above<R: Runtime>(&self, pager: &Pager<R>, position: u32) -> Result<(), PageError> {
        'walk: loop {
            let Some((path, cap)) = self.path(&Stage::new(), pager, position)? else {
                return Ok(());
            };
            for k in (0..path.len() - 1).rev() {
                let l = (path.len() - 1 - k) as u32;
                let i = digit(u64::from(position), l, cap);
                if !self.repair(pager, path[k], i, path[k + 1])? {
                    continue 'walk;
                }
            }
            return Ok(());
        }
    }

    /// Set every inner entry to the greatest category and the touched mark of its child, the
    /// deepest first, each in an update of its own.
    pub fn settle<R: Runtime>(&self, pager: &Pager<R>) -> Result<(), PageError> {
        self.settle_node(pager, self.root)
    }

    fn settle_node<R: Runtime>(&self, pager: &Pager<R>, pid: Pid) -> Result<(), PageError> {
        let node = pager.read(pid)?.to_vec();
        if level(&node) == 0 {
            return Ok(());
        }
        for i in 0..count(&node) {
            let child = entry(&node, i).0;
            self.settle_node(pager, child)?;
            self.repair(pager, pid, i, child)?;
        }
        Ok(())
    }

    /// Returns every page whose entry is touched, with its position, in position order.
    pub fn touched<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<(u32, Pid)>, PageError> {
        let mut out = Vec::new();
        let root = pager.read(self.root)?.to_vec();
        let cap = capacity(&root);
        let mut todo = vec![(self.root, 0u64)];
        while let Some((pid, base)) = todo.pop() {
            let node = pager.read(pid)?.to_vec();
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
                    todo.push((child, at));
                }
            }
        }
        out.sort();
        Ok(out)
    }

    /// Add a new page in `arrangement` and row format `format`, empty, at the next position.
    ///
    /// Each step is an update of its own: growing the root, adding each directory page the
    /// position needs, then the new page with its entry, latching one level-0 directory page;
    /// the entries above it are settled afterwards.
    pub fn append<R: Runtime>(
        &self,
        pager: &Pager<R>,
        arrangement: Arrangement,
        format: u16,
    ) -> Result<Pid, PageError> {
        loop {
            let stage = Stage::new();
            let root = stage.read(pager, self.root)?;
            let (cap, height) = (capacity(&root), level(&root));
            let position = self.len(&stage, pager)?;
            if position == cap.pow(height + 1) {
                self.grow(pager, height)?;
                continue;
            }
            let position = u32::try_from(position).expect("a space holds fewer than 2^32 pages");
            let mut at = self.root;
            let mut node = root;
            while level(&node) > 0 {
                let i = digit(u64::from(position), level(&node), cap);
                if i >= count(&node) {
                    break;
                }
                at = entry(&node, i).0;
                node = stage.read(pager, at)?;
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

    /// Move the root's entries to a new directory page and make the root list it alone, one level
    /// up, if the root is still at `height` and full, in an update of its own.
    fn grow<R: Runtime>(&self, pager: &Pager<R>, height: u32) -> Result<(), PageError> {
        let mut stage = Stage::new();
        let old = stage.page(pager, self.root)?.clone();
        let cap = capacity(&old);
        if level(&old) != height || self.len(&stage, pager)? != cap.pow(height + 1) {
            return Ok(());
        }
        let mut new = pager.reserve();
        let moved = new.page(1, PageKind::FreeSpaceMap, self.owner)?;
        stage.put(pager, moved, new.image(moved))?;
        let copy = stage.page(pager, moved)?;
        copy[CONTENTS..].copy_from_slice(&old[CONTENTS..]);
        copy[VM..VM + 8].fill(0);
        let root = stage.page(pager, self.root)?;
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
        let node = stage.page(pager, parent)?;
        if level(node) != l || count(node) != i {
            return Ok(());
        }
        let cap = capacity(node);
        let mut new = pager.reserve();
        let child = new.page(1, PageKind::FreeSpaceMap, self.owner)?;
        stage.put(pager, child, new.image(child))?;
        init(stage.page(pager, child)?, l - 1, cap);
        push(stage.page(pager, parent)?, child, 0);
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
        let mut image = new.image(page);
        arrangement.init(&mut image, position);
        let cat = category(page::free(&image), pager.page_size());
        let mut stage = Stage::new();
        stage.put(pager, page, image)?;
        let node = stage.page(pager, leaf)?;
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
