use std::cmp::Ordering;

use super::node::{self, Item, Malformed, Node};

/// Where an entry stands among a leaf's: the cell that holds it or that it
/// would go before, its position among that cell's remainders, and whether it
/// is there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub cell: usize,
    pub at: usize,
    pub found: bool,
}

/// Returns how `x` compares with `key` followed by `rest`.
fn cmp_joined(x: &[u8], key: &[u8], rest: &[u8]) -> Ordering {
    if x.len() < key.len() {
        return match x.cmp(&key[..x.len()]) {
            Ordering::Equal => Ordering::Less,
            o => o,
        };
    }
    match x[..key.len()].cmp(key) {
        Ordering::Equal => x[key.len()..].cmp(rest),
        o => o,
    }
}

/// Returns how `x` compares with the last entry of slot `i`.
fn cmp_last(n: &Node, i: usize, x: &[u8]) -> Result<Ordering, Malformed> {
    let key = n.key(i)?;
    Ok(match n.rest(i)? {
        None => x.cmp(key),
        Some(rest) => cmp_joined(x, key, rest.last().ok_or(Malformed)?),
    })
}

/// Returns where `x` stands among the entries of leaf `n`.
pub fn find(n: &Node, x: &[u8]) -> Result<Place, Malformed> {
    let (mut lo, mut hi) = (0, n.count());
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if cmp_last(n, mid, x)? == Ordering::Greater {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo == n.count() {
        return Ok(Place {
            cell: lo,
            at: 0,
            found: false,
        });
    }
    let key = n.key(lo)?;
    let Some(rest) = n.rest(lo)? else {
        return Ok(Place {
            cell: lo,
            at: 0,
            found: key == x,
        });
    };
    let (mut a, mut b) = (0, rest.len());
    while a < b {
        let mid = a + (b - a) / 2;
        if cmp_joined(x, key, rest[mid]) == Ordering::Greater {
            a = mid + 1;
        } else {
            b = mid;
        }
    }
    let found = a < rest.len() && cmp_joined(x, key, rest[a]) == Ordering::Equal;
    Ok(Place {
        cell: lo,
        at: a,
        found,
    })
}

/// Returns whether leaf `n` holds `x`.
pub fn contains(n: &Node, x: &[u8]) -> Result<bool, Malformed> {
    Ok(find(n, x)?.found)
}

/// Returns the number of entries of leaf `n`, counting each of a posting's.
pub fn len(n: &Node) -> Result<usize, Malformed> {
    let mut total = 0;
    for i in 0..n.count() {
        total += n.rest(i)?.map_or(1, |r| r.len());
    }
    Ok(total)
}

/// Returns the entries of leaf `n` at or after `place`, in order.
pub fn entries_from(n: &Node, place: Place) -> Result<Vec<Vec<u8>>, Malformed> {
    let mut out = Vec::new();
    for i in place.cell..n.count() {
        let key = n.key(i)?;
        match n.rest(i)? {
            None => out.push(key.to_vec()),
            Some(rest) => {
                let skip = if i == place.cell { place.at } else { 0 };
                out.extend(rest[skip..].iter().map(|r| [key, r].concat()));
            }
        }
    }
    Ok(out)
}

/// Returns the entries of leaf `n`, in order.
pub fn entries(n: &Node) -> Result<Vec<Vec<u8>>, Malformed> {
    entries_from(
        n,
        Place {
            cell: 0,
            at: 0,
            found: false,
        },
    )
}

/// Returns the largest cost a posting may grow to in a page of `page_size`
/// bytes.
pub fn posting_cap(page_size: usize) -> usize {
    node::capacity(page_size) / 5
}

/// Returns `x`'s remainder after `key`, if `x` starts with `key` and the
/// remainder is short enough to be a posting's.
fn remainder<'a>(x: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    x.strip_prefix(key)
        .filter(|r| r.len() <= usize::from(u8::MAX))
}

/// How [`add`] placed an entry among a leaf's items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Added {
    /// Into the posting at this index, which grew.
    Joined(usize),
    /// As a new item at this index, the others unchanged.
    Inserted(usize),
    /// Into the middle of a posting that was full, which was cut in two
    /// around the item at this index.
    Cut(usize),
}

impl Added {
    pub fn index(self) -> usize {
        match self {
            Added::Joined(i) | Added::Inserted(i) | Added::Cut(i) => i,
        }
    }
}

/// Add `x`, which is not among them, to the leaf items `items`, joining the
/// posting whose key it starts with while the posting costs at most `cap`.
pub fn add(items: &mut Vec<Item>, x: &[u8], cap: usize) -> Added {
    let last_below = |it: &Item| it.entries().last().is_some_and(|l| l.as_slice() < x);
    let c = items.partition_point(last_below);
    let join = |it: &Item| -> Option<Item> {
        if it.rest.is_empty() {
            return None;
        }
        let r = remainder(x, &it.key)?;
        let mut grown = it.clone();
        let j = grown.rest.partition_point(|s| s.as_slice() < r);
        grown.rest.insert(j, r.to_vec());
        Some(grown)
    };
    if let Some(it) = items.get(c)
        && !it.rest.is_empty()
        && it.first().as_slice() < x
    {
        if let Some(grown) = join(it).filter(|g| g.cost() <= cap) {
            items[c] = grown;
            return Added::Joined(c);
        }
        let j = it
            .rest
            .partition_point(|s| cmp_joined(x, &it.key, s) == Ordering::Greater);
        let upper = Item {
            key: it.key.clone(),
            child: None,
            rest: it.rest[j..].to_vec(),
        };
        items[c].rest.truncate(j);
        items.insert(c + 1, upper);
        items.insert(c + 1, Item::entry(x.to_vec()));
        return Added::Cut(c + 1);
    }
    for at in [c, c.wrapping_sub(1)] {
        if let Some(grown) = items.get(at).and_then(join).filter(|g| g.cost() <= cap) {
            items[at] = grown;
            return Added::Joined(at);
        }
    }
    items.insert(c, Item::entry(x.to_vec()));
    Added::Inserted(c)
}

/// Returns `items` with each run of entries that share a key, as `key_len`
/// measures it, kept once as postings under `cap`.
pub fn dedup(items: &[Item], key_len: &dyn Fn(&[u8]) -> Option<usize>, cap: usize) -> Vec<Item> {
    let entries: Vec<Vec<u8>> = items.iter().flat_map(Item::entries).collect();
    let mut out: Vec<Item> = Vec::new();
    let mut i = 0;
    while i < entries.len() {
        let key = key_len(&entries[i])
            .filter(|&k| k <= entries[i].len())
            .map(|k| entries[i][..k].to_vec());
        let mut j = i + 1;
        if let Some(key) = &key {
            while j < entries.len() && remainder(&entries[j], key).is_some() {
                j += 1;
            }
            if remainder(&entries[i], key).is_none() {
                j = i + 1;
            }
        }
        match key {
            Some(key) if j - i >= 2 => {
                let mut posting = Item {
                    key: key.clone(),
                    child: None,
                    rest: Vec::new(),
                };
                for e in &entries[i..j] {
                    let r = e[key.len()..].to_vec();
                    let mut grown = posting.clone();
                    grown.rest.push(r.clone());
                    if grown.cost() > cap && !posting.rest.is_empty() {
                        out.push(posting);
                        posting = Item {
                            key: key.clone(),
                            child: None,
                            rest: vec![r],
                        };
                    } else {
                        posting = grown;
                    }
                }
                out.push(posting);
            }
            _ => out.extend(entries[i..j].iter().cloned().map(Item::entry)),
        }
        i = j;
    }
    out
}

/// Add `x` to the leaf `page`, which does not hold it, joining a posting when
/// it can. Returns false, changing nothing, if there is no room.
///
/// Panics if the page is not a leaf.
pub fn insert(page: &mut [u8], x: &[u8]) -> bool {
    let n = Node::new(page).expect("an inserted leaf is a node");
    let place = find(&n, x).expect("an inserted leaf is a node");
    let cap = posting_cap(page.len());
    let room = || n.room().expect("an inserted leaf is a node");
    let inside = place.cell < n.count() && place.at > 0;
    let neighbours: &[usize] = if inside {
        &[place.cell]
    } else {
        &[place.cell, place.cell.wrapping_sub(1)]
    };
    for &c in neighbours {
        if c >= n.count() {
            continue;
        }
        let it = n.item(c).expect("an inserted leaf is a node");
        let Some(r) = remainder(x, &it.key).filter(|_| !it.rest.is_empty()) else {
            continue;
        };
        let mut grown = it.clone();
        let j = grown.rest.partition_point(|s| s.as_slice() < r);
        grown.rest.insert(j, r.to_vec());
        if grown.cost() <= cap {
            if room() + it.cost() < grown.cost() {
                return false;
            }
            node::replace(page, c, &grown);
            return true;
        }
    }
    if !inside {
        let item = Item::entry(x.to_vec());
        if room() < item.cost() {
            return false;
        }
        node::insert(page, place.cell, &item);
        return true;
    }
    let mut items = n.items().expect("an inserted leaf is a node");
    add(&mut items, x, cap);
    if !node::fits(page, &items) {
        return false;
    }
    node::rebuild(page, &items);
    true
}

/// Remove the entry at `place`, which the leaf `page` holds.
///
/// Panics if the page is not a leaf or holds no entry there.
pub fn remove(page: &mut [u8], place: Place) {
    let n = Node::new(page).expect("a leaf removed from is a node");
    let mut it = n.item(place.cell).expect("a leaf removed from is a node");
    if it.rest.len() <= 1 {
        node::remove(page, place.cell);
        return;
    }
    it.rest.remove(place.at);
    node::replace(page, place.cell, &it);
}

/// Returns `items` with the entries `keep` refuses removed, postings left
/// empty dropped, and the number removed.
pub fn retain(items: &[Item], mut keep: impl FnMut(&[u8]) -> bool) -> (Vec<Item>, usize) {
    let mut out = Vec::new();
    let mut gone = 0;
    for it in items {
        if it.rest.is_empty() {
            if keep(&it.key) {
                out.push(it.clone());
            } else {
                gone += 1;
            }
            continue;
        }
        let mut kept = it.clone();
        kept.rest.clear();
        for r in &it.rest {
            if keep(&[&it.key[..], r].concat()) {
                kept.rest.push(r.clone());
            } else {
                gone += 1;
            }
        }
        if !kept.rest.is_empty() {
            out.push(kept);
        }
    }
    (out, gone)
}
