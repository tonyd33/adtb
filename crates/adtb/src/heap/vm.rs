use crate::runtime::Runtime;
use crate::storage::{self, Damage, PageError, PageKind, Pager, Pid};

use super::page::{CONTENTS, Owned, PageCopy, u16_at, u64_at};
use super::stage::Stage;
use super::write::Rank;

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

/// Returns whether `node` is a map page of `owner`, one level below `parent` when it has one.
pub(super) fn shaped(node: &impl Owned, owner: u32, parent: Option<u32>) -> bool {
    storage::kind(node) == PageKind::VisibilityMap as u16
        && node.owner() == owner
        && parent.is_none_or(|l| level(node) + 1 == l)
}

/// Returns every page of the map rooted at `root`, the root first, each read by `read` with the
/// page naming it and that page's level, `None` for the root.
pub(super) fn walk<E>(
    root: Pid,
    mut read: impl FnMut(Pid, Option<(Pid, u32)>) -> Result<Vec<u8>, E>,
) -> Result<Vec<Pid>, E> {
    let mut out = Vec::new();
    let mut todo = vec![(root, None)];
    while let Some((at, parent)) = todo.pop() {
        let node = read(at, parent)?;
        out.push(at);
        let l = level(&node);
        if l > 0 {
            let children = (0..fanout(node.len()) as usize).filter_map(|i| child(&node, i));
            todo.extend(children.rev().map(|c| (c, Some((at, l)))));
        }
    }
    Ok(out)
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

pub(super) fn bit(leaf: &[u8], i: u64) -> bool {
    leaf[BODY + (i / 8) as usize] >> (i % 8) & 1 == 1
}

/// Returns the index of every set bit of a leaf, in order.
pub(super) fn set_bits(leaf: &[u8]) -> impl Iterator<Item = u64> {
    leaf[BODY..].iter().enumerate().flat_map(|(at, &byte)| {
        std::iter::successors((byte != 0).then_some(byte), |rest| {
            Some(rest & (rest - 1)).filter(|&next| next != 0)
        })
        .map(move |rest| at as u64 * 8 + u64::from(rest.trailing_zeros()))
    })
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
    /// The directory root naming `root`.
    named_by: Pid,
}

impl Vm {
    /// Returns the root of a new empty map of `owner`.
    pub fn create<R: Runtime>(pager: &Pager<R>, owner: u32) -> Result<Pid, PageError> {
        pager.allocate(1, PageKind::VisibilityMap, owner)
    }

    pub fn open(root: Pid, owner: u32, named_by: Pid) -> Vm {
        Vm {
            root,
            owner,
            named_by,
        }
    }

    /// Returns the map's root as `stage` holds it, or as the pager does, read without latching
    /// it. Refuse the directory root naming it, as [`Vm::refuse`] does, when it is not a map page
    /// of the heap.
    fn top<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
    ) -> Result<PageCopy, PageError> {
        let node = stage.read(pager, Rank::Vm(self.root))?;
        if shaped(&node, self.owner, None) {
            return Ok(node);
        }
        Err(self.refuse(stage, pager, self.named_by))
    }

    /// Returns map page `at`, latched and staged in `stage`. Refuse the page naming it, as
    /// [`Vm::refuse`] does, when it is not a map page of the heap: the root at any level, any
    /// other page one level below `parent`, the map page naming it and its level.
    fn latched<'s, 'p, R: Runtime>(
        &self,
        stage: &'s mut Stage<'p, R>,
        pager: &'p Pager<R>,
        at: Pid,
        parent: Option<(Pid, u32)>,
    ) -> Result<&'s mut PageCopy, PageError> {
        let node = stage.page(pager, Rank::Vm(at))?;
        if !shaped(node, self.owner, parent.map(|(_, l)| l)) {
            let blamed = parent.map_or(self.named_by, |(p, _)| p);
            return Err(self.refuse(stage, pager, blamed));
        }
        stage.page(pager, Rank::Vm(at))
    }

    /// Abandon the change `stage` holds, then record `blamed` as damaged.
    fn refuse<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        blamed: Pid,
    ) -> PageError {
        stage.abandon();
        pager.damage(blamed, Damage::Checksum)
    }

    /// Returns map page `pid` as `stage` holds it, or as the pager does, read without latching it.
    /// Refuse `parent`, the map page naming it, as [`Vm::refuse`] does when `pid` is not a map
    /// page of the heap one level below it.
    ///
    /// Precondition: the caller holds no latch on `parent` outside `stage`.
    fn child<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        pid: Pid,
        parent: (Pid, u32),
    ) -> Result<PageCopy, PageError> {
        let node = stage.read_any(pager, Rank::Vm(pid))?;
        if shaped(&node, self.owner, Some(parent.1)) {
            return Ok(node);
        }
        Err(self.refuse(stage, pager, parent.0))
    }

    /// Returns the leaf covering `pid`, if one exists, as `stage` holds the map.
    fn leaf<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        pid: Pid,
    ) -> Result<Option<Pid>, PageError> {
        let size = pager.page_size();
        let mut node = self.top(stage, pager)?;
        if pid.0 >= reach(size, level(&node)) {
            return Ok(None);
        }
        let mut at = self.root;
        while level(&node) > 0 {
            let Some(c) = child(&node, child_index(size, level(&node), pid)) else {
                return Ok(None);
            };
            node = self.child(stage, pager, c, (at, level(&node)))?;
            at = c;
        }
        Ok(Some(at))
    }

    /// Returns whether the bit of `pid` is set.
    pub fn get<'p, R: Runtime>(
        &self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
        pid: Pid,
    ) -> Result<bool, PageError> {
        let Some(leaf) = self.leaf(stage, pager, pid)? else {
            return Ok(false);
        };
        let i = bit_index(pager.page_size(), pid);
        Ok(bit(&stage.read(pager, Rank::Vm(leaf))?, i))
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
            let node = stage.page(pager, Rank::Vm(leaf))?;
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

    /// Add the map pages the path to `pid` lacks, each under the latch of the page that links
    /// it and in an update of its own.
    pub fn extend<R: Runtime>(&self, pager: &Pager<R>, pid: Pid) -> Result<(), PageError> {
        let size = pager.page_size();
        loop {
            let mut stage = Stage::new();
            let root = self.latched(&mut stage, pager, self.root, None)?;
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
            stage.put_reserved(&new, moved, new.image(moved));
            stage.page(pager, Rank::Vm(moved))?[CONTENTS..].copy_from_slice(&contents);
            stage.commit_with(pager, new)?;
        }
        let (mut at, mut parent) = (self.root, None);
        loop {
            let mut stage = Stage::new();
            let node = self.latched(&mut stage, pager, at, parent)?;
            let l = level(node);
            if l == 0 {
                return Ok(());
            }
            let i = child_index(size, l, pid);
            parent = Some((at, l));
            if let Some(c) = child(node, i) {
                at = c;
                continue;
            }
            let mut new = pager.reserve();
            let c = new.page(1, PageKind::VisibilityMap, self.owner)?;
            set_child(node, i, c);
            stage.put_reserved(&new, c, new.image(c));
            set_level(stage.page(pager, Rank::Vm(c))?, l - 1);
            stage.commit_with(pager, new)?;
            at = c;
        }
    }

    /// Returns every page id whose bit is set, in order.
    pub fn marked<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pid>, PageError> {
        let size = pager.page_size();
        let mut out = Vec::new();
        let mut todo = vec![(self.root, 0, None)];
        while let Some((at, base, parent)) = todo.pop() {
            let node = match parent {
                None => self.top(&mut Stage::new(), pager)?.into_bytes(),
                Some(parent) => self
                    .child(&mut Stage::new(), pager, at, parent)?
                    .into_bytes(),
            };
            let l = level(&node);
            if l == 0 {
                let set: u32 = node[BODY..].iter().map(|b| b.count_ones()).sum();
                out.reserve(set as usize);
                out.extend(set_bits(&node).map(|i| Pid(base + i)));
                continue;
            }
            let below = reach(size, l - 1);
            let children: Vec<_> = (0..fanout(size) as usize)
                .filter_map(|i| {
                    child(&node, i).map(|c| (c, base + i as u64 * below, Some((at, l))))
                })
                .collect();
            todo.extend(children.into_iter().rev());
        }
        Ok(out)
    }
}
