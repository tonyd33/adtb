use std::collections::VecDeque;
use std::ops::Bound;

use crate::key::DecodeError;
use crate::runtime::Runtime;
use crate::storage::{PageError, PageExclusive, PageOptimistic, Pager, Pid};

use super::leaf;
use super::node::{self, Bounds, FORMAT, Item, Malformed, Node, kind_of};

/// A B-link tree of byte strings ordered by `memcmp`, rooted at a page that
/// never moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BTree {
    root: Pid,
    owner: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Page(PageError),
    /// A page of the tree is not a node of it, or breaks the tree's order.
    Malformed(Pid),
    /// An entry is longer than a page allows.
    TooLong {
        len: usize,
        max: usize,
    },
    /// A stored entry does not decode under its index's key schema.
    Entry(DecodeError),
}

impl From<PageError> for Error {
    fn from(e: PageError) -> Error {
        Error::Page(e)
    }
}

/// What an inserter knows of a tree's entries, consulted when a leaf fills.
pub trait Entries {
    /// Returns whether `entry` may be deleted to make room.
    fn dead(&mut self, entry: &[u8]) -> bool;

    /// Returns the length of `entry`'s key, or `None` to keep the entry
    /// whole. Entries sharing a key are kept once, followed by their
    /// remainders.
    ///
    /// No key is a proper prefix of another.
    fn key_len(&self, entry: &[u8]) -> Option<usize>;
}

/// Entries kept whole, none dead.
struct Whole;

impl Entries for Whole {
    fn dead(&mut self, _: &[u8]) -> bool {
        false
    }

    fn key_len(&self, _: &[u8]) -> Option<usize> {
        None
    }
}

/// Which keys a descent seeks: the key itself, or those just below it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    At,
    Before,
}

enum Step {
    Right(Pid, Vec<u8>),
    Down(Pid),
    Here,
}

/// The page at a level whose range held a key when it was read, and the page
/// passed at each level above it, root first.
pub(super) struct Descent {
    pub(super) pid: Pid,
    pub(super) stack: Vec<Pid>,
    /// The first page passed by a right link whose downlink was not in its
    /// parent.
    pub(super) incomplete: Option<Split>,
}

/// A page whose downlink is not in its parent, its level and low key, and the
/// page passed at each level above it on the way to it.
pub(super) struct Split {
    pub(super) pid: Pid,
    pub(super) level: u16,
    pub(super) low: Vec<u8>,
    pub(super) stack: Vec<Pid>,
}

/// The optimistic reads of one page tried before a read takes its shared
/// latch.
const OPTIMISTIC_TRIES: usize = 4;

pub(super) fn node_of(page: &[u8], owner: u32) -> Result<Node<'_>, Malformed> {
    Node::new(page)
        .ok()
        .filter(|n| n.owner() == owner)
        .ok_or(Malformed)
}

/// Returns what `f` makes of a validated read of node `pid`, and an
/// optimistic guard taken no later than the read, whose validation shows the
/// page unchanged since.
///
/// `f` sees a copy taken by an optimistic read; after a few failed
/// validations it sees the page under a shared latch.
fn observe<'p, R: Runtime, T>(
    pager: &'p Pager<R>,
    owner: u32,
    pid: Pid,
    f: impl Fn(&Node) -> Result<T, Malformed>,
) -> Result<(Result<T, Malformed>, PageOptimistic<'p, R>), PageError> {
    let apply = |page: &[u8]| node_of(page, owner).and_then(|n| f(&n));
    for _ in 0..OPTIMISTIC_TRIES {
        let read = pager.optimistic(pid)?;
        let result = read.read(apply);
        if read.validate() {
            return Ok((result, read));
        }
    }
    let page = pager.shared(pid)?;
    let read = pager.optimistic(pid)?;
    Ok((apply(&page), read))
}

/// Returns what `f` makes of node `pid`, refusing a page that is not a node
/// of `owner`.
pub(super) fn visit<R: Runtime, T>(
    pager: &Pager<R>,
    owner: u32,
    pid: Pid,
    f: impl Fn(&Node) -> Result<T, Malformed>,
) -> Result<T, Error> {
    observe(pager, owner, pid, f)?
        .0
        .map_err(|Malformed| Error::Malformed(pid))
}

/// Returns the node of a page under a latch, refusing a page that is not a
/// node of `owner`.
fn latched<'a>(page: &'a [u8], pid: Pid, owner: u32) -> Result<Node<'a>, Error> {
    node_of(page, owner).map_err(|Malformed| Error::Malformed(pid))
}

/// Returns whether `n` is a live page at `level` whose low key is at or below
/// `key`.
fn holds_from(n: &Node, key: &[u8], level: u16) -> Result<bool, Malformed> {
    Ok(!n.deleted() && n.level() == level && n.above_low(key)?)
}

/// Clear the mark of the child page at `at` in `pages`, if there is one.
fn complete(pages: &mut [&mut [u8]], at: usize) {
    if let Some(child) = pages.get_mut(at) {
        node::set_incomplete(child, false);
    }
}

/// Returns the shortest prefix of `right` above `left`.
///
/// Preconditions: `left` is below `right`.
pub(super) fn separator(left: &[u8], right: &[u8]) -> Vec<u8> {
    let common = left.iter().zip(right).take_while(|(a, b)| a == b).count();
    right[..(common + 1).min(right.len())].to_vec()
}

/// Returns the slot that splits `items` into two halves, each holding at
/// least one item: of about equal bytes, or, when the new item at slot `at`
/// lands past every other on the rightmost page of its level, just before
/// the item it follows.
fn split_point(items: &[Item], at: usize, rightmost: bool) -> usize {
    if rightmost && at == items.len() - 1 && at >= 2 {
        return at - 1;
    }
    let total: usize = items.iter().map(Item::cost).sum();
    let mut left = 0;
    let mut s = 0;
    while s < items.len() && left * 2 < total {
        left += items[s].cost();
        s += 1;
    }
    s.clamp(1, items.len() - 1)
}

impl BTree {
    /// Allocate an empty tree whose pages carry `owner`.
    pub fn create<R: Runtime>(pager: &Pager<R>, owner: u32) -> Result<BTree, PageError> {
        let root = pager.allocate(FORMAT, kind_of(0), owner)?;
        Ok(BTree { root, owner })
    }

    pub fn open(root: Pid, owner: u32) -> BTree {
        BTree { root, owner }
    }

    pub fn root(&self) -> Pid {
        self.root
    }

    pub fn owner(&self) -> u32 {
        self.owner
    }

    /// Returns the root's level, which is the number of levels above the
    /// leaves.
    pub fn height<R: Runtime>(&self, pager: &Pager<R>) -> Result<u16, Error> {
        visit(pager, self.owner, self.root, |n| Ok(n.level()))
    }

    /// Returns the longest entry a tree over pages of `page_size` bytes holds.
    pub fn max_entry_len(page_size: usize) -> usize {
        node::capacity(page_size) / 5 - node::item_cost(0, 1)
    }

    /// Returns the descent from page `from` to the page at `level` whose
    /// range holds `key`.
    ///
    /// Each page is read on its own and checked to be a live page of the
    /// tree, at the level expected, whose low key is at or below `key`. A
    /// page that fails the check while the page that led to it is unchanged
    /// is malformed; otherwise the id that led to it was stale, and the
    /// descent starts again from the root.
    pub(super) fn descend<R: Runtime>(
        &self,
        pager: &Pager<R>,
        from: Pid,
        key: &[u8],
        level: u16,
    ) -> Result<Descent, Error> {
        Ok(self
            .descend_to(pager, from, key, level, Side::At, |_| Ok(()))?
            .0)
    }

    /// Returns the page at `level` whose range holds the keys just below
    /// `key`: the one whose high key is `key`, if a page's low key is.
    fn descend_before<R: Runtime>(
        &self,
        pager: &Pager<R>,
        key: &[u8],
        level: u16,
    ) -> Result<Pid, Error> {
        Ok(self
            .descend_to(pager, self.root, key, level, Side::Before, |_| Ok(()))?
            .0
            .pid)
    }

    /// Returns the descent [`BTree::descend`] makes, or for `Side::Before`
    /// the one to the keys just below `key`, and what `f` makes of the page
    /// it ends at, from the same read.
    fn descend_to<R: Runtime, T>(
        &self,
        pager: &Pager<R>,
        from: Pid,
        key: &[u8],
        level: u16,
        side: Side,
        f: impl Fn(&Node) -> Result<T, Malformed>,
    ) -> Result<(Descent, T), Error> {
        let mut start = from;
        'descent: loop {
            let mut pid = start;
            let mut expected: Option<u16> = None;
            let mut stack = Vec::new();
            let mut low: Option<Vec<u8>> = None;
            let mut incomplete = None;
            let mut came_from: Option<PageOptimistic<'_, R>> = None;
            loop {
                let read = observe(pager, self.owner, pid, |n| {
                    let at = n.level();
                    let (from_low, past_high) = match side {
                        Side::At => (n.above_low(key)?, n.beyond(key)?),
                        Side::Before => (
                            n.low()?.is_none_or(|l| l < key),
                            n.high()?.is_some_and(|h| h < key),
                        ),
                    };
                    if n.deleted() || at < level || expected.is_some_and(|e| e != at) || !from_low {
                        return Ok(None);
                    }
                    let mut found = None;
                    let step = if past_high {
                        let high = n.high()?.ok_or(Malformed)?.to_vec();
                        Step::Right(n.right().ok_or(Malformed)?, high)
                    } else if at == level {
                        found = Some(f(n)?);
                        Step::Here
                    } else {
                        let child = match side {
                            Side::At => n.child_for(key)?,
                            Side::Before => {
                                n.child(n.search(key, true)?.checked_sub(1).ok_or(Malformed)?)?
                            }
                        };
                        Step::Down(child)
                    };
                    Ok(Some((step, n.incomplete(), at, found)))
                });
                let (step, marked, at, found, guard) = match read {
                    Ok((Ok(Some((step, marked, at, found))), guard)) => {
                        (step, marked, at, found, guard)
                    }
                    Ok((_, _)) | Err(_) if came_from.as_ref().is_some_and(|g| !g.validate()) => {
                        start = self.root;
                        continue 'descent;
                    }
                    Ok((_, _)) if came_from.is_none() && pid != self.root => {
                        start = self.root;
                        continue 'descent;
                    }
                    Ok((_, _)) => return Err(Error::Malformed(pid)),
                    Err(e) => return Err(Error::Page(e)),
                };
                if marked
                    && incomplete.is_none()
                    && let Some(low) = &low
                {
                    incomplete = Some(Split {
                        pid,
                        level: at,
                        low: low.clone(),
                        stack: stack.clone(),
                    });
                }
                came_from = Some(guard);
                match step {
                    Step::Right(p, high) => {
                        pid = p;
                        expected = Some(at);
                        low = Some(high);
                    }
                    Step::Down(c) => {
                        stack.push(pid);
                        pid = c;
                        expected = Some(at - 1);
                        low = None;
                    }
                    Step::Here => {
                        let descent = Descent {
                            pid,
                            stack,
                            incomplete,
                        };
                        return Ok((
                            descent,
                            found.expect("the page a descent ends at is judged"),
                        ));
                    }
                }
            }
        }
    }

    /// Returns the exclusive latch on the page at `level` whose range holds
    /// `key`, found from page `from`, which is at or above that level, and
    /// the first page passed by a right link without a downlink.
    fn latch<'p, R: Runtime>(
        &self,
        pager: &'p Pager<R>,
        from: Pid,
        key: &[u8],
        level: u16,
    ) -> Result<(PageExclusive<'p, R>, Option<Split>), Error> {
        let descent = self.descend(pager, from, key, level)?;
        let (g, passed) = self.claim(pager, descent.pid, key, level)?;
        Ok((g, descent.incomplete.or(passed)))
    }

    /// Returns the exclusive latch on the page at `level` whose range holds
    /// `key`, starting at page `pid`, and the first page passed by a right
    /// link without a downlink.
    ///
    /// Latches are taken one at a time, moving right along the level; a page
    /// found unlinked, or no longer at or left of `key`, sends the search
    /// back to the root.
    fn claim<'p, R: Runtime>(
        &self,
        pager: &'p Pager<R>,
        mut pid: Pid,
        key: &[u8],
        level: u16,
    ) -> Result<(PageExclusive<'p, R>, Option<Split>), Error> {
        let mut passed = None;
        let mut low: Option<Vec<u8>> = None;
        loop {
            let g = pager.exclusive(pid)?;
            let next = match node_of(&g, self.owner) {
                Ok(n) if holds_from(&n, key, level).unwrap_or(false) => {
                    if n.incomplete()
                        && passed.is_none()
                        && let Some(low) = low.take()
                    {
                        passed = Some(Split {
                            pid,
                            level,
                            low,
                            stack: Vec::new(),
                        });
                    }
                    match n.beyond(key) {
                        Ok(false) => return Ok((g, passed)),
                        Ok(true) => match (n.right(), n.high()) {
                            (Some(r), Ok(Some(h))) => Some((r, h.to_vec())),
                            _ => return Err(Error::Malformed(pid)),
                        },
                        Err(Malformed) => return Err(Error::Malformed(pid)),
                    }
                }
                _ => None,
            };
            drop(g);
            match next {
                Some((right, high)) => {
                    pid = right;
                    low = Some(high);
                }
                None => {
                    pid = self.descend(pager, self.root, key, level)?.pid;
                    low = None;
                }
            }
        }
    }

    /// Returns what `f` makes of the leaf whose range holds `key`, read on
    /// its own and checked to hold it.
    fn at_leaf<R: Runtime, T>(
        &self,
        pager: &Pager<R>,
        key: &[u8],
        f: impl Fn(&Node) -> Result<T, Malformed>,
    ) -> Result<T, Error> {
        Ok(self.descend_to(pager, self.root, key, 0, Side::At, f)?.1)
    }

    pub fn contains<R: Runtime>(&self, pager: &Pager<R>, entry: &[u8]) -> Result<bool, Error> {
        self.at_leaf(pager, entry, |n| leaf::contains(n, entry))
    }

    /// Insert `entry`, first finishing each split whose page the descent
    /// reaches by a right link without a downlink. Returns false, changing no
    /// entry, if it is already in the tree.
    pub fn insert<R: Runtime>(&self, pager: &Pager<R>, entry: &[u8]) -> Result<bool, Error> {
        self.insert_with(pager, entry, &mut Whole)
    }

    /// Insert `entry` as [`BTree::insert`] does. A leaf that would otherwise
    /// split first loses every entry `entries` judges dead, and keeps each run
    /// of entries sharing a key once, as a posting of their remainders; it
    /// splits only if that leaves too little room.
    ///
    /// `entries` is consulted under the leaf's exclusive latch.
    pub fn insert_with<R: Runtime>(
        &self,
        pager: &Pager<R>,
        entry: &[u8],
        entries: &mut impl Entries,
    ) -> Result<bool, Error> {
        let max = BTree::max_entry_len(pager.page_size());
        if entry.len() > max {
            return Err(Error::TooLong {
                len: entry.len(),
                max,
            });
        }
        loop {
            let descent = self.descend(pager, self.root, entry, 0)?;
            if let Some(split) = descent.incomplete {
                self.finish(pager, split)?;
                continue;
            }
            let (leaf, passed) = self.claim(pager, descent.pid, entry, 0)?;
            let pid = leaf.pid();
            let n = latched(&leaf, pid, self.owner)?;
            if leaf::contains(&n, entry).map_err(|Malformed| Error::Malformed(pid))? {
                drop(leaf);
                self.finish_all(pager, passed)?;
                return Ok(false);
            }
            self.add_to_leaf(pager, leaf, entry, entries, descent.stack)?;
            self.finish_all(pager, passed)?;
            return Ok(true);
        }
    }

    /// Add `entry`, which leaf `g` lacks and whose range holds it, cleaning
    /// and packing the leaf if it is full, and splitting it up the levels of
    /// `stack` if that is not enough.
    fn add_to_leaf<'p, R: Runtime>(
        &self,
        pager: &'p Pager<R>,
        mut g: PageExclusive<'p, R>,
        entry: &[u8],
        entries: &mut impl Entries,
        mut stack: Vec<Pid>,
    ) -> Result<(), Error> {
        let mut done = false;
        g.update(|p| done = leaf::insert(p, entry));
        if done {
            return Ok(());
        }
        let pid = g.pid();
        let malformed = |Malformed| Error::Malformed(pid);
        let cap = leaf::posting_cap(pager.page_size());
        let items = latched(&g, pid, self.owner)?.items().map_err(malformed)?;
        let (kept, _) = leaf::retain(&items, |e| !entries.dead(e));
        let packed = leaf::dedup(&kept, &|e| entries.key_len(e), cap);
        let packed = if node::fits(&g, &packed) {
            packed
        } else {
            kept
        };
        if packed != items {
            g.update(|p| {
                node::rebuild(p, &packed);
                done = leaf::insert(p, entry);
            });
            if done {
                return Ok(());
            }
        }
        let mut items = latched(&g, pid, self.owner)?.items().map_err(malformed)?;
        let at = leaf::add(&mut items, entry, cap).index();
        if let Some((sep, q)) = self.split_page(pager, &mut g, 0, &items, at, None)? {
            drop(g);
            let from = stack.pop().unwrap_or(self.root);
            let (parent, passed) = self.latch(pager, from, &sep, 1)?;
            self.put(pager, parent, 1, stack, Item::link(sep, q))?;
            self.finish_all(pager, passed)?;
        }
        Ok(())
    }

    /// Finish `split`, if there is one.
    fn finish_all<R: Runtime>(&self, pager: &Pager<R>, split: Option<Split>) -> Result<(), Error> {
        if let Some(s) = split {
            self.finish(pager, s)?;
        }
        Ok(())
    }

    /// Insert the downlink of `split`'s page into its parent's level. Returns
    /// false if another thread had.
    pub(super) fn finish<R: Runtime>(&self, pager: &Pager<R>, split: Split) -> Result<bool, Error> {
        let mut stack = split.stack;
        let from = stack.pop().unwrap_or(self.root);
        let level = split.level + 1;
        let (parent, passed) = self.latch(pager, from, &split.low, level)?;
        let added = self.put(
            pager,
            parent,
            level,
            stack,
            Item::link(split.low, split.pid),
        )?;
        self.finish_all(pager, passed)?;
        Ok(added)
    }

    /// Delete `entry`. Returns false if it was not in the tree.
    ///
    /// A leaf the delete empties is unlinked and freed when it has a left
    /// sibling under the same parent and is not the rightmost of its level.
    pub fn delete<R: Runtime>(&self, pager: &Pager<R>, entry: &[u8]) -> Result<bool, Error> {
        let leaf = self.descend(pager, self.root, entry, 0)?.pid;
        let (mut g, passed) = self.claim(pager, leaf, entry, 0)?;
        let pid = g.pid();
        let malformed = |Malformed| Error::Malformed(pid);
        let n = latched(&g, pid, self.owner)?;
        let place = leaf::find(&n, entry).map_err(malformed)?;
        if !place.found {
            drop(g);
            self.finish_all(pager, passed)?;
            return Ok(false);
        }
        let last = leaf::len(&n).map_err(malformed)? == 1;
        let emptied = last.then(|| n.low().map(|l| l.map(<[u8]>::to_vec)));
        g.update(|p| leaf::remove(p, place));
        drop(g);
        if let Some(low) = emptied.transpose().map_err(malformed)? {
            self.unlink(pager, low.as_deref())?;
        }
        self.finish_all(pager, passed)?;
        Ok(true)
    }

    /// Delete every entry `keep` refuses, walking the leaves left to right and changing each in
    /// one update. Returns the number of entries deleted.
    ///
    /// Each leaf is judged and changed under its exclusive latch, which is released before the
    /// next leaf's is taken; a leaf left empty is unlinked as [`BTree::delete`] unlinks one. An
    /// entry a concurrent writer inserts during the walk may be left unjudged.
    pub fn retain<R: Runtime>(
        &self,
        pager: &Pager<R>,
        mut keep: impl FnMut(&[u8]) -> bool,
    ) -> Result<usize, Error> {
        let mut from: Vec<u8> = Vec::new();
        let mut deleted = 0;
        loop {
            let leaf = self.descend(pager, self.root, &from, 0)?.pid;
            let (mut g, _) = self.claim(pager, leaf, &from, 0)?;
            let pid = g.pid();
            let malformed = |Malformed| Error::Malformed(pid);
            let n = latched(&g, pid, self.owner)?;
            let (kept, gone) = leaf::retain(&n.items().map_err(malformed)?, &mut keep);
            let high = n.high().map_err(malformed)?.map(<[u8]>::to_vec);
            let low = n.low().map_err(malformed)?.map(<[u8]>::to_vec);
            let emptied = kept.is_empty();
            if gone > 0 {
                g.update(|p| node::rebuild(p, &kept));
                deleted += gone;
            }
            drop(g);
            if emptied {
                self.unlink(pager, low.as_deref())?;
            }
            match high {
                Some(h) => from = h,
                None => return Ok(deleted),
            }
        }
    }

    /// Unlink the empty leaf whose low key is `low`, `None` for the leftmost,
    /// from its parent and its siblings, and free it, in one change. Returns
    /// whether it did.
    ///
    /// The leaf's range goes to its left sibling under the same parent,
    /// whose high key becomes the leaf's. If there is none, or it has no room
    /// for that key, the range goes to the right sibling, whose low key and
    /// separator in the parent become the leaf's; the left sibling, under
    /// this parent or the one before, then only takes the leaf's right link.
    /// Latches the parent, then the left sibling, then the leaf and its right
    /// sibling. Does nothing if the leaf is no longer empty, is the rightmost
    /// of its level, lacks its downlink, or is its parent's only child, or if
    /// neither sibling can take its range.
    fn unlink<R: Runtime>(&self, pager: &Pager<R>, low: Option<&[u8]>) -> Result<bool, Error> {
        if self.height(pager)? == 0 {
            return Ok(false);
        }
        let key = low.unwrap_or_default();
        let (mut parent, passed) = self.latch(pager, self.root, key, 1)?;
        let done = self.unlink_under(pager, &mut parent, low);
        drop(parent);
        self.finish_all(pager, passed)?;
        done
    }

    fn unlink_under<'p, R: Runtime>(
        &self,
        pager: &'p Pager<R>,
        parent: &mut PageExclusive<'p, R>,
        low: Option<&[u8]>,
    ) -> Result<bool, Error> {
        let key = low.unwrap_or_default();
        let ppid = parent.pid();
        let pm = |Malformed| Error::Malformed(ppid);
        let pn = latched(parent, ppid, self.owner)?;
        let j = pn.search(key, true).map_err(pm)?;
        if j >= pn.count() || pn.key(j).map_err(pm)? != key {
            return Ok(false);
        }
        if low.is_none() && pn.low().map_err(pm)?.is_some() {
            return Ok(false);
        }
        let lpid = pn.child(j).map_err(pm)?;
        let next_link = if j + 1 < pn.count() {
            Some((
                pn.child(j + 1).map_err(pm)?,
                pn.key(j + 1).map_err(pm)?.to_vec(),
            ))
        } else {
            None
        };
        let first = j == 0;
        let mut left = match low {
            None => None,
            Some(low) => match self.left_of(pager, (ppid, &pn), j, lpid, low)? {
                Some(x) => Some(x),
                None => return Ok(false),
            },
        };
        let mut leaf = pager.exclusive(lpid)?;
        let ln = latched(&leaf, lpid, self.owner)?;
        let lf = node::fences(&ln).map_err(|Malformed| Error::Malformed(lpid))?;
        let (Some(rpid), Some(lhigh)) = (lf.right, lf.high) else {
            return Ok(false);
        };
        if ln.count() > 0 || ln.incomplete() {
            return Ok(false);
        }
        let mut freed = pager.reserve();
        freed.free(lpid);
        if let Some(x) = left.as_mut().filter(|_| !first) {
            let xpid = x.pid();
            let xm = |Malformed| Error::Malformed(xpid);
            let xn = latched(x, xpid, self.owner)?;
            let xf = node::fences(&xn).map_err(xm)?;
            let shrinks = xf.high.as_ref().map_or(0, Vec::len);
            if xn.room().map_err(xm)? + shrinks >= lhigh.len() {
                let x_items = xn.items().map_err(xm)?;
                pager.update_all_with(freed, &mut [parent, x, &mut leaf], |p| {
                    node::remove(p[0], j);
                    let bounds = Bounds {
                        high: Some(&lhigh),
                        right: Some(rpid),
                        ..xf.bounds()
                    };
                    node::build(p[1], 0, &x_items, bounds);
                    node::set_deleted(p[2]);
                })?;
                return Ok(true);
            }
        }
        let Some((next, sep)) = next_link.filter(|(c, k)| *c == rpid && *k == lhigh) else {
            return Ok(false);
        };
        let mut right = pager.exclusive(next)?;
        let rn = latched(&right, next, self.owner)?;
        let rm = |Malformed| Error::Malformed(next);
        let grows = key.len().saturating_sub(sep.len());
        if rn.incomplete() || rn.room().map_err(rm)? < grows {
            return Ok(false);
        }
        let r_items = rn.items().map_err(rm)?;
        let rf = node::fences(&rn).map_err(rm)?;
        let x_page = match &left {
            Some(x) => {
                let xpid = x.pid();
                let xn = latched(x, xpid, self.owner)?;
                let xm = |Malformed| Error::Malformed(xpid);
                Some((xn.items().map_err(xm)?, node::fences(&xn).map_err(xm)?))
            }
            None => None,
        };
        let mut pages: Vec<&mut PageExclusive<'p, R>> = vec![parent, &mut leaf, &mut right];
        pages.extend(left.as_mut());
        pager.update_all_with(freed, &mut pages, |p| {
            node::remove(p[0], j + 1);
            let mut items = Node::new(p[0])
                .and_then(|n| n.items())
                .expect("a parent is a node");
            items[j].child = Some(next);
            node::rebuild(p[0], &items);
            node::set_deleted(p[1]);
            let r_bounds = Bounds { low, ..rf.bounds() };
            node::build(p[2], 0, &r_items, r_bounds);
            if let Some((x_items, xf)) = &x_page {
                let x_bounds = Bounds {
                    right: Some(next),
                    ..xf.bounds()
                };
                node::build(p[3], 0, x_items, x_bounds);
            }
        })?;
        Ok(true)
    }

    /// Returns the exclusive latch on the leaf whose right link names `lpid`,
    /// the child at slot `j` of the latched `parent`, given by page id and
    /// node, with low key `low`, or `None` if the leaf at the parent's first
    /// slot has no such left neighbour now.
    ///
    /// For a later slot, walks right from the child before it under the same
    /// parent; for the first, finds the neighbour under the parent's left
    /// sibling by a descent to the keys just below `low`.
    fn left_of<'p, R: Runtime>(
        &self,
        pager: &'p Pager<R>,
        parent: (Pid, &Node),
        j: usize,
        lpid: Pid,
        low: &[u8],
    ) -> Result<Option<PageExclusive<'p, R>>, Error> {
        let (ppid, pn) = parent;
        let pm = |Malformed| Error::Malformed(ppid);
        if j == 0 {
            let start = self.descend_before(pager, low, 0)?;
            let x = pager.exclusive(start)?;
            let xn = latched(&x, start, self.owner)?;
            let high = xn.high().map_err(|Malformed| Error::Malformed(start))?;
            let linked = !xn.deleted() && xn.level() == 0 && xn.right() == Some(lpid);
            return Ok((linked && high == Some(low)).then_some(x));
        }
        let mut x = pager.exclusive(pn.child(j - 1).map_err(pm)?)?;
        loop {
            let xpid = x.pid();
            let xn = latched(&x, xpid, self.owner)?;
            match xn.right() {
                Some(r) if r == lpid => return Ok(Some(x)),
                Some(r) if xn.high().map_err(|Malformed| Error::Malformed(xpid))? < Some(low) => {
                    x = pager.exclusive(r)?;
                }
                _ => return Err(Error::Malformed(xpid)),
            }
        }
    }

    /// Insert the downlink `item` into inner page `g` at `level`, which holds
    /// its key under an exclusive latch, splitting pages up the levels of
    /// `stack` as they fill. Returns false if another thread added the
    /// downlink first.
    ///
    /// Each change that adds a downlink clears the mark of the page it names,
    /// under that page's latch, and is skipped if another thread cleared it
    /// first. Each page a split adds below the root is marked until its own
    /// downlink is added. A split releases its latches before latching the
    /// parent level, so latches are taken top-down and left to right. A page
    /// without a downlink passed on the way to a parent is finished last.
    fn put<'p, R: Runtime>(
        &self,
        pager: &'p Pager<R>,
        mut g: PageExclusive<'p, R>,
        mut level: u16,
        mut stack: Vec<Pid>,
        mut item: Item,
    ) -> Result<bool, Error> {
        let mut first = true;
        let mut passed = Vec::new();
        let added = loop {
            let c = item.child.expect("a downlink names a child");
            let mut child = pager.exclusive(c)?;
            let cn = latched(&child, c, self.owner)?;
            let same =
                cn.low().map_err(|Malformed| Error::Malformed(c))? == Some(item.key.as_slice());
            if !cn.incomplete() || cn.deleted() || cn.level() + 1 != level || !same {
                break !first;
            }
            let pid = g.pid();
            let malformed = |Malformed| Error::Malformed(pid);
            let n = latched(&g, pid, self.owner)?;
            let i = n.search(&item.key, true).map_err(malformed)?;
            if n.room().map_err(malformed)? >= item.cost() {
                pager.update_all(&mut [&mut g, &mut child], |p| {
                    node::insert(p[0], i, &item);
                    complete(p, 1);
                });
                break true;
            }
            let mut items = n.items().map_err(malformed)?;
            items.insert(i, item.clone());
            let Some((sep, q)) =
                self.split_page(pager, &mut g, level, &items, i, Some(&mut child))?
            else {
                break true;
            };
            drop((child, g));
            first = false;
            let from = stack.pop().unwrap_or(self.root);
            level += 1;
            let (parent, skipped) = self.latch(pager, from, &sep, level)?;
            passed.extend(skipped);
            g = parent;
            item = Item::link(sep, q);
        };
        drop(g);
        for split in passed {
            self.finish(pager, split)?;
        }
        Ok(added)
    }

    /// Split page `g` at `level` to hold `items`, the item at `at` among
    /// them new, clearing the mark of the downlink's child `below` in the same
    /// change. Returns the separator and the new right page, which is marked
    /// until its downlink is added, or `None` when `g` is the root, whose
    /// halves move to two new pages under it.
    fn split_page<'p, R: Runtime>(
        &self,
        pager: &'p Pager<R>,
        g: &mut PageExclusive<'p, R>,
        level: u16,
        items: &[Item],
        at: usize,
        below: Option<&mut PageExclusive<'p, R>>,
    ) -> Result<Option<(Vec<u8>, Pid)>, Error> {
        let pid = g.pid();
        let n = latched(g, pid, self.owner)?;
        let f = node::fences(&n).map_err(|Malformed| Error::Malformed(pid))?;
        let s = split_point(items, at, f.right.is_none());
        let sep = if level == 0 {
            separator(&items[s - 1].last(), &items[s].first())
        } else {
            items[s].first()
        };
        let (left, upper) = items.split_at(s);
        if pid == self.root {
            let mut new = pager.reserve();
            let a = new.page(FORMAT, kind_of(level), self.owner)?;
            let b = new.page(FORMAT, kind_of(level), self.owner)?;
            let root = [Item::link(Vec::new(), a), Item::link(sep.clone(), b)];
            let mut ga = pager.exclusive(a)?;
            let mut gb = pager.exclusive(b)?;
            let mut pages: Vec<&mut PageExclusive<'p, R>> = vec![g, &mut ga, &mut gb];
            pages.extend(below);
            pager.update_all_with(new, &mut pages, |p| {
                let a_bounds = Bounds {
                    low: None,
                    high: Some(&sep),
                    right: Some(b),
                };
                let b_bounds = Bounds {
                    low: Some(&sep),
                    high: None,
                    right: None,
                };
                let root_bounds = Bounds {
                    low: None,
                    high: None,
                    right: None,
                };
                node::build(p[1], level, left, a_bounds);
                node::build(p[2], level, upper, b_bounds);
                node::build(p[0], level + 1, &root, root_bounds);
                complete(p, 3);
            })?;
            return Ok(None);
        }
        let mut new = pager.reserve();
        let q = new.page(FORMAT, kind_of(level), self.owner)?;
        let mut gq = pager.exclusive(q)?;
        let mut pages: Vec<&mut PageExclusive<'p, R>> = vec![g, &mut gq];
        pages.extend(below);
        pager.update_all_with(new, &mut pages, |p| {
            let left_bounds = Bounds {
                high: Some(&sep),
                right: Some(q),
                ..f.bounds()
            };
            let upper_bounds = Bounds {
                low: Some(&sep),
                ..f.bounds()
            };
            node::build(p[0], level, left, left_bounds);
            node::build(p[1], level, upper, upper_bounds);
            node::set_incomplete(p[1], true);
            complete(p, 2);
        })?;
        Ok(Some((sep, q)))
    }

    /// Returns a cursor over the entries between `lo` and `hi`.
    pub fn range(&self, lo: Bound<&[u8]>, hi: Bound<&[u8]>) -> Cursor {
        let from = match lo {
            Bound::Included(x) | Bound::Excluded(x) => x.to_vec(),
            Bound::Unbounded => Vec::new(),
        };
        Cursor {
            tree: *self,
            resume: Some(from),
            after: lo.map(<[u8]>::to_vec),
            hi: hi.map(<[u8]>::to_vec),
            buffered: VecDeque::new(),
        }
    }

    /// Returns a cursor over the entries that start with `prefix`.
    pub fn prefix(&self, prefix: &[u8]) -> Cursor {
        let end = prefix_end(prefix);
        self.range(Bound::Included(prefix), end.as_ref().map(Vec::as_slice))
    }
}

/// Returns the bound just past every byte string that starts with `prefix`.
pub fn prefix_end(prefix: &[u8]) -> Bound<Vec<u8>> {
    let Some(last) = prefix.iter().rposition(|&b| b != 0xFF) else {
        return Bound::Unbounded;
    };
    let mut end = prefix[..=last].to_vec();
    end[last] += 1;
    Bound::Excluded(end)
}

fn below(x: &[u8], hi: &Bound<Vec<u8>>) -> bool {
    match hi {
        Bound::Included(h) => x <= h.as_slice(),
        Bound::Excluded(h) => x < h.as_slice(),
        Bound::Unbounded => true,
    }
}

/// An ascending walk over a range of a tree's entries, one leaf at a time.
///
/// Between leaves it holds a key, the high key of the last leaf read, and
/// never a page id.
#[derive(Debug)]
pub struct Cursor {
    tree: BTree,
    /// The key the next leaf is found by, or `None` past the last leaf.
    resume: Option<Vec<u8>>,
    /// Entries at or below this bound have been returned or are out of range.
    after: Bound<Vec<u8>>,
    hi: Bound<Vec<u8>>,
    buffered: VecDeque<Vec<u8>>,
}

impl Cursor {
    /// Returns the next entry in the range, or `None` past its end.
    pub fn next<R: Runtime>(&mut self, pager: &Pager<R>) -> Result<Option<Vec<u8>>, Error> {
        loop {
            if let Some(e) = self.buffered.pop_front() {
                self.after = Bound::Excluded(e.clone());
                return Ok(Some(e));
            }
            let Some(from) = self.resume.take() else {
                return Ok(None);
            };
            let (entries, resume) = self.tree.at_leaf(pager, &from, |n| {
                let mut entries = Vec::new();
                for e in leaf::entries(n)? {
                    let past = match &self.after {
                        Bound::Included(x) => e >= *x,
                        Bound::Excluded(x) => e > *x,
                        Bound::Unbounded => true,
                    };
                    if !past {
                        continue;
                    }
                    if !below(&e, &self.hi) {
                        return Ok((entries, None));
                    }
                    entries.push(e);
                }
                let high = n.high()?.filter(|h| below(h, &self.hi));
                Ok((entries, high.map(<[u8]>::to_vec)))
            })?;
            self.buffered = entries.into();
            self.resume = resume;
        }
    }

    /// Returns every entry left in the range.
    pub fn collect<R: Runtime>(mut self, pager: &Pager<R>) -> Result<Vec<Vec<u8>>, Error> {
        let mut out = Vec::new();
        while let Some(e) = self.next(pager)? {
            out.push(e);
        }
        Ok(out)
    }
}
