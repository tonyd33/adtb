use std::cmp::Ordering;

use crate::storage::{FIRST_EDITABLE, HEADER_LEN, PageKind, Pid, flags, header_edit, kind, owner};

pub const FORMAT: u16 = 1;

const RIGHT: usize = HEADER_LEN;
const COUNT: usize = 32;
const LEVEL: usize = 34;
const USED: usize = 36;
const HIGH: usize = 38;
const LOW: usize = 40;
const SLOTS: usize = 42;

const INCOMPLETE_SPLIT: u16 = 1 << 1;
const DELETED: u16 = 1 << 2;

const SLOT: usize = 2;
const LEN: usize = 2;
const CHILD: usize = 8;
const POSTING: u16 = 1 << 15;
const COUNT_LEN: usize = 2;

/// A page whose bytes are not a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Malformed;

/// A read-only view of a leaf or inner page.
///
/// Layout from offset 24: the right sibling's page id (0 for none), the slot
/// count, the level (0 for a leaf), the size of the cell area, which ends at
/// the page's end, the offset of the high key's cell and of the low key's
/// cell (0 for none), then one 2-byte cell offset per slot in key order. A
/// cell is a 2-byte key length and the key, followed in an inner page by the
/// child's page id. A leaf cell whose length has its top bit set is a
/// posting: its key is followed by a 2-byte count and that many remainders,
/// each a 1-byte length and its bytes, in increasing order, and it stands for
/// the entries that are its key followed by each remainder. Every field is
/// little-endian. A page of zero contents is
/// an empty leaf with no right sibling and no fences. Bit 1 of the header's
/// flags marks a page whose downlink is not yet in its parent, and bit 2 a
/// page unlinked from its tree.
#[derive(Clone, Copy, Debug)]
pub struct Node<'a> {
    page: &'a [u8],
    count: usize,
    level: u16,
}

pub fn kind_of(level: u16) -> PageKind {
    if level == 0 {
        PageKind::IndexLeaf
    } else {
        PageKind::IndexInner
    }
}

fn u16_at(page: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([page[at], page[at + 1]])
}

fn put_u16(page: &mut [u8], at: usize, v: usize) {
    let v = u16::try_from(v).expect("a page field fits in 16 bits");
    page[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

fn cell_len(key: usize, level: u16) -> usize {
    LEN + key + if level == 0 { 0 } else { CHILD }
}

/// Returns the bytes one item of `key` bytes takes in a page at `level`,
/// slot included.
pub fn item_cost(key: usize, level: u16) -> usize {
    SLOT + cell_len(key, level)
}

/// Returns the bytes of a page available to slots, cells and the fence keys.
pub fn capacity(page_size: usize) -> usize {
    page_size - SLOTS
}

impl<'a> Node<'a> {
    pub fn new(page: &'a [u8]) -> Result<Node<'a>, Malformed> {
        if page.len() < SLOTS {
            return Err(Malformed);
        }
        let count = usize::from(u16_at(page, COUNT));
        let level = u16_at(page, LEVEL);
        let used = usize::from(u16_at(page, USED));
        if kind(page) != kind_of(level) as u16 || SLOTS + count * SLOT + used > page.len() {
            return Err(Malformed);
        }
        Ok(Node { page, count, level })
    }

    pub fn level(&self) -> u16 {
        self.level
    }

    pub fn is_leaf(&self) -> bool {
        self.level == 0
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn owner(&self) -> u32 {
        owner(self.page)
    }

    /// Returns whether the page's downlink is not yet in its parent.
    pub fn incomplete(&self) -> bool {
        flags(self.page) & INCOMPLETE_SPLIT != 0
    }

    /// Returns whether the page was unlinked from its tree.
    pub fn deleted(&self) -> bool {
        flags(self.page) & DELETED != 0
    }

    pub fn right(&self) -> Option<Pid> {
        let p = u64::from_le_bytes(self.page[RIGHT..RIGHT + 8].try_into().unwrap());
        (p != 0).then_some(Pid(p))
    }

    fn cells_start(&self) -> usize {
        self.page.len() - usize::from(u16_at(self.page, USED))
    }

    fn free(&self) -> usize {
        self.cells_start() - SLOTS - self.count * SLOT
    }

    fn key_at(&self, at: usize) -> Result<&'a [u8], Malformed> {
        if at < self.cells_start() || at + LEN > self.page.len() {
            return Err(Malformed);
        }
        let len = usize::from(u16_at(self.page, at) & !POSTING);
        self.page.get(at + LEN..at + LEN + len).ok_or(Malformed)
    }

    /// Returns the bytes of the cell at `at`.
    fn cell_size(&self, at: usize) -> Result<usize, Malformed> {
        let key = self.key_at(at)?;
        let head = LEN + key.len();
        if u16_at(self.page, at) & POSTING == 0 {
            return Ok(head + if self.is_leaf() { 0 } else { CHILD });
        }
        let mut p = at + head;
        let n = self.page.get(p..p + COUNT_LEN).ok_or(Malformed)?;
        let n = usize::from(u16::from_le_bytes([n[0], n[1]]));
        p += COUNT_LEN;
        for _ in 0..n {
            p += 1 + usize::from(*self.page.get(p).ok_or(Malformed)?);
        }
        if p > self.page.len() {
            return Err(Malformed);
        }
        Ok(p - at)
    }

    /// Returns the remainders of the posting cell at `at`, or `None` for a
    /// cell that is not a posting.
    fn rest_at(&self, at: usize) -> Result<Option<Vec<&'a [u8]>>, Malformed> {
        let key = self.key_at(at)?;
        if u16_at(self.page, at) & POSTING == 0 {
            return Ok(None);
        }
        if !self.is_leaf() {
            return Err(Malformed);
        }
        let mut p = at + LEN + key.len();
        let n = self.page.get(p..p + COUNT_LEN).ok_or(Malformed)?;
        let n = usize::from(u16::from_le_bytes([n[0], n[1]]));
        p += COUNT_LEN;
        let mut rest = Vec::with_capacity(n.min(self.page.len()));
        for _ in 0..n {
            let len = usize::from(*self.page.get(p).ok_or(Malformed)?);
            rest.push(self.page.get(p + 1..p + 1 + len).ok_or(Malformed)?);
            p += 1 + len;
        }
        if rest.is_empty() {
            return Err(Malformed);
        }
        Ok(Some(rest))
    }

    fn slot(&self, i: usize) -> Result<usize, Malformed> {
        if i >= self.count {
            return Err(Malformed);
        }
        Ok(usize::from(u16_at(self.page, SLOTS + i * SLOT)))
    }

    pub fn high(&self) -> Result<Option<&'a [u8]>, Malformed> {
        match usize::from(u16_at(self.page, HIGH)) {
            0 => Ok(None),
            at => self.key_at(at).map(Some),
        }
    }

    /// Returns the low key: every key of the page is at or above it, and the
    /// page's left sibling has it as its high key.
    pub fn low(&self) -> Result<Option<&'a [u8]>, Malformed> {
        match usize::from(u16_at(self.page, LOW)) {
            0 => Ok(None),
            at => self.key_at(at).map(Some),
        }
    }

    /// Returns whether `x` is at or above the low key.
    pub fn above_low(&self, x: &[u8]) -> Result<bool, Malformed> {
        Ok(self.low()?.is_none_or(|l| x >= l))
    }

    /// Returns the entry of slot `i` in a leaf, or the key of its posting, or
    /// its separator in an inner page.
    pub fn key(&self, i: usize) -> Result<&'a [u8], Malformed> {
        self.key_at(self.slot(i)?)
    }

    /// Returns the remainders of slot `i` if it is a posting.
    pub fn rest(&self, i: usize) -> Result<Option<Vec<&'a [u8]>>, Malformed> {
        self.rest_at(self.slot(i)?)
    }

    pub fn child(&self, i: usize) -> Result<Pid, Malformed> {
        if self.is_leaf() {
            return Err(Malformed);
        }
        let at = self.slot(i)?;
        let key = self.key_at(at)?;
        if u16_at(self.page, at) & POSTING != 0 {
            return Err(Malformed);
        }
        let end = at + LEN + key.len();
        let bytes = self.page.get(end..end + CHILD).ok_or(Malformed)?;
        Ok(Pid(u64::from_le_bytes(bytes.try_into().unwrap())))
    }

    /// Returns the first slot whose key is not below `x`, or after it when
    /// `inclusive` is false.
    pub fn search(&self, x: &[u8], inclusive: bool) -> Result<usize, Malformed> {
        let (mut lo, mut hi) = (0, self.count);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let below = match self.key(mid)?.cmp(x) {
                Ordering::Less => true,
                Ordering::Equal => !inclusive,
                Ordering::Greater => false,
            };
            if below {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        Ok(lo)
    }

    /// Returns whether `x` is at or above the high key, so that it belongs to
    /// a page to the right.
    pub fn beyond(&self, x: &[u8]) -> Result<bool, Malformed> {
        Ok(self.high()?.is_some_and(|h| x >= h))
    }

    /// Returns the child of an inner page that covers `x`.
    pub fn child_for(&self, x: &[u8]) -> Result<Pid, Malformed> {
        self.child(self.search(x, false)?.saturating_sub(1))
    }

    /// Returns the bytes free for items once the cell area is compacted.
    pub fn room(&self) -> Result<usize, Malformed> {
        let mut live = 0;
        for i in 0..self.count {
            live += self.cell_size(self.slot(i)?)?;
        }
        let high = self.high()?.map_or(0, |h| LEN + h.len());
        let low = self.low()?.map_or(0, |l| LEN + l.len());
        Ok(capacity(self.page.len()) - self.count * SLOT - live - high - low)
    }

    pub fn item(&self, i: usize) -> Result<Item, Malformed> {
        let child = if self.is_leaf() {
            None
        } else {
            Some(self.child(i)?)
        };
        let rest = self
            .rest(i)?
            .map_or_else(Vec::new, |r| r.into_iter().map(<[u8]>::to_vec).collect());
        Ok(Item {
            key: self.key(i)?.to_vec(),
            child,
            rest,
        })
    }

    pub fn items(&self) -> Result<Vec<Item>, Malformed> {
        (0..self.count).map(|i| self.item(i)).collect()
    }
}

/// An entry of a leaf, a posting of a leaf, or a separator and its child in
/// an inner page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub key: Vec<u8>,
    pub child: Option<Pid>,
    /// The remainders of a posting, in increasing order; empty for any other
    /// item.
    pub rest: Vec<Vec<u8>>,
}

impl Item {
    pub fn entry(entry: Vec<u8>) -> Item {
        Item {
            key: entry,
            child: None,
            rest: Vec::new(),
        }
    }

    pub fn link(key: Vec<u8>, child: Pid) -> Item {
        Item {
            key,
            child: Some(child),
            rest: Vec::new(),
        }
    }

    fn cell_len(&self) -> usize {
        let rest = if self.rest.is_empty() {
            0
        } else {
            COUNT_LEN + self.rest.iter().map(|r| 1 + r.len()).sum::<usize>()
        };
        LEN + self.key.len() + self.child.map_or(0, |_| CHILD) + rest
    }

    pub fn cost(&self) -> usize {
        SLOT + self.cell_len()
    }

    /// Returns the entries the item stands for, in order.
    pub fn entries(&self) -> Vec<Vec<u8>> {
        if self.rest.is_empty() {
            return vec![self.key.clone()];
        }
        self.rest
            .iter()
            .map(|r| [&self.key[..], r].concat())
            .collect()
    }

    pub fn first(&self) -> Vec<u8> {
        match self.rest.first() {
            Some(r) => [&self.key[..], r].concat(),
            None => self.key.clone(),
        }
    }
}

fn push_cell(page: &mut [u8], item: &Item) -> usize {
    let len = item.cell_len();
    let used = usize::from(u16_at(page, USED)) + len;
    let at = page.len() - used;
    let key = &item.key;
    let flag = if item.rest.is_empty() { 0 } else { POSTING };
    put_u16(page, at, key.len() | usize::from(flag));
    let mut p = at + LEN;
    page[p..p + key.len()].copy_from_slice(key);
    p += key.len();
    if let Some(c) = item.child {
        page[p..p + CHILD].copy_from_slice(&c.0.to_le_bytes());
        p += CHILD;
    }
    if !item.rest.is_empty() {
        put_u16(page, p, item.rest.len());
        p += COUNT_LEN;
        for r in &item.rest {
            page[p] = u8::try_from(r.len()).expect("a remainder fits in 255 bytes");
            page[p + 1..p + 1 + r.len()].copy_from_slice(r);
            p += 1 + r.len();
        }
    }
    put_u16(page, USED, used);
    at
}

fn fence_cell(page: &mut [u8], key: &[u8]) -> usize {
    push_cell(page, &Item::entry(key.to_vec()))
}

/// The fence keys of a node and its right sibling.
#[derive(Clone, Copy, Debug)]
pub struct Bounds<'a> {
    pub low: Option<&'a [u8]>,
    pub high: Option<&'a [u8]>,
    pub right: Option<Pid>,
}

/// Write a node of `items` at `level` over `page`, keeping its owner and flags.
///
/// Panics if the items and fence keys do not fit, or if an item's child does
/// not match the level.
pub fn build(page: &mut [u8], level: u16, items: &[Item], bounds: Bounds) {
    let Bounds { low, high, right } = bounds;
    let header = header_edit(FORMAT, kind_of(level), owner(page), flags(page));
    page[FIRST_EDITABLE..HEADER_LEN].copy_from_slice(&header);
    page[HEADER_LEN..].fill(0);
    page[RIGHT..RIGHT + 8].copy_from_slice(&right.map_or(0, |p| p.0).to_le_bytes());
    put_u16(page, LEVEL, usize::from(level));
    if let Some(h) = high {
        let at = fence_cell(page, h);
        put_u16(page, HIGH, at);
    }
    if let Some(l) = low {
        let at = fence_cell(page, l);
        put_u16(page, LOW, at);
    }
    for (i, item) in items.iter().enumerate() {
        assert_eq!(
            item.child.is_some(),
            level > 0,
            "item does not suit level {level}"
        );
        assert!(
            item.rest.is_empty() || level == 0,
            "a posting lies only in a leaf"
        );
        let at = push_cell(page, item);
        put_u16(page, SLOTS + i * SLOT, at);
    }
    put_u16(page, COUNT, items.len());
    assert!(
        SLOTS + items.len() * SLOT <= page.len() - usize::from(u16_at(page, USED)),
        "items overflow the page"
    );
}

fn set_flag(page: &mut [u8], flag: u16, on: bool) {
    let level = Node::new(page).expect("a marked page is a node").level();
    let rest = flags(page) & !flag;
    let mark = if on { flag } else { 0 };
    let header = header_edit(FORMAT, kind_of(level), owner(page), rest | mark);
    page[FIRST_EDITABLE..HEADER_LEN].copy_from_slice(&header);
}

/// Mark `page` as lacking its downlink in its parent, or clear the mark.
///
/// Panics if the page is not a node.
pub fn set_incomplete(page: &mut [u8], incomplete: bool) {
    set_flag(page, INCOMPLETE_SPLIT, incomplete);
}

/// Mark `page` as unlinked from its tree.
///
/// Panics if the page is not a node.
pub fn set_deleted(page: &mut [u8]) {
    set_flag(page, DELETED, true);
}

/// The fence keys and right sibling of a node, copied.
#[derive(Clone, Debug)]
pub struct Fences {
    pub low: Option<Vec<u8>>,
    pub high: Option<Vec<u8>>,
    pub right: Option<Pid>,
}

impl Fences {
    pub fn bounds(&self) -> Bounds<'_> {
        Bounds {
            low: self.low.as_deref(),
            high: self.high.as_deref(),
            right: self.right,
        }
    }
}

pub fn fences(node: &Node) -> Result<Fences, Malformed> {
    Ok(Fences {
        low: node.low()?.map(<[u8]>::to_vec),
        high: node.high()?.map(<[u8]>::to_vec),
        right: node.right(),
    })
}

/// Returns whether `items` fit in `page` beside its fence keys.
///
/// Panics if the page is not a node.
pub fn fits(page: &[u8], items: &[Item]) -> bool {
    let node = Node::new(page).expect("a page fitted is a node");
    let f = fences(&node).expect("a page fitted is a node");
    let fences: usize = [f.low, f.high]
        .iter()
        .flatten()
        .map(|k| LEN + k.len())
        .sum();
    items.iter().map(Item::cost).sum::<usize>() + fences <= capacity(page.len())
}

/// Rewrite `page` to hold `items` with its own fence keys and right sibling.
///
/// Panics if the page is not a node or the items do not fit.
pub fn rebuild(page: &mut [u8], items: &[Item]) {
    let node = Node::new(page).expect("a rebuilt page is a node");
    let level = node.level();
    let f = fences(&node).expect("a rebuilt page is a node");
    build(page, level, items, f.bounds());
}

/// Insert `item` at slot `i`, compacting the cell area first if its free
/// gap is too small.
///
/// Panics if the item does not fit or the page is not a node.
pub fn insert(page: &mut [u8], i: usize, item: &Item) {
    let node = Node::new(page).expect("an inserted page is a node");
    let level = node.level();
    if node.free() < item.cost() {
        let items = node.items().expect("an inserted page is a node");
        rebuild(page, &items);
    }
    let node = Node::new(page).unwrap();
    let count = node.count();
    assert!(node.free() >= item.cost(), "item overflows the page");
    assert!(i <= count, "slot {i} is past {count}");
    assert_eq!(
        item.child.is_some(),
        level > 0,
        "item does not suit level {level}"
    );
    let at = push_cell(page, item);
    let slot = SLOTS + i * SLOT;
    page.copy_within(slot..SLOTS + count * SLOT, slot + SLOT);
    put_u16(page, slot, at);
    put_u16(page, COUNT, count + 1);
}

/// Replace the cell of slot `i` by `item`, compacting the cell area first if
/// its free gap is too small. The old cell stays as garbage.
///
/// Panics if the item does not fit, there is no slot `i`, or the page is not
/// a node.
pub fn replace(page: &mut [u8], i: usize, item: &Item) {
    let node = Node::new(page).expect("a replaced page is a node");
    assert!(i < node.count(), "slot {i} is past {}", node.count());
    if node.free() < item.cell_len() {
        let mut items = node.items().expect("a replaced page is a node");
        items[i] = item.clone();
        rebuild(page, &items);
        return;
    }
    let at = push_cell(page, item);
    put_u16(page, SLOTS + i * SLOT, at);
}

/// Remove slot `i`. Its cell stays as garbage until the page is compacted.
///
/// Panics if there is no slot `i`.
pub fn remove(page: &mut [u8], i: usize) {
    let count = Node::new(page)
        .expect("a page removed from is a node")
        .count();
    assert!(i < count, "slot {i} is past {count}");
    let slot = SLOTS + i * SLOT;
    page.copy_within(slot + SLOT..SLOTS + count * SLOT, slot);
    put_u16(page, SLOTS + (count - 1) * SLOT, 0);
    put_u16(page, COUNT, count - 1);
}
