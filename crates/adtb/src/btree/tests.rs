use std::ops::Bound;

use super::node::{self, Item, Malformed, Node};
use super::tree::prefix_end;
use super::{BTree, FORMAT};
use crate::storage::{FIRST_EDITABLE, HEADER_LEN, PageKind, Pid, header_edit};

const PAGE: usize = 4096;

fn blank(kind: PageKind) -> Vec<u8> {
    let mut p = vec![0; PAGE];
    p[FIRST_EDITABLE..HEADER_LEN].copy_from_slice(&header_edit(FORMAT, kind, 9, 0));
    p
}

fn leaf(key: &[u8]) -> Item {
    Item {
        key: key.to_vec(),
        child: None,
    }
}

fn keys(page: &[u8]) -> Vec<Vec<u8>> {
    let n = Node::new(page).unwrap();
    (0..n.count()).map(|i| n.key(i).unwrap().to_vec()).collect()
}

#[test]
fn a_zero_leaf_page_is_an_empty_rightmost_leaf() {
    let p = blank(PageKind::IndexLeaf);
    let n = Node::new(&p).unwrap();
    assert!(n.is_leaf());
    assert_eq!(n.count(), 0);
    assert_eq!(n.right(), None);
    assert_eq!(n.high(), Ok(None));
    assert_eq!(n.owner(), 9);
}

#[test]
fn a_page_of_another_kind_or_level_is_not_a_node() {
    assert_eq!(
        Node::new(&blank(PageKind::HeapSlotted)).err(),
        Some(Malformed)
    );
    assert_eq!(
        Node::new(&blank(PageKind::IndexInner)).err(),
        Some(Malformed)
    );
}

#[test]
fn inserts_keep_slots_in_order() {
    let mut p = blank(PageKind::IndexLeaf);
    for k in [&b"m"[..], b"a", b"z", b"", b"mm"] {
        let i = Node::new(&p).unwrap().search(k, true).unwrap();
        node::insert(&mut p, i, &leaf(k));
    }
    assert_eq!(
        keys(&p),
        [&b""[..], b"a", b"m", b"mm", b"z"].map(<[u8]>::to_vec)
    );
    let n = Node::new(&p).unwrap();
    assert_eq!(n.search(b"m", true), Ok(2));
    assert_eq!(n.search(b"m", false), Ok(3));
    assert_eq!(n.search(b"zz", true), Ok(5));
}

#[test]
fn removed_cells_are_reclaimed_by_compaction() {
    let mut p = blank(PageKind::IndexLeaf);
    let big = vec![7; 600];
    let room = Node::new(&p).unwrap().room().unwrap();
    let mut n = 0;
    while Node::new(&p).unwrap().room().unwrap() >= leaf(&big).cost() {
        let mut k = big.clone();
        k.push(n);
        node::insert(&mut p, usize::from(n), &leaf(&k));
        n += 1;
    }
    assert_eq!(n, 6);
    for _ in 0..n {
        node::remove(&mut p, 0);
    }
    assert_eq!(Node::new(&p).unwrap().room(), Ok(room));
    for i in 0..n {
        let mut k = big.clone();
        k.push(i + 100);
        node::insert(&mut p, usize::from(i), &leaf(&k));
    }
    assert_eq!(keys(&p).len(), usize::from(n));
}

#[test]
fn build_writes_high_key_right_link_and_children() {
    let mut p = blank(PageKind::IndexLeaf);
    let items = [
        Item {
            key: vec![],
            child: Some(Pid(10)),
        },
        Item {
            key: b"k".to_vec(),
            child: Some(Pid(11)),
        },
    ];
    node::build(&mut p, 1, &items, Some(b"q"), Some(Pid(12)));
    assert_eq!(crate::storage::kind(&p), PageKind::IndexInner as u16);
    assert_eq!(crate::storage::owner(&p), 9);
    let n = Node::new(&p).unwrap();
    assert_eq!(n.level(), 1);
    assert_eq!(n.high(), Ok(Some(&b"q"[..])));
    assert_eq!(n.right(), Some(Pid(12)));
    assert_eq!(n.child_for(b"a"), Ok(Pid(10)));
    assert_eq!(n.child_for(b"k"), Ok(Pid(11)));
    assert_eq!(n.child_for(b"p"), Ok(Pid(11)));
    assert_eq!(n.beyond(b"q"), Ok(true));
    assert_eq!(n.beyond(b"p"), Ok(false));
    assert_eq!(n.items(), Ok(items.to_vec()));
}

#[test]
fn out_of_bounds_offsets_are_refused() {
    let mut p = blank(PageKind::IndexLeaf);
    node::insert(&mut p, 0, &leaf(b"abc"));
    let mut bad_slot = p.clone();
    bad_slot[40..42].copy_from_slice(&30_u16.to_le_bytes());
    assert_eq!(Node::new(&bad_slot).unwrap().key(0), Err(Malformed));
    let mut bad_len = p.clone();
    let at = usize::from(u16::from_le_bytes([p[40], p[41]]));
    bad_len[at..at + 2].copy_from_slice(&900_u16.to_le_bytes());
    assert_eq!(Node::new(&bad_len).unwrap().key(0), Err(Malformed));
    let mut bad_count = p.clone();
    bad_count[32..34].copy_from_slice(&3000_u16.to_le_bytes());
    assert!(Node::new(&bad_count).is_err());
}

#[test]
fn the_largest_entries_still_split_into_two_pages() {
    let max = BTree::max_entry_len(PAGE);
    let item = Item {
        key: vec![0; max],
        child: Some(Pid(1)),
    };
    let high = 2 + max;
    assert!(node::capacity(PAGE) >= 3 * item.cost() + high);
    assert!(BTree::max_entry_len(65536) > 10_000);
}

#[test]
fn prefix_ends_just_past_the_prefix() {
    assert_eq!(prefix_end(b"ab"), Bound::Excluded(b"ac".to_vec()));
    assert_eq!(prefix_end(&[1, 0xFF, 0xFF]), Bound::Excluded(vec![2]));
    assert_eq!(prefix_end(&[0xFF]), Bound::Unbounded);
    assert_eq!(prefix_end(&[]), Bound::Unbounded);
}
