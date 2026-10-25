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
    pub fn create<R: Runtime>(pager: &mut Pager<R>, owner: u32) -> Result<Vm, PageError> {
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
    fn leaf<R: Runtime>(
        &self,
        stage: &Stage,
        pager: &mut Pager<R>,
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
    pub fn get<R: Runtime>(&self, pager: &mut Pager<R>, pid: Pid) -> Result<bool, PageError> {
        let stage = Stage::new();
        let Some(leaf) = self.leaf(&stage, pager, pid)? else {
            return Ok(false);
        };
        let i = bit_index(pager.page_size(), pid);
        Ok(bit(&pager.read(leaf)?, i))
    }

    /// Set or clear the bit of `pid` in `stage`.
    ///
    /// Setting a bit first adds, each in an update of its own, the map pages its path lacks.
    pub fn put<R: Runtime>(
        &self,
        stage: &mut Stage,
        pager: &mut Pager<R>,
        pid: Pid,
        on: bool,
    ) -> Result<(), PageError> {
        if on {
            self.extend(pager, pid)?;
        }
        let Some(leaf) = self.leaf(stage, pager, pid)? else {
            return Ok(());
        };
        let i = bit_index(pager.page_size(), pid);
        set_bit(stage.page(pager, leaf)?, i, on);
        Ok(())
    }

    /// Add the map pages the path to `pid` lacks.
    fn extend<R: Runtime>(&self, pager: &mut Pager<R>, pid: Pid) -> Result<(), PageError> {
        let size = pager.page_size();
        while pid.0 >= reach(size, level(&pager.read(self.root)?)) {
            let moved = pager.allocate(1, PageKind::VisibilityMap, self.owner)?;
            pager.update(&[self.root, moved], |p| {
                let (root, copy) = p.split_at_mut(1);
                copy[0][CONTENTS..].copy_from_slice(&root[0][CONTENTS..]);
                let l = level(root[0]);
                root[0][CONTENTS..].fill(0);
                set_level(root[0], l + 1);
                set_child(root[0], 0, moved);
            })?;
        }
        let mut at = self.root;
        loop {
            let node = pager.read(at)?.to_vec();
            let l = level(&node);
            if l == 0 {
                return Ok(());
            }
            let i = child_index(size, l, pid);
            at = match child(&node, i) {
                Some(c) => c,
                None => {
                    let c = pager.allocate(1, PageKind::VisibilityMap, self.owner)?;
                    pager.update(&[c, at], |p| {
                        set_level(p[0], l - 1);
                        set_child(p[1], i, c);
                    })?;
                    c
                }
            };
        }
    }

    /// Returns every page id whose bit is set, in order.
    pub fn marked<R: Runtime>(&self, pager: &mut Pager<R>) -> Result<Vec<Pid>, PageError> {
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
