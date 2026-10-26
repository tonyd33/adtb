use crate::runtime::Runtime;
use crate::storage::{PageError, PageKind, Pager, Pid};

use super::page::{CONTENTS, u16_at, u64_at};
use super::stage::Stage;

const LEVEL: usize = CONTENTS;
const BODY: usize = CONTENTS + 8;

/// Returns the number of page ids one leaf of a map over pages of `page_size` bytes covers.
fn span(page_size: usize) -> u64 {
    ((page_size - BODY) * 8) as u64
}

/// Returns the number of children an inner page of a map over pages of `page_size` bytes holds.
fn fanout(page_size: usize) -> u64 {
    ((page_size - BODY) / 8) as u64
}

fn level(node: &[u8]) -> u32 {
    u32::from(u16_at(node, LEVEL))
}

fn set_level(node: &mut [u8], level: u32) {
    node[LEVEL..LEVEL + 2].copy_from_slice(&(level as u16).to_le_bytes());
}

/// Returns the number of page ids a node at `level` covers.
pub(super) fn reach(page_size: usize, level: u32) -> u64 {
    span(page_size).saturating_mul(fanout(page_size).saturating_pow(level))
}

/// Returns the child of a node at `level`, above the leaves, on the path to `pid`.
pub(super) fn child_index(page_size: usize, level: u32, pid: Pid) -> usize {
    (pid.0 / reach(page_size, level - 1) % fanout(page_size)) as usize
}

/// Returns the bit of `pid` in its leaf.
pub(super) fn bit_index(page_size: usize, pid: Pid) -> u64 {
    pid.0 % span(page_size)
}

fn child(node: &[u8], i: usize) -> Option<Pid> {
    let pid = u64_at(node, BODY + 8 * i);
    (pid != 0).then_some(Pid(pid))
}

fn set_child(node: &mut [u8], i: usize, pid: Pid) {
    node[BODY + 8 * i..BODY + 8 * i + 8].copy_from_slice(&pid.0.to_le_bytes());
}

fn bit(leaf: &[u8], i: u64) -> bool {
    leaf[BODY + (i / 8) as usize] >> (i % 8) & 1 == 1
}

fn set_bit(leaf: &mut [u8], i: u64, on: bool) {
    let byte = &mut leaf[BODY + (i / 8) as usize];
    let mask = 1 << (i % 8);
    *byte = if on { *byte | mask } else { *byte & !mask };
}

/// The visibility map of one heap: one all-visible bit per page id, in a radix tree of
/// visibility map pages owned by the heap. The root page never moves.
///
/// Invariants:
///
/// - A leaf (level 0) covers `span` consecutive page ids from a multiple of `span`; bit `i` of
///   its body is the bit of the `i`th. An inner page at level `l` lists up to `fanout` children
///   at level `l - 1`, zero for one not yet added.
/// - The root at level `l` covers page ids from 0 below `span * fanout^l`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vm {
    root: Pid,
    owner: u32,
}

impl Vm {
    pub fn create<R: Runtime>(pager: &Pager<R>, owner: u32) -> Result<Vm, PageError> {
        let root = pager.allocate(1, PageKind::VisibilityMap, owner)?;
        Ok(Vm { root, owner })
    }

    pub fn open(root: Pid, owner: u32) -> Vm {
        Vm { root, owner }
    }

    pub fn root(&self) -> Pid {
        self.root
    }

    /// Returns the leaf covering `pid`, if one exists, as `stage` holds the map.
    fn leaf<'p, R: Runtime>(
        &self,
        stage: &Stage<'p, R>,
        pager: &'p Pager<R>,
        pid: Pid,
    ) -> Result<Option<Pid>, PageError> {
        let size = pager.page_size();
        let mut node = stage.read(pager, self.root)?;
        if pid.0 >= reach(size, level(&node)) {
            return Ok(None);
        }
        let mut at = self.root;
        while level(&node) > 0 {
            let Some(c) = child(&node, child_index(size, level(&node), pid)) else {
                return Ok(None);
            };
            at = c;
            node = stage.read(pager, c)?;
        }
        Ok(Some(at))
    }

    /// Returns whether the bit of `pid` is set.
    pub fn get<R: Runtime>(&self, pager: &Pager<R>, pid: Pid) -> Result<bool, PageError> {
        let stage = Stage::new();
        let Some(leaf) = self.leaf(&stage, pager, pid)? else {
            return Ok(false);
        };
        let i = bit_index(pager.page_size(), pid);
        Ok(bit(&pager.read(leaf)?, i))
    }

    /// Set or clear the bit of `pid` in `stage`, latching its leaf.
    ///
    /// Precondition: to set a bit, [`Vm::extend`] has added the path to `pid`.
    ///
    /// Panics if a bit to set has no leaf.
    pub fn put<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        pid: Pid,
        on: bool,
    ) -> Result<(), PageError> {
        loop {
            let Some(leaf) = self.leaf(stage, pager, pid)? else {
                assert!(!on, "the visibility map has no leaf for {pid:?}");
                return Ok(());
            };
            let fresh = !stage.holds(leaf);
            let node = stage.page(pager, leaf)?;
            // The root may have grown above the leaf it was when looked up.
            if level(node) != 0 {
                if fresh {
                    stage.unstage(leaf);
                }
                continue;
            }
            set_bit(node, bit_index(pager.page_size(), pid), on);
            return Ok(());
        }
    }

    /// Clear the bit of `pid`, in an update of its own.
    pub fn clear<R: Runtime>(&self, pager: &Pager<R>, pid: Pid) -> Result<(), PageError> {
        let mut stage = Stage::new();
        self.put(&mut stage, pager, pid, false)?;
        stage.commit(pager)
    }

    /// Add the map pages the path to `pid` lacks, each under the latch of the page that links
    /// it and in an update of its own.
    pub fn extend<R: Runtime>(&self, pager: &Pager<R>, pid: Pid) -> Result<(), PageError> {
        let size = pager.page_size();
        loop {
            let mut stage = Stage::new();
            let root = stage.page(pager, self.root)?;
            let l = level(root);
            if pid.0 < reach(size, l) {
                break;
            }
            let mut new = pager.reserve();
            let moved = new.page(1, PageKind::VisibilityMap, self.owner)?;
            let contents = root[CONTENTS..].to_vec();
            root[CONTENTS..].fill(0);
            set_level(root, l + 1);
            set_child(root, 0, moved);
            stage.put(pager, moved, new.image(moved))?;
            stage.page(pager, moved)?[CONTENTS..].copy_from_slice(&contents);
            stage.commit_with(pager, new)?;
        }
        let mut at = self.root;
        loop {
            let mut stage = Stage::new();
            let node = stage.page(pager, at)?;
            let l = level(node);
            if l == 0 {
                return Ok(());
            }
            let i = child_index(size, l, pid);
            if let Some(c) = child(node, i) {
                at = c;
                continue;
            }
            let mut new = pager.reserve();
            let c = new.page(1, PageKind::VisibilityMap, self.owner)?;
            set_child(node, i, c);
            stage.put(pager, c, new.image(c))?;
            set_level(stage.page(pager, c)?, l - 1);
            stage.commit_with(pager, new)?;
            at = c;
        }
    }

    /// Returns every page of the map, the root first.
    pub fn pages<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pid>, PageError> {
        let size = pager.page_size();
        let mut out = Vec::new();
        let mut todo = vec![self.root];
        while let Some(at) = todo.pop() {
            out.push(at);
            let node = &pager.read(at)?.to_vec();
            if level(node) > 0 {
                todo.extend(
                    (0..fanout(size) as usize)
                        .rev()
                        .filter_map(|i| child(node, i)),
                );
            }
        }
        Ok(out)
    }

    /// Returns every page id whose bit is set, in order.
    pub fn marked<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pid>, PageError> {
        let size = pager.page_size();
        let mut out = Vec::new();
        let mut todo = vec![(self.root, 0)];
        while let Some((at, base)) = todo.pop() {
            let node = &pager.read(at)?.to_vec();
            let l = level(node);
            if l == 0 {
                out.extend(
                    (0..span(size))
                        .filter(|&i| bit(node, i))
                        .map(|i| Pid(base + i)),
                );
                continue;
            }
            let below = reach(size, l - 1);
            let children: Vec<(Pid, u64)> = (0..fanout(size) as usize)
                .filter_map(|i| child(node, i).map(|c| (c, base + i as u64 * below)))
                .collect();
            todo.extend(children.into_iter().rev());
        }
        Ok(out)
    }
}
