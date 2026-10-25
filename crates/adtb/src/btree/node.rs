use std::cmp::Ordering;

use crate::storage::{FIRST_EDITABLE, HEADER_LEN, PageKind, Pid, flags, header_edit, kind, owner};

pub const FORMAT: u16 = 1;

const RIGHT: usize = HEADER_LEN;
const COUNT: usize = 32;
const LEVEL: usize = 34;
const USED: usize = 36;
const HIGH: usize = 38;
const SLOTS: usize = 40;

const SLOT: usize = 2;
const LEN: usize = 2;
const CHILD: usize = 8;

/// A page whose bytes are not a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Malformed;

/// A read-only view of a leaf or inner page.
///
/// Layout from offset 24: the right sibling's page id (0 for none), the slot
/// count, the level (0 for a leaf), the size of the cell area, which ends at
/// the page's end, and the offset of the high key's cell (0 for none), then
/// one 2-byte cell offset per slot in key order. A cell is a 2-byte key
/// length and the key, followed in an inner page by the child's page id.
/// Every field is little-endian. A page of zero contents is an empty leaf
/// with no right sibling.
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

/// Returns the bytes of a page available to slots, cells and the high key.
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
        let len = usize::from(u16_at(self.page, at));
        self.page.get(at + LEN..at + LEN + len).ok_or(Malformed)
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

    /// Returns the entry of slot `i` in a leaf, or its separator in an inner
    /// page.
    pub fn key(&self, i: usize) -> Result<&'a [u8], Malformed> {
        self.key_at(self.slot(i)?)
    }

    pub fn child(&self, i: usize) -> Result<Pid, Malformed> {
        if self.is_leaf() {
            return Err(Malformed);
        }
        let at = self.slot(i)?;
        let key = self.key_at(at)?;
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
            live += cell_len(self.key(i)?.len(), self.level);
        }
        let high = self.high()?.map_or(0, |h| LEN + h.len());
        Ok(capacity(self.page.len()) - self.count * SLOT - live - high)
    }

    pub fn items(&self) -> Result<Vec<Item>, Malformed> {
        (0..self.count)
            .map(|i| {
                let child = if self.is_leaf() {
                    None
                } else {
                    Some(self.child(i)?)
                };
                Ok(Item {
                    key: self.key(i)?.to_vec(),
                    child,
                })
            })
            .collect()
    }
}

/// An entry of a leaf, or a separator and its child in an inner page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub key: Vec<u8>,
    pub child: Option<Pid>,
}

impl Item {
    pub fn cost(&self) -> usize {
        SLOT + LEN + self.key.len() + self.child.map_or(0, |_| CHILD)
    }
}

fn push_cell(page: &mut [u8], key: &[u8], child: Option<Pid>) -> usize {
    let len = LEN + key.len() + child.map_or(0, |_| CHILD);
    let used = usize::from(u16_at(page, USED)) + len;
    let at = page.len() - used;
    put_u16(page, at, key.len());
    page[at + LEN..at + LEN + key.len()].copy_from_slice(key);
    if let Some(c) = child {
        page[at + LEN + key.len()..at + len].copy_from_slice(&c.0.to_le_bytes());
    }
    put_u16(page, USED, used);
    at
}

/// Write a node of `items` at `level` over `page`, keeping its owner and flags.
///
/// Panics if the items and high key do not fit, or if an item's child does
/// not match the level.
pub fn build(page: &mut [u8], level: u16, items: &[Item], high: Option<&[u8]>, right: Option<Pid>) {
    let header = header_edit(FORMAT, kind_of(level), owner(page), flags(page));
    page[FIRST_EDITABLE..HEADER_LEN].copy_from_slice(&header);
    page[HEADER_LEN..].fill(0);
    page[RIGHT..RIGHT + 8].copy_from_slice(&right.map_or(0, |p| p.0).to_le_bytes());
    put_u16(page, LEVEL, usize::from(level));
    if let Some(h) = high {
        let at = push_cell(page, h, None);
        put_u16(page, HIGH, at);
    }
    for (i, item) in items.iter().enumerate() {
        assert_eq!(
            item.child.is_some(),
            level > 0,
            "item does not suit level {level}"
        );
        let at = push_cell(page, &item.key, item.child);
        put_u16(page, SLOTS + i * SLOT, at);
    }
    put_u16(page, COUNT, items.len());
    assert!(
        SLOTS + items.len() * SLOT <= page.len() - usize::from(u16_at(page, USED)),
        "items overflow the page"
    );
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
        let high = node
            .high()
            .expect("an inserted page is a node")
            .map(<[u8]>::to_vec);
        let right = node.right();
        build(page, level, &items, high.as_deref(), right);
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
    let at = push_cell(page, &item.key, item.child);
    let slot = SLOTS + i * SLOT;
    page.copy_within(slot..SLOTS + count * SLOT, slot + SLOT);
    put_u16(page, slot, at);
    put_u16(page, COUNT, count + 1);
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
