use std::cmp::Ordering;
use std::collections::VecDeque;
use std::ops::Bound;

use crate::key::DecodeError;
use crate::runtime::Runtime;
use crate::storage::{PageError, Pager, Pid};

use super::node::{self, FORMAT, Item, Malformed, Node, kind_of};

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

enum Step {
    Right(Pid),
    Down(Pid),
    Here,
}

enum Plan {
    Right(Pid),
    Fits(usize),
    Split {
        level: u16,
        items: Vec<Item>,
        high: Option<Vec<u8>>,
        right: Option<Pid>,
    },
}

/// Returns what `f` makes of node `pid`, refusing a page that is not a node
/// of `owner`.
pub(super) fn visit<R: Runtime, T>(
    pager: &mut Pager<R>,
    owner: u32,
    pid: Pid,
    f: impl FnOnce(&Node) -> Result<T, Malformed>,
) -> Result<T, Error> {
    let page = &pager.read(pid)?.to_vec();
    let node = Node::new(page)
        .ok()
        .filter(|n| n.owner() == owner)
        .ok_or(Error::Malformed(pid))?;
    f(&node).map_err(|Malformed| Error::Malformed(pid))
}

/// Returns the slot that splits `items` into two halves of about equal bytes,
/// each holding at least one item.
fn split_point(items: &[Item]) -> usize {
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
    pub fn create<R: Runtime>(pager: &mut Pager<R>, owner: u32) -> Result<BTree, PageError> {
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
    pub fn height<R: Runtime>(&self, pager: &mut Pager<R>) -> Result<u16, Error> {
        visit(pager, self.owner, self.root, |n| Ok(n.level()))
    }

    /// Returns the longest entry a tree over pages of `page_size` bytes holds.
    pub fn max_entry_len(page_size: usize) -> usize {
        node::capacity(page_size) / 5 - node::item_cost(0, 1)
    }

    /// Returns the page at `level` whose range holds `key`, and the page
    /// passed at each level above it, root first.
    fn descend<R: Runtime>(
        &self,
        pager: &mut Pager<R>,
        key: &[u8],
        level: u16,
    ) -> Result<(Pid, Vec<Pid>), Error> {
        let mut pid = self.root;
        let mut stack = Vec::new();
        loop {
            let step = visit(pager, self.owner, pid, |n| {
                if n.beyond(key)? {
                    return n.right().map(Step::Right).ok_or(Malformed);
                }
                match n.level().cmp(&level) {
                    Ordering::Equal => Ok(Step::Here),
                    Ordering::Greater => n.child_for(key).map(Step::Down),
                    Ordering::Less => Err(Malformed),
                }
            })?;
            match step {
                Step::Right(p) => pid = p,
                Step::Down(c) => {
                    stack.push(pid);
                    pid = c;
                }
                Step::Here => return Ok((pid, stack)),
            }
        }
    }

    /// Returns the leaf that holds `entry` and its slot there, if it is in the
    /// tree.
    fn find<R: Runtime>(
        &self,
        pager: &mut Pager<R>,
        entry: &[u8],
    ) -> Result<(Pid, Vec<Pid>, Option<usize>), Error> {
        let (leaf, stack) = self.descend(pager, entry, 0)?;
        let slot = visit(pager, self.owner, leaf, |n| {
            let i = n.search(entry, true)?;
            Ok((i < n.count() && n.key(i)? == entry).then_some(i))
        })?;
        Ok((leaf, stack, slot))
    }

    pub fn contains<R: Runtime>(&self, pager: &mut Pager<R>, entry: &[u8]) -> Result<bool, Error> {
        Ok(self.find(pager, entry)?.2.is_some())
    }

    /// Insert `entry`. Returns false, changing nothing, if it is already in
    /// the tree.
    pub fn insert<R: Runtime>(&self, pager: &mut Pager<R>, entry: &[u8]) -> Result<bool, Error> {
        let max = BTree::max_entry_len(pager.page_size());
        if entry.len() > max {
            return Err(Error::TooLong {
                len: entry.len(),
                max,
            });
        }
        let (leaf, stack, slot) = self.find(pager, entry)?;
        if slot.is_some() {
            return Ok(false);
        }
        let item = Item {
            key: entry.to_vec(),
            child: None,
        };
        self.put(pager, leaf, stack, item)?;
        Ok(true)
    }

    /// Delete `entry`. Returns false if it was not in the tree.
    pub fn delete<R: Runtime>(&self, pager: &mut Pager<R>, entry: &[u8]) -> Result<bool, Error> {
        let (leaf, _, slot) = self.find(pager, entry)?;
        let Some(i) = slot else {
            return Ok(false);
        };
        pager.update(&[leaf], |p| node::remove(p[0], i))?;
        Ok(true)
    }

    /// Insert `item` into the level of page `pid`, splitting pages up the
    /// levels of `stack` as they fill.
    fn put<R: Runtime>(
        &self,
        pager: &mut Pager<R>,
        mut pid: Pid,
        mut stack: Vec<Pid>,
        mut item: Item,
    ) -> Result<(), Error> {
        loop {
            let plan = visit(pager, self.owner, pid, |n| {
                if n.beyond(&item.key)? {
                    return n.right().map(Plan::Right).ok_or(Malformed);
                }
                let i = n.search(&item.key, true)?;
                if n.room()? >= item.cost() {
                    return Ok(Plan::Fits(i));
                }
                let mut items = n.items()?;
                items.insert(i, item.clone());
                Ok(Plan::Split {
                    level: n.level(),
                    items,
                    high: n.high()?.map(<[u8]>::to_vec),
                    right: n.right(),
                })
            })?;
            let (level, items, high, right) = match plan {
                Plan::Right(p) => {
                    pid = p;
                    continue;
                }
                Plan::Fits(i) => {
                    pager.update(&[pid], |p| node::insert(p[0], i, &item))?;
                    return Ok(());
                }
                Plan::Split {
                    level,
                    items,
                    high,
                    right,
                } => (level, items, high, right),
            };
            let s = split_point(&items);
            let sep = items[s].key.clone();
            let (left, upper) = items.split_at(s);
            if pid == self.root {
                let a = pager.allocate(FORMAT, kind_of(level), self.owner)?;
                let b = pager.allocate(FORMAT, kind_of(level), self.owner)?;
                let root = [
                    Item {
                        key: Vec::new(),
                        child: Some(a),
                    },
                    Item {
                        key: sep.clone(),
                        child: Some(b),
                    },
                ];
                pager.update(&[pid, a, b], |p| {
                    node::build(p[1], level, left, Some(&sep), Some(b));
                    node::build(p[2], level, upper, None, None);
                    node::build(p[0], level + 1, &root, None, None);
                })?;
                return Ok(());
            }
            let q = pager.allocate(FORMAT, kind_of(level), self.owner)?;
            pager.update(&[pid, q], |p| {
                node::build(p[0], level, left, Some(&sep), Some(q));
                node::build(p[1], level, upper, high.as_deref(), right);
            })?;
            pid = stack
                .pop()
                .expect("a page below the root was reached through a parent");
            item = Item {
                key: sep,
                child: Some(q),
            };
        }
    }

    /// Returns a cursor over the entries between `lo` and `hi`.
    pub fn range(&self, lo: Bound<&[u8]>, hi: Bound<&[u8]>) -> Cursor {
        Cursor {
            tree: *self,
            next: None,
            after: lo.map(<[u8]>::to_vec),
            hi: hi.map(<[u8]>::to_vec),
            buffered: VecDeque::new(),
            started: false,
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
#[derive(Debug)]
pub struct Cursor {
    tree: BTree,
    next: Option<Pid>,
    /// Entries at or below this bound have been returned or are out of range.
    after: Bound<Vec<u8>>,
    hi: Bound<Vec<u8>>,
    buffered: VecDeque<Vec<u8>>,
    started: bool,
}

impl Cursor {
    /// Returns the next entry in the range, or `None` past its end.
    pub fn next<R: Runtime>(&mut self, pager: &mut Pager<R>) -> Result<Option<Vec<u8>>, Error> {
        loop {
            if let Some(e) = self.buffered.pop_front() {
                self.after = Bound::Excluded(e.clone());
                return Ok(Some(e));
            }
            let pid = if self.started {
                match self.next {
                    Some(p) => p,
                    None => return Ok(None),
                }
            } else {
                self.started = true;
                let from = match &self.after {
                    Bound::Included(x) | Bound::Excluded(x) => x.as_slice(),
                    Bound::Unbounded => &[],
                };
                self.tree.descend(pager, from, 0)?.0
            };
            let (entries, right) = visit(pager, self.tree.owner, pid, |n| {
                let start = match &self.after {
                    Bound::Included(x) => n.search(x, true)?,
                    Bound::Excluded(x) => n.search(x, false)?,
                    Bound::Unbounded => 0,
                };
                let mut entries = Vec::new();
                for i in start..n.count() {
                    let e = n.key(i)?;
                    if !below(e, &self.hi) {
                        return Ok((entries, None));
                    }
                    entries.push(e.to_vec());
                }
                let past = n.high()?.is_some_and(|h| !below(h, &self.hi));
                Ok((entries, if past { None } else { n.right() }))
            })?;
            self.buffered = entries.into();
            self.next = right;
        }
    }

    /// Returns every entry left in the range.
    pub fn collect<R: Runtime>(mut self, pager: &mut Pager<R>) -> Result<Vec<Vec<u8>>, Error> {
        let mut out = Vec::new();
        while let Some(e) = self.next(pager)? {
            out.push(e);
        }
        Ok(out)
    }
}
