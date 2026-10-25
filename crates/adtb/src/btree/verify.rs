use std::collections::{BTreeMap, BTreeSet};

use crate::runtime::Runtime;
use crate::storage::{Pager, Pid};

use super::node::{Item, Malformed};
use super::tree::{BTree, Error, Split, visit};

struct Page {
    incomplete: bool,
    items: Vec<Item>,
    high: Option<Vec<u8>>,
    right: Option<Pid>,
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
    /// Each level is walked by its right links. A page lacks a downlink from
    /// the level above exactly when it is marked incomplete; every downlink
    /// must name a page of the level below, in order, whose low key is the
    /// downlink's separator.
    pub fn verify<R: Runtime>(&self, pager: &mut Pager<R>) -> Result<Vec<Vec<u8>>, Error> {
        let mut seen = BTreeSet::new();
        let height = visit(pager, self.owner(), self.root(), |n| {
            if n.right().is_some() || n.high()?.is_some() {
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
                    if n.level() != level || n.high()?.is_some() != n.right().is_some() {
                        return Err(Malformed);
                    }
                    Ok(Page {
                        incomplete: n.incomplete(),
                        items: n.items()?,
                        high: n.high()?.map(<[u8]>::to_vec),
                        right: n.right(),
                    })
                })?;
                let ordered = in_order(&page.items, low.as_deref(), page.high.as_deref())
                    && page
                        .high
                        .as_deref()
                        .is_none_or(|h| low.as_deref().is_none_or(|l| h > l));
                if !ordered {
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

    /// Finish every split whose page lacks its downlink. Returns the number
    /// finished.
    pub fn repair<R: Runtime>(&self, pager: &mut Pager<R>) -> Result<usize, Error> {
        let height = self.height(pager)?;
        let mut finished = 0;
        for level in (0..height).rev() {
            let mut next = Some(self.descend(pager, &[], level)?.pid);
            let mut low: Option<Vec<u8>> = None;
            while let Some(pid) = next {
                let (incomplete, high, right) = visit(pager, self.owner(), pid, |n| {
                    Ok((n.incomplete(), n.high()?.map(<[u8]>::to_vec), n.right()))
                })?;
                if incomplete {
                    let low = low.ok_or(Error::Malformed(pid))?;
                    let parent = self.descend(pager, &low, level + 1)?;
                    let mut stack = parent.stack;
                    stack.push(parent.pid);
                    self.finish(pager, Split { pid, low, stack })?;
                    finished += 1;
                }
                low = high;
                next = right;
            }
        }
        Ok(finished)
    }
}
