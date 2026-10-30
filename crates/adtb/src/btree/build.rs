//! A tree built whole from sorted runs of entries, and the retirement of the pages of a tree a
//! rebuild replaced.

use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};

use crate::runtime::Runtime;
use crate::storage::{Damage, PageError, Pager, Pid, Quarantine};

use super::node::{self, Bounds, FORMAT, Item, kind_of};
use super::tree::{BTree, Error, node_of, separator, visit};

/// One entry of a run.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RunEntry {
    pub entry: Vec<u8>,
    /// The length of the entry's key, which no other key has as a proper prefix.
    pub key_len: usize,
    /// Whether the entry counts against a unique index: two entries of one key that both count
    /// are a violation.
    pub counted: bool,
}

/// Returns the entries of `runs`, each sorted, in order and each once.
///
/// Fail with the key of two different entries that both count when `unique`.
pub fn merge_runs(runs: Vec<Vec<RunEntry>>, unique: bool) -> Result<Vec<Vec<u8>>, Vec<u8>> {
    let mut runs: Vec<_> = runs.into_iter().map(Vec::into_iter).collect();
    let mut heads = BinaryHeap::new();
    for (i, run) in runs.iter_mut().enumerate() {
        if let Some(e) = run.next() {
            heads.push(Reverse((e, i)));
        }
    }
    let mut out: Vec<Vec<u8>> = Vec::new();
    let mut counted: Option<Vec<u8>> = None;
    while let Some(Reverse((e, i))) = heads.pop() {
        if let Some(next) = runs[i].next() {
            heads.push(Reverse((next, i)));
        }
        let key = &e.entry[..e.key_len];
        if counted.as_ref().is_some_and(|c| !c.starts_with(key)) {
            counted = None;
        }
        if unique && e.counted {
            if counted.as_ref().is_some_and(|c| *c != e.entry) {
                return Err(key.to_vec());
            }
            counted = Some(e.entry.clone());
        }
        if out.last() != Some(&e.entry) {
            out.push(e.entry);
        }
    }
    Ok(out)
}

/// The share of a page's capacity a build fills, leaving room for inserts.
const FILL_PERCENT: usize = 90;

/// Returns the end of the page that starts at `start` among `items`, and the fence below it.
fn page_end(page_size: usize, items: &[Item], start: usize, low: usize, level: u16) -> usize {
    let capacity = node::capacity(page_size);
    let fill = capacity * FILL_PERCENT / 100;
    let mut cost = 0;
    let mut end = start;
    while end < items.len() {
        let added = cost + items[end].cost();
        let high = items
            .get(end + 1)
            .map_or(0, |n| node::item_cost(n.first().len(), level));
        if end > start && (added > fill || added + low + high > capacity) {
            break;
        }
        cost = added;
        end += 1;
    }
    end
}

/// Write `items` as the pages of `level`, left to right, each logged once. Returns each page's
/// low key, empty for the leftmost, and its id; every page written is added to `made`.
fn write_level<R: Runtime>(
    pager: &Pager<R>,
    owner: u32,
    level: u16,
    items: &[Item],
    made: &mut Vec<Pid>,
) -> Result<Vec<(Vec<u8>, Pid)>, PageError> {
    let mut out = Vec::new();
    let mut new = pager.reserve();
    let mut pid = new.page(FORMAT, kind_of(level), owner)?;
    let mut start = 0;
    let mut low: Option<Vec<u8>> = None;
    loop {
        let fence = low.as_ref().map_or(0, |l| node::item_cost(l.len(), level));
        let end = page_end(pager.page_size(), items, start, fence, level);
        let high = (end < items.len()).then(|| {
            if level == 0 {
                separator(&items[end - 1].last(), &items[end].first())
            } else {
                items[end].first()
            }
        });
        let mut next = pager.reserve();
        let right = high
            .is_some()
            .then(|| next.page(FORMAT, kind_of(level), owner))
            .transpose()?;
        pager.update_with(new, &[pid], |p| {
            let bounds = Bounds {
                low: low.as_deref(),
                high: high.as_deref(),
                right,
            };
            node::build(p[0], level, &items[start..end], bounds);
        })?;
        made.push(pid);
        out.push((low.take().unwrap_or_default(), pid));
        let Some(right) = right else {
            return Ok(out);
        };
        new = next;
        pid = right;
        start = end;
        low = high;
    }
}

impl BTree {
    /// Returns a new tree of `entries`, sorted and distinct, whose pages carry `owner`. Leaves are
    /// filled left to right and each level is built from the first keys of the level below.
    ///
    /// Every page written is added to `made`, a failed build's included.
    pub fn bulk<R: Runtime>(
        pager: &Pager<R>,
        owner: u32,
        entries: Vec<Vec<u8>>,
        made: &mut Vec<Pid>,
    ) -> Result<BTree, PageError> {
        let items: Vec<Item> = entries.into_iter().map(Item::entry).collect();
        let mut level = 0;
        let mut pages = write_level(pager, owner, level, &items, made)?;
        while pages.len() > 1 {
            level += 1;
            let links: Vec<Item> = pages
                .into_iter()
                .map(|(low, pid)| Item::link(low, pid))
                .collect();
            pages = write_level(pager, owner, level, &links, made)?;
        }
        Ok(BTree::open(pages[0].1, owner))
    }

    /// Give this tree's root the contents of `built`'s root and free that page, removing the mark
    /// on `release` if one is named and held, in one batch. Returns once that is durable.
    ///
    /// Preconditions: nothing writes this tree or `built`, and `built`'s pages carry this tree's
    /// owner.
    pub fn take_over<R: Runtime>(
        &self,
        pager: &Pager<R>,
        built: BTree,
        release: Option<Quarantine>,
    ) -> Result<(), PageError> {
        pager.replace_contents(self.root(), built.root(), release)
    }

    /// Returns the pages below the root as far as they can be followed, each level left to right
    /// from the top, a damaged one included, and whether every one was reached.
    pub fn reachable<R: Runtime>(&self, pager: &Pager<R>) -> (Vec<Pid>, bool) {
        let mut out = Vec::new();
        let mut seen = BTreeSet::from([self.root()]);
        let mut complete = true;
        let mut leftmost = Some(self.root());
        while let Some(start) = leftmost.take() {
            let mut at = Some(start);
            while let Some(pid) = at.take() {
                if pid != self.root() {
                    out.push(pid);
                }
                match visit(pager, self.owner(), pid, |n| {
                    let down = if n.is_leaf() { None } else { Some(n.child(0)?) };
                    Ok((n.right(), down))
                }) {
                    Ok((right, down)) => {
                        if pid == start {
                            leftmost = down.filter(|d| seen.insert(*d));
                        }
                        at = right.filter(|r| seen.insert(*r));
                        complete &= right.is_none() || at.is_some();
                    }
                    Err(e) => {
                        complete = false;
                        if !matches!(e, Error::Page(p) if p.pid == pid && p.damage != Damage::Unreadable)
                        {
                            out.pop();
                        }
                    }
                }
            }
        }
        (out, complete)
    }

    /// Returns the allocated pages that are nodes of this tree's owner and not reached from its
    /// root, and whether every allocated page could be judged.
    ///
    /// Precondition: nothing writes this tree.
    pub fn strays<R: Runtime>(&self, pager: &Pager<R>) -> Result<(Vec<Pid>, bool), PageError> {
        let (reached, mut complete) = self.reachable(pager);
        let reached: BTreeSet<Pid> = reached.into_iter().collect();
        let mut out = Vec::new();
        for pid in pager.allocated()? {
            if pid == self.root() || reached.contains(&pid) {
                continue;
            }
            match pager.read(pid) {
                Ok(page) => {
                    if node_of(&page, self.owner()).is_ok() {
                        out.push(pid);
                    }
                }
                Err(_) => complete = false,
            }
        }
        Ok((out, complete))
    }
}

/// Mark `pages`, the old pages of a replaced tree in the order [`BTree::reachable`] gives them,
/// deleted and free them.
///
/// A page is marked after the pages leading to it, so a reader still in the old tree that finds a
/// page deleted or reused finds the page that led to it changed, and starts again from the root.
///
/// Precondition: nothing writes the pages.
pub fn retire<R: Runtime>(pager: &Pager<R>, pages: &[Pid]) -> Result<(), PageError> {
    let mut sound = Vec::with_capacity(pages.len());
    for &pid in pages {
        if pager.is_free(pid)? {
            continue;
        }
        match pager.update(&[pid], |p| node::set_deleted(p[0])) {
            Ok(_) => sound.push(pid),
            Err(e) if e.pid == pid && e.damage != Damage::Unreadable => pager.scrap(pid)?,
            Err(e) => return Err(e),
        }
    }
    for pid in sound {
        pager.free(pid)?;
    }
    Ok(())
}
