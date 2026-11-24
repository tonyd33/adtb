//! A tree built whole from sorted runs of entries, and the retirement of the pages of a tree a
//! rebuild replaced.

use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};
use std::marker::PhantomData;

use adtb_sys::lock::{Ranked, RankedGuard};

use crate::ranks;
use crate::runtime::{Runtime, Threads};
use crate::steps::{Retire, Step};
use crate::storage::{Bitmapped, Damage, PageError, Pager, Pid, Quarantine};

use super::node::{self, Bounds, FORMAT, Item, NodeMut, kind_of};
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

/// Why a build ended before its tree was whole.
#[derive(Debug, PartialEq, Eq)]
pub enum BuildError {
    /// The build's `stop` returned true.
    Stopped,
    Page(PageError),
    /// The key of two different entries of a unique merge that both count.
    Duplicate(Vec<u8>),
    /// An entry of a bulk build not above the entry before it.
    Unsorted(Vec<u8>),
}

impl From<PageError> for BuildError {
    fn from(e: PageError) -> BuildError {
        BuildError::Page(e)
    }
}

/// Returns the entries of `runs`, each sorted, in order and each once.
///
/// Fail with `Duplicate` when `unique` and two different entries of one key both count, and with
/// `Stopped` once `stop` returns true.
pub fn merge_runs(
    runs: Vec<Vec<RunEntry>>,
    unique: bool,
    stop: &dyn Fn() -> bool,
) -> Result<Vec<Vec<u8>>, BuildError> {
    let mut runs: Vec<_> = runs.into_iter().map(Vec::into_iter).collect();
    let mut heads = BinaryHeap::new();
    for (i, run) in runs.iter_mut().enumerate() {
        if let Some(e) = run.next() {
            heads.push(Reverse((e, i)));
        }
    }
    let sorted = std::iter::from_fn(|| {
        let Reverse((e, i)) = heads.pop()?;
        if let Some(next) = runs[i].next() {
            heads.push(Reverse((next, i)));
        }
        Some(Ok(e))
    });
    Merged::new(sorted, unique, stop).collect()
}

/// The entries of runs merged in order, each once.
///
/// Its items fail with `Duplicate` when `unique` and two different entries of one key both
/// count, and with `Stopped` once `stop` returns true, which it is asked before each entry.
pub struct Merged<'s, I> {
    sorted: I,
    unique: bool,
    stop: &'s dyn Fn() -> bool,
    /// The entry of the current key that counts, if one has.
    counted: Option<Vec<u8>>,
    last: Option<Vec<u8>>,
}

impl<I> std::fmt::Debug for Merged<'_, I> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Merged")
            .field("unique", &self.unique)
            .finish_non_exhaustive()
    }
}

impl<'s, I> Merged<'s, I> {
    /// Returns the distinct entries of `sorted`, the entries of runs in order.
    pub fn new(sorted: I, unique: bool, stop: &'s dyn Fn() -> bool) -> Merged<'s, I> {
        Merged {
            sorted,
            unique,
            stop,
            counted: None,
            last: None,
        }
    }
}

impl<I, E> Iterator for Merged<'_, I>
where
    I: Iterator<Item = Result<RunEntry, E>>,
    E: From<BuildError>,
{
    type Item = Result<Vec<u8>, E>;

    fn next(&mut self) -> Option<Result<Vec<u8>, E>> {
        loop {
            let e = match self.sorted.next()? {
                Ok(e) => e,
                Err(err) => return Some(Err(err)),
            };
            if (self.stop)() {
                return Some(Err(BuildError::Stopped.into()));
            }
            let key = &e.entry[..e.key_len];
            if self.counted.as_ref().is_some_and(|c| !c.starts_with(key)) {
                self.counted = None;
            }
            if self.unique && e.counted {
                if self.counted.as_ref().is_some_and(|c| *c != e.entry) {
                    return Some(Err(BuildError::Duplicate(key.to_vec()).into()));
                }
                self.counted = Some(e.entry.clone());
            }
            if self.last.as_ref() != Some(&e.entry) {
                self.last = Some(e.entry.clone());
                return Some(Ok(e.entry));
            }
        }
    }
}

/// The trees of one index: its own, and its history tree when history is kept.
#[derive(Clone, Copy, Debug)]
pub struct IndexTrees {
    index: BTree,
    history: Option<BTree>,
}

impl IndexTrees {
    pub fn new(index: BTree, history: Option<BTree>) -> IndexTrees {
        IndexTrees { index, history }
    }
}

/// The latch of one index's inserts, through which alone its trees are taken over.
#[derive(Debug)]
pub struct IndexLatch(Ranked<IndexTrees>);

/// The guard of an [`IndexLatch`].
#[derive(Debug)]
pub struct IndexLatched<'a>(RankedGuard<'a, IndexTrees>);

/// A tree of an index whose latch is held.
#[derive(Clone, Copy, Debug)]
pub struct LatchedTree<'g> {
    tree: BTree,
    _held: PhantomData<&'g ()>,
}

impl IndexLatch {
    pub const fn new(trees: IndexTrees) -> IndexLatch {
        IndexLatch(Ranked::new(ranks::INDEX_LATCH, trees))
    }

    /// Wait for the latch as [`Ranked::lock`] does.
    pub fn lock(&self, threads: &impl Threads) -> IndexLatched<'_> {
        IndexLatched(self.0.lock(threads))
    }
}

impl IndexLatched<'_> {
    pub fn index(&self) -> LatchedTree<'_> {
        LatchedTree {
            tree: self.0.index,
            _held: PhantomData,
        }
    }

    pub fn history(&self) -> Option<LatchedTree<'_>> {
        Some(LatchedTree {
            tree: self.0.history?,
            _held: PhantomData,
        })
    }
}

impl LatchedTree<'_> {
    pub fn tree(&self) -> BTree {
        self.tree
    }

    /// Give the tree's root the contents of `built`'s root and free that page, removing the mark
    /// on `release` if one is named and held, in one batch. Returns once that is durable.
    ///
    /// Preconditions: nothing but the latch's holder writes the tree or `built`, and `built`'s
    /// pages carry the tree's owner.
    pub fn take_over<R: Runtime>(
        &self,
        pager: &Pager<R>,
        built: BTree,
        release: Option<Quarantine>,
    ) -> Result<(), PageError> {
        pager.replace_contents(self.tree.root(), built.root(), release)
    }
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

/// Write `items` as the pages of `level`, left to right, each logged once, holding at most a
/// page of them past the page being written. Returns each page's low key, empty for the
/// leftmost, and its id; every page written is added to `made`.
fn write_level<R: Runtime, E: From<BuildError>>(
    pager: &Pager<R>,
    owner: u32,
    level: u16,
    items: &mut dyn Iterator<Item = Result<Item, E>>,
    made: &mut Vec<Pid>,
    stop: &dyn Fn() -> bool,
) -> Result<Vec<(Vec<u8>, Pid)>, E> {
    let capacity = node::capacity(pager.page_size());
    let mut buf: Vec<Item> = Vec::new();
    let mut held = 0;
    let mut more = true;
    let mut out = Vec::new();
    let mut new = pager.reserve();
    let mut pid = new
        .page(FORMAT, kind_of(level), owner)
        .map_err(BuildError::from)?;
    let mut low: Option<Vec<u8>> = None;
    loop {
        if stop() {
            return Err(BuildError::Stopped.into());
        }
        while more && (held <= capacity || buf.len() < 2) {
            match items.next().transpose()? {
                Some(item) => {
                    held += item.cost();
                    buf.push(item);
                }
                None => more = false,
            }
        }
        let fence = low.as_ref().map_or(0, |l| node::item_cost(l.len(), level));
        let end = page_end(pager.page_size(), &buf, 0, fence, level);
        let high = (end < buf.len()).then(|| {
            if level == 0 {
                separator(&buf[end - 1].last(), &buf[end].first())
            } else {
                buf[end].first()
            }
        });
        let mut next = pager.reserve();
        let right = high
            .is_some()
            .then(|| next.page(FORMAT, kind_of(level), owner))
            .transpose()
            .map_err(BuildError::from)?;
        pager
            .update_with(new, &[pid], |p| {
                let bounds = Bounds {
                    low: low.as_deref(),
                    high: high.as_deref(),
                    right,
                };
                node::build(p[0], level, &buf[..end], bounds);
            })
            .map_err(BuildError::from)?;
        made.push(pid);
        out.push((low.take().unwrap_or_default(), pid));
        let Some(right) = right else {
            return Ok(out);
        };
        held -= buf.drain(..end).map(|i| i.cost()).sum::<usize>();
        new = next;
        pid = right;
        low = high;
    }
}

impl BTree {
    /// Returns a new tree of `entries`, sorted and distinct, whose pages carry `owner`, reading
    /// them as its leaves are written. Leaves are filled left to right and each level is built
    /// from the first keys of the level below.
    ///
    /// Every page written is added to `made`, a failed build's included. Fail with the first
    /// failure of `entries`, with `Stopped` once `stop` returns true, which it is asked before each
    /// page, and with `Unsorted` at an entry not above the one before it.
    pub fn bulk<R: Runtime, E: From<BuildError>>(
        pager: &Pager<R>,
        owner: u32,
        entries: impl IntoIterator<Item = Result<Vec<u8>, E>>,
        made: &mut Vec<Pid>,
        stop: &dyn Fn() -> bool,
    ) -> Result<BTree, E> {
        let mut before: Option<Vec<u8>> = None;
        let mut leaves = entries.into_iter().map(|e| -> Result<Item, E> {
            let e = e?;
            if before.as_ref().is_some_and(|b| e <= *b) {
                return Err(BuildError::Unsorted(e).into());
            }
            before = Some(e.clone());
            Ok(Item::entry(e))
        });
        let mut level = 0;
        let mut pages = write_level(pager, owner, level, &mut leaves, made, stop)?;
        while pages.len() > 1 {
            level += 1;
            let mut links = pages
                .into_iter()
                .map(|(low, pid)| Ok::<_, E>(Item::link(low, pid)));
            pages = write_level(pager, owner, level, &mut links, made, stop)?;
        }
        Ok(BTree::open(pages[0].1, owner))
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
/// A page that is not a node is freed unmarked.
///
/// Precondition: nothing writes the pages.
pub fn retire<R: Runtime>(pager: &Pager<R>, pages: &[Pid]) -> Result<(), PageError> {
    let mut sound = Vec::with_capacity(pages.len());
    for &pid in pages {
        match pager.bitmapped(pid) {
            Bitmapped::Free => continue,
            Bitmapped::Unknown(e) => return Err(e),
            Bitmapped::Allocated => {}
        }
        let mark = |p: &mut [&mut [u8]]| {
            if let Ok(mut n) = NodeMut::new(p[0]) {
                n.set_deleted();
            }
        };
        match pager.update(&[pid], mark) {
            Ok(_) => sound.push(pid),
            Err(e) if e.pid == pid && e.damage != Damage::Unreadable => pager.scrap(pid)?,
            Err(e) => return Err(e),
        }
    }
    pager.steps().at(Step::Retire(Retire::Deleted));
    for pid in sound {
        pager.free(pid)?;
        pager.steps().at(Step::Retire(Retire::Freed));
    }
    Ok(())
}
