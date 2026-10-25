use std::collections::{BTreeMap, BTreeSet};

use crate::runtime::Runtime;
use crate::storage::{Pager, Pid};

use super::node::{Item, Malformed};
use super::tree::{BTree, Error, visit};

struct Page {
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
    /// Each level is walked by its right links. A page may lack a downlink
    /// from the level above; every downlink must name a page of the level
    /// below, in order, whose low key is the downlink's separator.
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
            let mut chain: BTreeMap<Pid, (usize, Option<Vec<u8>>)> = BTreeMap::new();
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
                chain.insert(pid, (chain.len(), low));
                low = page.high;
                next = page.right;
            }
            let mut last = None;
            for (pid, sep) in &downlinks {
                match chain.get(pid) {
                    Some((at, l)) if last < Some(at) && l == sep => last = Some(at),
                    _ => return Err(Error::Malformed(*pid)),
                }
            }
            downlinks = below;
        }
        Ok(entries)
    }
}
