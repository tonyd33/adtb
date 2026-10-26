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
/// - A level-0 entry's category never exceeds the category of its page's free space; an inner
///   entry's category is the greatest category of its child.
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
            let node = &pager.read(pid)?.to_vec();
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
    ) -> Result<u8, PageError> {
        let problems = &mut found.problems;
        let node = pager.read(pid)?.to_vec();
        if storage::kind(&node) != PageKind::FreeSpaceMap as u16
            || storage::owner(&node) != self.owner
        {
            problems.push((pid, format!("is not a directory page of {}", self.owner)));
            return Ok(0);
        }
        if level(&node) != want.level || capacity(&node) != want.capacity {
            problems.push((pid, "has the wrong level or capacity".into()));
            return Ok(0);
        }
        let n = count(&node);
        if !want.rightmost && n as u64 != want.capacity {
            problems.push((pid, "is not full but is not the last at its level".into()));
        }
        if want.level > 0 && n == 0 {
            problems.push((pid, "is an empty inner directory page".into()));
        }
        for i in 0..n {
            let (child, cat) = entry(&node, i);
            if want.level == 0 {
                found.leaves.push((child != HOLE).then_some((child, cat)));
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
            let top = self.check_node(pager, child, below, found)?;
            if top != cat {
                found.problems.push((
                    pid,
                    format!("entry {i} has category {cat}, its child's greatest is {top}"),
                ));
            }
        }
        Ok(max_category(&node))
    }

    /// Returns a page whose entry has category `need` or more, if there is one.
    ///
    /// A hole found first gets a new empty page in `arrangement` and row format `format`, written
    /// with its entry in one update of its own.
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
            let mut node = stage.read(pager, self.root)?;
            let cap = capacity(&node);
            let mut position = 0;
            loop {
                let Some(i) = (0..count(&node)).find(|&i| u16::from(entry(&node, i).1) >= need)
                else {
                    return Ok(None);
                };
                position += i as u64 * cap.pow(level(&node));
                let child = entry(&node, i).0;
                if level(&node) > 0 {
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

    /// Latch the path to `position` in `stage`, the root first. Returns each node of the path
    /// with the index of its entry on the path, the root first.
    fn latch_path<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        position: u32,
    ) -> Result<Vec<(Pid, usize)>, PageError> {
        let root = stage.page(pager, self.root)?;
        let cap = capacity(root);
        let top = level(root);
        let mut path = Vec::new();
        let mut pid = self.root;
        for l in (0..=top).rev() {
            let node = stage.page(pager, pid)?;
            let i = digit(u64::from(position), l, cap);
            assert!(
                i < count(node),
                "position {position} is past the directory's end"
            );
            path.push((pid, i));
            pid = entry(node, i).0;
        }
        Ok(path)
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
        let mut stage = Stage::new();
        let path = self.latch_path(&mut stage, pager, position)?;
        let (leaf, i) = *path.last().unwrap();
        if entry(&stage.read(pager, leaf)?, i).0 != HOLE {
            return Ok(None);
        }
        let mut new = pager.reserve();
        let page = new.page(format, arrangement.kind(), self.owner)?;
        stage.put(pager, page, new.image(page))?;
        let fresh = stage.page(pager, page)?;
        arrangement.init(fresh, position);
        let cat = category(page::free(fresh), pager.page_size());
        self.write_entry(&mut stage, pager, position, Some(page), Some(cat), None)?;
        stage.commit_with(pager, new)?;
        Ok(Some(page))
    }

    /// Replace the entry at `position` by a hole, in `stage`. The page it named is no longer part
    /// of the space.
    pub fn vacate<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        position: u32,
    ) -> Result<(), PageError> {
        self.write_entry(
            stage,
            pager,
            position,
            Some(HOLE),
            Some(HOLE_CATEGORY),
            Some(false),
        )
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

    /// Set the category of the entry at `position` in `stage`, and the categories above it.
    pub fn set_category<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        position: u32,
        category: u8,
    ) -> Result<(), PageError> {
        self.write_entry(stage, pager, position, None, Some(category), None)
    }

    /// Mark or unmark the entry at `position` in `stage` as touched, and the entries above it.
    pub fn touch<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        position: u32,
        on: bool,
    ) -> Result<(), PageError> {
        self.write_entry(stage, pager, position, None, None, Some(on))
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

    /// Set the entry at `position` in `stage` to name `page`, with `category` and marked
    /// `touched`, each kept as it is when `None`, and the entries above it. The path is latched
    /// the root first.
    fn write_entry<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        position: u32,
        page: Option<Pid>,
        category: Option<u8>,
        mark: Option<bool>,
    ) -> Result<(), PageError> {
        let path = self.latch_path(stage, pager, position)?;
        let (leaf, i) = *path.last().unwrap();
        let node = stage.page(pager, leaf)?;
        let (old_page, old_cat) = entry(node, i);
        let t = mark.unwrap_or(touched(node, i));
        set_word(
            node,
            i,
            page.unwrap_or(old_page),
            category.unwrap_or(old_cat),
            t,
        );
        for k in (0..path.len() - 1).rev() {
            let below = stage.read(pager, path[k + 1].0)?;
            let (top, any) = (max_category(&below), any_touched(&below));
            let (parent, i) = path[k];
            let node = stage.page(pager, parent)?;
            let (child, _) = entry(node, i);
            set_word(node, i, child, top, any);
        }
        Ok(())
    }

    /// Add a new page in `arrangement` and row format `format`, empty, at the next position.
    ///
    /// The new page, the directory pages it needs, its entry and the directory pages it touches
    /// are written in one update, the root latched first.
    pub fn append<R: Runtime>(
        &self,
        pager: &Pager<R>,
        arrangement: Arrangement,
        format: u16,
    ) -> Result<Pid, PageError> {
        let mut stage = Stage::new();
        let root = stage.page(pager, self.root)?.clone();
        let cap = capacity(&root);
        let height = level(&root);
        let position = self.len(&stage, pager)?;
        let position32 = u32::try_from(position).expect("a space holds fewer than 2^32 pages");
        let split = position == cap.pow(height + 1);
        let fresh = if split {
            height + 1
        } else {
            let mut node = root.clone();
            let mut l = height;
            while digit(position, l, cap) < count(&node) {
                node = stage.read(pager, entry(&node, digit(position, l, cap)).0)?;
                l -= 1;
            }
            l
        };
        let mut new = pager.reserve();
        let page = new.page(format, arrangement.kind(), self.owner)?;
        let moved = if split {
            Some(new.page(1, PageKind::FreeSpaceMap, self.owner)?)
        } else {
            None
        };
        let mut chain = Vec::new();
        for _ in 0..fresh {
            chain.push(new.page(1, PageKind::FreeSpaceMap, self.owner)?);
        }
        for &pid in [page].iter().chain(&moved).chain(&chain) {
            stage.put(pager, pid, new.image(pid))?;
        }
        let mut height = height;
        if let Some(moved) = moved {
            let old = stage.read(pager, self.root)?;
            let copy = stage.page(pager, moved)?;
            copy[CONTENTS..].copy_from_slice(&old[CONTENTS..]);
            copy[VM..VM + 8].fill(0);
            let top = max_category(&old);
            let r = stage.page(pager, self.root)?;
            init(r, height + 1, cap);
            push(r, moved, top);
            set_word(r, 0, moved, top, any_touched(&old));
            height += 1;
        }
        let mut pid = self.root;
        for l in (fresh + 1..=height).rev() {
            let node = stage.read(pager, pid)?;
            pid = entry(&node, digit(position, l, cap)).0;
        }
        let mut child_level = fresh;
        let mut parent = pid;
        for &node in &chain {
            child_level -= 1;
            init(stage.page(pager, node)?, child_level, cap);
            push(stage.page(pager, parent)?, node, 0);
            parent = node;
        }
        push(stage.page(pager, parent)?, page, 0);
        let fresh_page = stage.page(pager, page)?;
        arrangement.init(fresh_page, position32);
        let free = page::free(fresh_page);
        self.set_category(
            &mut stage,
            pager,
            position32,
            category(free, pager.page_size()),
        )?;
        stage.commit_with(pager, new)?;
        Ok(page)
    }
}
