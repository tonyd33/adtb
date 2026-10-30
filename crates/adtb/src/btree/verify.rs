use std::collections::{BTreeMap, BTreeSet};

use crate::runtime::Runtime;
use crate::storage::{self, Lsn, Pager, Pid, Stopped};

use super::leaf;
use super::node::{self, Item, Malformed, Node};
use super::tree::{BTree, Error, Split, node_of, visit};

struct Page {
    incomplete: bool,
    low: Option<Vec<u8>>,
    items: Vec<Item>,
    high: Option<Vec<u8>>,
    right: Option<Pid>,
}

/// How many times a live check rereads a page pair a writer keeps changing before it passes on.
const LIVE_TRIES: usize = 16;

/// A node as one read under its latch saw it.
struct Live {
    lsn: Lsn,
    level: u16,
    deleted: bool,
    page: Page,
}

/// The node a walk reached the next one through: its page and LSN, and the low key the next must
/// have.
#[derive(Clone)]
struct Anchor {
    pid: Pid,
    lsn: Lsn,
    high: Option<Vec<u8>>,
}

/// Returns `pid` as one latched read sees it, `Err` for its LSN when it is no node of `owner`.
fn live_at<R: Runtime>(pager: &Pager<R>, owner: u32, pid: Pid) -> Result<Result<Live, Lsn>, Error> {
    let bytes = pager.read(pid)?;
    let lsn = storage::lsn(&bytes);
    let read = |n: &Node| -> Result<Live, Malformed> {
        Ok(Live {
            lsn,
            level: n.level(),
            deleted: n.deleted(),
            page: Page {
                incomplete: n.incomplete(),
                low: n.low()?.map(<[u8]>::to_vec),
                items: if n.is_leaf() {
                    leaf::entries(n)?.into_iter().map(Item::entry).collect()
                } else {
                    n.items()?
                },
                high: n.high()?.map(<[u8]>::to_vec),
                right: n.right(),
            },
        })
    };
    Ok(node_of(&bytes, owner)
        .and_then(|n| read(&n))
        .map_err(|Malformed| lsn))
}

/// Returns whether `pid` still has LSN `lsn`.
fn unchanged<R: Runtime>(pager: &Pager<R>, pid: Pid, lsn: Lsn) -> Result<bool, Error> {
    Ok(storage::lsn(&pager.read(pid)?) == lsn)
}

/// Returns what is wrong with one node, as its latched read shows: its order, fences and level.
fn local_fault(l: &Live, level: u16, low: Option<&[u8]>) -> Option<&'static str> {
    let p = &l.page;
    if l.level != level {
        Some("lies at the wrong level")
    } else if l.deleted {
        Some("is deleted and still linked")
    } else if p.high.is_some() != p.right.is_some() {
        Some("has a high key and right link that disagree")
    } else if p.low.as_deref() != low {
        Some("has a low key other than its left neighbour's high key")
    } else if !in_order(&p.items, p.low.as_deref(), p.high.as_deref())
        || p.high
            .as_deref()
            .is_some_and(|h| p.low.as_deref().is_some_and(|l| h <= l))
    {
        Some("holds entries out of order")
    } else {
        None
    }
}

/// Returns whether `items` increase strictly, lie at or above `low` and lie
/// below `high`.
fn in_order(items: &[Item], low: Option<&[u8]>, high: Option<&[u8]>) -> bool {
    items.windows(2).all(|w| w[0].key < w[1].key)
        && items
            .first()
            .is_none_or(|i| low.is_none_or(|l| i.key.as_slice() >= l))
        && items
            .last()
            .is_none_or(|i| high.is_none_or(|h| i.key.as_slice() < h))
}

impl BTree {
    /// Returns every entry in order, failing at the first page that breaks
    /// the tree's structure.
    ///
    /// Each level is walked by its right links. Each page's low key is its
    /// left sibling's high key, and no page reached is marked unlinked. A
    /// page lacks a downlink from the level above exactly when it is marked
    /// incomplete; every downlink must name a page of the level below, in
    /// order, whose low key is the downlink's separator.
    pub fn verify<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Vec<u8>>, Error> {
        let mut seen = BTreeSet::new();
        let height = visit(pager, self.owner(), self.root(), |n| {
            if n.right().is_some() || n.high()?.is_some() || n.low()?.is_some() {
                return Err(Malformed);
            }
            Ok(n.level())
        })?;
        let mut downlinks: Vec<(Pid, Option<Vec<u8>>)> = vec![(self.root(), None)];
        let mut entries = Vec::new();
        for level in (0..=height).rev() {
            let mut chain: BTreeMap<Pid, (usize, Option<Vec<u8>>, bool)> = BTreeMap::new();
            let mut below = Vec::new();
            let mut low: Option<Vec<u8>> = None;
            let mut next = Some(downlinks[0].0);
            while let Some(pid) = next {
                if !seen.insert(pid) {
                    return Err(Error::Malformed(pid));
                }
                let page = visit(pager, self.owner(), pid, |n| {
                    if n.level() != level
                        || n.deleted()
                        || n.high()?.is_some() != n.right().is_some()
                    {
                        return Err(Malformed);
                    }
                    Ok(Page {
                        incomplete: n.incomplete(),
                        low: n.low()?.map(<[u8]>::to_vec),
                        items: if n.is_leaf() {
                            leaf::entries(n)?.into_iter().map(Item::entry).collect()
                        } else {
                            n.items()?
                        },
                        high: n.high()?.map(<[u8]>::to_vec),
                        right: n.right(),
                    })
                })?;
                let ordered = in_order(&page.items, low.as_deref(), page.high.as_deref())
                    && page
                        .high
                        .as_deref()
                        .is_none_or(|h| low.as_deref().is_none_or(|l| h > l));
                if !ordered || page.low != low {
                    return Err(Error::Malformed(pid));
                }
                if level == 0 {
                    entries.extend(page.items.into_iter().map(|i| i.key));
                } else {
                    let first = page.items.first().ok_or(Error::Malformed(pid))?;
                    if first.key != low.clone().unwrap_or_default() {
                        return Err(Error::Malformed(pid));
                    }
                    for (i, item) in page.items.into_iter().enumerate() {
                        let sep = if i == 0 { low.clone() } else { Some(item.key) };
                        below.push((item.child.expect("an inner item has a child"), sep));
                    }
                }
                chain.insert(pid, (chain.len(), low, page.incomplete));
                low = page.high;
                next = page.right;
            }
            let mut last = None;
            for (pid, sep) in &downlinks {
                match chain.get(pid) {
                    Some((at, l, false)) if last < Some(at) && l == sep => last = Some(at),
                    _ => return Err(Error::Malformed(*pid)),
                }
            }
            let linked: BTreeSet<Pid> = downlinks.iter().map(|(p, _)| *p).collect();
            if let Some((pid, _)) = chain.iter().find(|(p, c)| !c.2 && !linked.contains(p)) {
                return Err(Error::Malformed(*pid));
            }
            downlinks = below;
        }
        Ok(entries)
    }

    /// Returns each node found to break the tree's structure, with what it breaks, while writers
    /// change the tree. A fact about two pages counts only when the page linking to the other kept
    /// its LSN across the read of the other. A pair a writer keeps changing is passed over.
    pub fn check_live<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<(Pid, String)>, Error> {
        self.check_live_trying(pager, LIVE_TRIES)
    }

    /// Check as [`BTree::check_live`] does, rereading a changing pair `tries` times.
    #[doc(hidden)]
    pub fn check_live_trying<R: Runtime>(
        &self,
        pager: &Pager<R>,
        tries: usize,
    ) -> Result<Vec<(Pid, String)>, Error> {
        let mut found = Vec::new();
        let Ok(root) = live_at(pager, self.owner(), self.root())? else {
            return Ok(vec![(self.root(), "is no node of its tree".into())]);
        };
        if root.page.right.is_some() || root.page.high.is_some() || root.page.low.is_some() {
            found.push((self.root(), "the root has a neighbour".into()));
        }
        let mut linked: Option<BTreeSet<Pid>> = None;
        let mut level = root.level;
        loop {
            let below = self.check_level(pager, level, tries, linked.as_ref(), &mut found)?;
            let Some(next) = level.checked_sub(1) else {
                break;
            };
            linked = Some(below);
            level = next;
        }
        Ok(found)
    }

    /// Walk `level` by right links, adding what is wrong to `found`. Returns the pages its
    /// downlinks name. Past `limit` rereads of a changing link, checks the page reached alone, and
    /// goes on from it when it is a live page of this level.
    fn check_level<R: Runtime>(
        &self,
        pager: &Pager<R>,
        level: u16,
        limit: usize,
        linked: Option<&BTreeSet<Pid>>,
        found: &mut Vec<(Pid, String)>,
    ) -> Result<BTreeSet<Pid>, Error> {
        let mut below = BTreeSet::new();
        let mut anchor: Option<Anchor> = None;
        let mut next = Some(self.descend(pager, self.root(), &[], level)?.pid);
        let mut tries = 0;
        let mut stale = 0;
        while let Some(pid) = next {
            let read = live_at(pager, self.owner(), pid)?;
            let fresh = match &anchor {
                Some(a) => {
                    if unchanged(pager, a.pid, a.lsn)? {
                        None
                    } else {
                        Some(self.step_from(pager, a, level)?)
                    }
                }
                None => {
                    let first = self.descend(pager, self.root(), &[], level)?.pid;
                    (first != pid).then_some((None, Some(first)))
                }
            };
            let mut passed = false;
            if let Some((moved, right)) = fresh {
                tries += 1;
                if tries <= limit {
                    anchor = moved;
                    next = right;
                    continue;
                }
                passed = true;
                if !matches!(&read, Ok(l) if !l.deleted && l.level == level) {
                    stale += 1;
                    if stale > limit.max(1) || right.is_none() {
                        break;
                    }
                    anchor = moved;
                    next = right;
                    continue;
                }
            }
            tries = 0;
            let Ok(l) = read else {
                found.push((pid, "is no node of its tree".into()));
                break;
            };
            let low = if passed {
                l.page.low.as_deref()
            } else {
                anchor.as_ref().and_then(|a| a.high.as_deref())
            };
            if let Some(fault) = local_fault(&l, level, low) {
                found.push((pid, fault.into()));
            }
            if let Some(linked) = linked
                && !l.page.incomplete
                && !linked.contains(&pid)
                && !self.has_downlink(pager, pid, l.lsn, l.page.low.as_deref(), level)?
            {
                found.push((pid, "has no downlink and is not marked incomplete".into()));
            }
            if level > 0 {
                self.check_downlinks(pager, pid, &l, &mut below, found)?;
            }
            next = l.page.right;
            anchor = Some(Anchor {
                pid,
                lsn: l.lsn,
                high: l.page.high,
            });
        }
        Ok(below)
    }

    /// Returns where a walk goes on after the page linking it changed: that page as it is now and
    /// its right link, or no anchor and the level's first page when it no longer lies on the level.
    fn step_from<R: Runtime>(
        &self,
        pager: &Pager<R>,
        a: &Anchor,
        level: u16,
    ) -> Result<(Option<Anchor>, Option<Pid>), Error> {
        Ok(match live_at(pager, self.owner(), a.pid)? {
            Ok(l) if !l.deleted && l.level == level => (
                Some(Anchor {
                    pid: a.pid,
                    lsn: l.lsn,
                    high: l.page.high,
                }),
                l.page.right,
            ),
            _ => (
                None,
                Some(self.descend(pager, self.root(), &[], level)?.pid),
            ),
        })
    }

    /// Returns whether the page above `pid` names it, or `pid` left LSN `lsn` meanwhile.
    fn has_downlink<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        lsn: Lsn,
        low: Option<&[u8]>,
        level: u16,
    ) -> Result<bool, Error> {
        let above = self
            .descend(pager, self.root(), low.unwrap_or_default(), level + 1)?
            .pid;
        let parent = live_at(pager, self.owner(), above)?;
        let named = parent.is_ok_and(|p| p.page.items.iter().any(|i| i.child == Some(pid)));
        Ok(named || !unchanged(pager, pid, lsn)?)
    }

    /// Check each child inner node `l` at `pid` names at one moment with it, adding each to
    /// `below` and what is wrong to `found`.
    fn check_downlinks<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        l: &Live,
        below: &mut BTreeSet<Pid>,
        found: &mut Vec<(Pid, String)>,
    ) -> Result<(), Error> {
        let low = l.page.low.clone();
        for (i, item) in l.page.items.iter().enumerate() {
            let child = item.child.expect("an inner item has a child");
            below.insert(child);
            let read = live_at(pager, self.owner(), child)?;
            if !unchanged(pager, pid, l.lsn)? {
                return Ok(());
            }
            let sep = if i == 0 {
                low.as_deref()
            } else {
                Some(item.key.as_slice())
            };
            let bad = match read {
                Err(_) => Some("is no node of its tree"),
                Ok(c) if c.level + 1 != l.level || c.deleted => {
                    Some("is at the wrong level or deleted under a downlink")
                }
                Ok(c) if c.page.incomplete => Some("is marked incomplete and has a downlink"),
                Ok(c) if c.page.low.as_deref() != sep => {
                    Some("has a low key other than its downlink's separator")
                }
                Ok(_) => None,
            };
            if let Some(bad) = bad {
                found.push((child, bad.into()));
            }
        }
        Ok(())
    }

    /// Swap the first two entries of leaf `pid`. Plants a fault for a test of the checks.
    #[doc(hidden)]
    pub fn misorder_leaf<R: Runtime>(&self, pager: &Pager<R>, pid: Pid) -> Result<(), Error> {
        let (mut items, bounds) = {
            let page = pager.read(pid)?;
            let n = node_of(&page, self.owner()).map_err(|Malformed| Error::Malformed(pid))?;
            let items: Vec<Item> = leaf::entries(&n)
                .map_err(|Malformed| Error::Malformed(pid))?
                .into_iter()
                .map(Item::entry)
                .collect();
            let f = node::fences(&n).map_err(|Malformed| Error::Malformed(pid))?;
            (items, f)
        };
        items.swap(0, 1);
        pager.update(&[pid], |p| node::build(p[0], 0, &items, bounds.bounds()))?;
        Ok(())
    }

    /// Flip the owner of node `pid`, which a second flip undoes. Plants a fault for a test of the
    /// mapping of a damaged page.
    #[doc(hidden)]
    pub fn flip_owner<R: Runtime>(&self, pager: &Pager<R>, pid: Pid) -> Result<(), Error> {
        pager.update(&[pid], |p| storage::flip_owner(p[0]))?;
        Ok(())
    }

    /// Finish every split whose page lacks its downlink. Returns the number
    /// this pass finished, which leaves out any another thread finished
    /// first.
    pub fn repair<R: Runtime>(&self, pager: &Pager<R>) -> Result<usize, Error> {
        let height = self.height(pager)?;
        let mut finished = 0;
        for level in (0..height).rev() {
            let mut next = Some(self.descend(pager, self.root(), &[], level)?.pid);
            let mut low: Option<Vec<u8>> = None;
            while let Some(pid) = next {
                let (live, incomplete, high, right) = visit(pager, self.owner(), pid, |n| {
                    let live =
                        !n.deleted() && n.level() == level && n.low()?.map(<[u8]>::to_vec) == low;
                    Ok((
                        live,
                        n.incomplete(),
                        n.high()?.map(<[u8]>::to_vec),
                        n.right(),
                    ))
                })?;
                if !live {
                    let key = low.clone().unwrap_or_default();
                    let found = self.descend(pager, self.root(), &key, level)?.pid;
                    low = visit(pager, self.owner(), found, |n| {
                        Ok(n.low()?.map(<[u8]>::to_vec))
                    })?;
                    next = Some(found);
                    continue;
                }
                if incomplete {
                    let low = low.ok_or(Error::Malformed(pid))?;
                    let split = Split {
                        pid,
                        level,
                        low,
                        stack: Vec::new(),
                    };
                    if self.finish(pager, split)? {
                        finished += 1;
                    }
                }
                low = high;
                next = right;
            }
        }
        Ok(finished)
    }
}

impl BTree {
    /// Returns every page of the tree as [`BTree::pages`] does, for the mapping of a damaged page
    /// to what holds it.
    pub fn walk<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pid>, Stopped> {
        self.pages(pager).map_err(|e| match e {
            Error::Page(p) => Stopped::from(p),
            _ => Stopped::Shape,
        })
    }

    /// Returns every page of the tree: each level from the root down, left to right by its
    /// right links.
    pub fn pages<R: Runtime>(&self, pager: &Pager<R>) -> Result<Vec<Pid>, Error> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        let mut leftmost = Some(self.root());
        while let Some(start) = leftmost.take() {
            let mut at = Some(start);
            while let Some(pid) = at {
                if !seen.insert(pid) {
                    return Err(Error::Malformed(pid));
                }
                out.push(pid);
                let (right, down) = visit(pager, self.owner(), pid, |n| {
                    let down = if n.is_leaf() { None } else { Some(n.child(0)?) };
                    Ok((n.right(), down))
                })?;
                if pid == start {
                    leftmost = down;
                }
                at = right;
            }
        }
        Ok(out)
    }
}
