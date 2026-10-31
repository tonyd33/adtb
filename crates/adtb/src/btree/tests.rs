use std::ops::Bound;

use super::build::{BuildError, RunEntry, merge_runs};
use super::leaf;
use super::node::{self, Item, Malformed, Node};
use super::tree::{prefix_end, separator};
use super::{BTree, FORMAT};
use crate::storage::{FIRST_EDITABLE, HEADER_LEN, PageKind, Pid, header_edit};

const PAGE: usize = 4096;

fn run_entry(entry: &[u8], key_len: usize, counted: bool) -> RunEntry {
    RunEntry {
        entry: entry.to_vec(),
        key_len,
        counted,
    }
}

#[test]
fn merging_runs_orders_and_dedups_their_entries() {
    let got = merge_runs(
        vec![
            vec![run_entry(b"a1", 1, false), run_entry(b"c1", 1, false)],
            vec![run_entry(b"a1", 1, true), run_entry(b"b1", 1, false)],
        ],
        false,
        &|| false,
    );
    assert_eq!(
        Ok(vec![b"a1".to_vec(), b"b1".to_vec(), b"c1".to_vec()]),
        got
    );
}

#[test]
fn merging_runs_of_a_unique_index_refuses_two_entries_of_a_key_that_count() {
    let run = |counted| {
        vec![
            vec![run_entry(b"k1", 1, true)],
            vec![run_entry(b"k2", 1, counted), run_entry(b"z1", 1, true)],
        ]
    };
    assert_eq!(
        Err(BuildError::Duplicate(b"k".to_vec())),
        merge_runs(run(true), true, &|| false)
    );
    assert!(merge_runs(run(false), true, &|| false).is_ok());
    assert!(merge_runs(run(true), false, &|| false).is_ok());
}

#[test]
fn a_uniqueness_check_restarts_at_each_key() {
    let got = merge_runs(
        vec![vec![run_entry(b"a1", 1, true), run_entry(b"b1", 1, true)]],
        true,
        &|| false,
    );
    assert!(got.is_ok());
}

#[test]
fn a_merge_stops_between_entries() {
    let asked = std::cell::Cell::new(0);
    let stop = || {
        asked.set(asked.get() + 1);
        asked.get() == 2
    };
    let got = merge_runs(
        vec![vec![
            run_entry(b"a1", 1, false),
            run_entry(b"b1", 1, false),
            run_entry(b"c1", 1, false),
        ]],
        false,
        &stop,
    );
    assert_eq!(Err(BuildError::Stopped), got);
    assert_eq!(2, asked.get());
}

fn blank(kind: PageKind) -> Vec<u8> {
    let mut p = vec![0; PAGE];
    p[FIRST_EDITABLE..HEADER_LEN].copy_from_slice(&header_edit(FORMAT, kind, 9, 0));
    p
}

fn leaf(key: &[u8]) -> Item {
    Item::entry(key.to_vec())
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
        Item::link(vec![], Pid(10)),
        Item::link(b"k".to_vec(), Pid(11)),
    ];
    let bounds = node::Bounds {
        low: Some(b"a"),
        high: Some(b"q"),
        right: Some(Pid(12)),
    };
    node::build(&mut p, 1, &items, bounds);
    assert_eq!(crate::storage::kind(&p), PageKind::IndexInner as u16);
    assert_eq!(crate::storage::owner(&p), 9);
    let n = Node::new(&p).unwrap();
    assert_eq!(n.level(), 1);
    assert_eq!(n.high(), Ok(Some(&b"q"[..])));
    assert_eq!(n.low(), Ok(Some(&b"a"[..])));
    assert_eq!(n.above_low(b"a"), Ok(true));
    assert_eq!(n.above_low(b""), Ok(false));
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
    bad_slot[42..44].copy_from_slice(&30_u16.to_le_bytes());
    assert_eq!(Node::new(&bad_slot).unwrap().key(0), Err(Malformed));
    let mut bad_len = p.clone();
    let at = usize::from(u16::from_le_bytes([p[42], p[43]]));
    bad_len[at..at + 2].copy_from_slice(&900_u16.to_le_bytes());
    assert_eq!(Node::new(&bad_len).unwrap().key(0), Err(Malformed));
    let mut bad_count = p.clone();
    bad_count[32..34].copy_from_slice(&3000_u16.to_le_bytes());
    assert!(Node::new(&bad_count).is_err());
}

#[test]
fn the_largest_entries_still_split_into_two_pages() {
    let max = BTree::max_entry_len(PAGE);
    let item = Item::link(vec![0; max], Pid(1));
    let fence = 2 + max;
    assert!(node::capacity(PAGE) >= 3 * item.cost() + 2 * fence);
    assert!(BTree::max_entry_len(65536) > 10_000);
}

#[test]
fn prefix_ends_just_past_the_prefix() {
    assert_eq!(prefix_end(b"ab"), Bound::Excluded(b"ac".to_vec()));
    assert_eq!(prefix_end(&[1, 0xFF, 0xFF]), Bound::Excluded(vec![2]));
    assert_eq!(prefix_end(&[0xFF]), Bound::Unbounded);
    assert_eq!(prefix_end(&[]), Bound::Unbounded);
}

#[test]
fn the_incomplete_mark_survives_rebuilds_and_keeps_other_flags() {
    let mut p = vec![0; PAGE];
    p[FIRST_EDITABLE..HEADER_LEN].copy_from_slice(&header_edit(FORMAT, PageKind::IndexLeaf, 9, 1));
    assert!(!Node::new(&p).unwrap().incomplete());
    node::set_incomplete(&mut p, true);
    assert!(Node::new(&p).unwrap().incomplete());
    let bounds = node::Bounds {
        low: None,
        high: Some(b"b"),
        right: Some(Pid(4)),
    };
    node::build(&mut p, 0, &[leaf(b"a")], bounds);
    assert!(Node::new(&p).unwrap().incomplete());
    assert_eq!(crate::storage::flags(&p) & 1, 1);
    node::set_incomplete(&mut p, false);
    assert!(!Node::new(&p).unwrap().incomplete());
    assert_eq!(crate::storage::flags(&p), 1);
    assert_eq!(keys(&p), [b"a".to_vec()]);
}

fn posting(key: &[u8], rest: &[&[u8]]) -> Item {
    Item {
        key: key.to_vec(),
        child: None,
        rest: rest.iter().map(|r| r.to_vec()).collect(),
    }
}

fn leaf_of(items: &[Item]) -> Vec<u8> {
    let mut p = blank(PageKind::IndexLeaf);
    let bounds = node::Bounds {
        low: None,
        high: None,
        right: None,
    };
    node::build(&mut p, 0, items, bounds);
    p
}

fn all(page: &[u8]) -> Vec<Vec<u8>> {
    leaf::entries(&Node::new(page).unwrap()).unwrap()
}

fn strings(xs: &[&[u8]]) -> Vec<Vec<u8>> {
    xs.iter().map(|x| x.to_vec()).collect()
}

#[test]
fn a_posting_stands_for_its_key_followed_by_each_remainder() {
    let p = leaf_of(&[leaf(b"a"), posting(b"k", &[b"1", b"3"]), leaf(b"z")]);
    let n = Node::new(&p).unwrap();
    assert_eq!(n.count(), 3);
    assert_eq!(n.item(1), Ok(posting(b"k", &[b"1", b"3"])));
    assert_eq!(leaf::len(&n), Ok(4));
    assert_eq!(all(&p), strings(&[b"a", b"k1", b"k3", b"z"]));
    let at = |x: &[u8]| leaf::find(&n, x).unwrap();
    assert_eq!(
        (at(b"k1").cell, at(b"k1").at, at(b"k1").found),
        (1, 0, true)
    );
    assert_eq!(
        (at(b"k3").cell, at(b"k3").at, at(b"k3").found),
        (1, 1, true)
    );
    assert_eq!(
        (at(b"k2").cell, at(b"k2").at, at(b"k2").found),
        (1, 1, false)
    );
    assert_eq!((at(b"k").cell, at(b"k").at, at(b"k").found), (1, 0, false));
    assert_eq!((at(b"k4").cell, at(b"k4").found), (2, false));
    assert_eq!((at(b"zz").cell, at(b"zz").found), (3, false));
    assert_eq!(leaf::contains(&n, b"k"), Ok(false));
}

#[test]
fn an_entry_sharing_a_postings_key_joins_it() {
    let mut p = leaf_of(&[posting(b"k", &[b"2", b"4"]), leaf(b"m")]);
    for x in [&b"k3"[..], b"k1", b"k5", b"l"] {
        assert!(leaf::insert(&mut p, x));
    }
    assert_eq!(
        all(&p),
        strings(&[b"k1", b"k2", b"k3", b"k4", b"k5", b"l", b"m"])
    );
    let n = Node::new(&p).unwrap();
    assert_eq!(n.count(), 3);
    assert_eq!(
        n.item(0),
        Ok(posting(b"k", &[b"1", b"2", b"3", b"4", b"5"]))
    );
}

#[test]
fn a_full_posting_is_cut_around_an_entry_inside_it() {
    let rest: Vec<Vec<u8>> = (0..200_u8).map(|i| vec![i, 0]).collect();
    let mut items = vec![Item {
        key: b"k".to_vec(),
        child: None,
        rest: rest.clone(),
    }];
    let cap = items[0].cost();
    let added = leaf::add(&mut items, &[b'k', 100, 1], cap);
    assert_eq!(added, leaf::Added::Cut(1));
    let entries: Vec<Vec<u8>> = items.iter().flat_map(Item::entries).collect();
    let mut expected: Vec<Vec<u8>> = rest.iter().map(|r| [&b"k"[..], r].concat()).collect();
    expected.insert(101, vec![b'k', 100, 1]);
    assert_eq!(entries, expected);
    assert!(items.iter().all(|it| it.cost() <= cap));
}

#[test]
fn removing_a_postings_last_remainder_removes_its_cell() {
    let mut p = leaf_of(&[posting(b"k", &[b"1", b"2"]), leaf(b"z")]);
    let find = |p: &[u8], x: &[u8]| leaf::find(&Node::new(p).unwrap(), x).unwrap();
    let at = find(&p, b"k2");
    leaf::remove(&mut p, at);
    assert_eq!(all(&p), strings(&[b"k1", b"z"]));
    let at = find(&p, b"k1");
    leaf::remove(&mut p, at);
    assert_eq!(all(&p), strings(&[b"z"]));
    assert_eq!(Node::new(&p).unwrap().count(), 1);
}

#[test]
fn dedup_keeps_each_run_of_one_key_once_under_the_cap() {
    let items: Vec<Item> = [&b"a1"[..], b"a2", b"a3", b"b1", b"c1", b"c2"]
        .iter()
        .map(|x| leaf(x))
        .collect();
    let packed = leaf::dedup(&items, &|_| Some(1), 1000);
    assert_eq!(
        packed,
        [
            posting(b"a", &[b"1", b"2", b"3"]),
            leaf(b"b1"),
            posting(b"c", &[b"1", b"2"])
        ]
    );
    let small = posting(b"a", &[b"1", b"2"]).cost();
    let capped = leaf::dedup(&items, &|_| Some(1), small);
    assert!(capped.iter().all(|it| it.cost() <= small));
    let entries: Vec<Vec<u8>> = capped.iter().flat_map(Item::entries).collect();
    let flat: Vec<Vec<u8>> = items.iter().flat_map(Item::entries).collect();
    assert_eq!(entries, flat);
    assert_eq!(leaf::dedup(&items, &|_| None, 1000), items);
}

#[test]
fn a_posting_with_no_remainders_or_in_an_inner_page_is_malformed() {
    let p = leaf_of(&[posting(b"k", &[b"1"])]);
    let at = usize::from(u16::from_le_bytes([p[42], p[43]]));
    let mut empty = p;
    empty[at + 3..at + 5].copy_from_slice(&0_u16.to_le_bytes());
    assert_eq!(Node::new(&empty).unwrap().rest(0), Err(Malformed));
    let mut inner = blank(PageKind::IndexInner);
    let bounds = node::Bounds {
        low: None,
        high: None,
        right: None,
    };
    node::build(&mut inner, 1, &[Item::link(b"k".to_vec(), Pid(5))], bounds);
    let at = usize::from(u16::from_le_bytes([inner[42], inner[43]]));
    inner[at + 1] |= 0x80;
    assert_eq!(Node::new(&inner).unwrap().child(0), Err(Malformed));
}

#[test]
fn a_separator_is_the_shortest_prefix_of_the_right_key_above_the_left() {
    assert_eq!(separator(b"apple", b"apricot"), b"apr");
    assert_eq!(separator(b"ab", b"abc"), b"abc");
    assert_eq!(separator(b"ab", b"abcd"), b"abc");
    assert_eq!(separator(b"a", b"b"), b"b");
    assert_eq!(separator(b"", b"x"), b"x");
    for (l, r) in [
        (&b"apple"[..], &b"apricot"[..]),
        (b"ab", b"abcd"),
        (b"\x00", b"\x00\x00"),
    ] {
        let s = separator(l, r);
        assert!(l < s.as_slice() && s.as_slice() <= r);
    }
}

#[test]
fn a_page_of_another_format_is_not_a_node() {
    let mut p = vec![0; PAGE];
    p[FIRST_EDITABLE..HEADER_LEN].copy_from_slice(&header_edit(1, PageKind::IndexLeaf, 9, 0));
    assert_eq!(FORMAT, 2);
    assert_eq!(Node::new(&p).err(), Some(Malformed));
}

#[test]
fn index_key_limits_include_encoding_constructor_width_and_tid() {
    use crate::key::{CtorId, Dir, Entry, KeySchema, KeyType, Scalar, Tid, Value};

    for page_size in [4096, 8192, 16384, 32768] {
        for id in [239, 240, 255, 256, 65535, 65536, u32::MAX] {
            let ctor = CtorId(id);
            let schema = KeySchema::new(vec![(KeyType::Scalar(Scalar::Bytes), Dir::Asc)]);
            let index = super::Index::open(BTree::open(Pid(1), 1), schema, [ctor].into());
            let mut entry = Entry {
                key: vec![Value::Bytes(Vec::new())],
                ctor,
                tid: Tid::new(1, 0).unwrap(),
            };
            let max = BTree::max_entry_len(page_size);
            let overhead = index.schema().encode_entry(&entry).len();
            entry.key = vec![Value::Bytes(vec![1; max - overhead])];
            assert_eq!(index.schema().encode_entry(&entry).len(), max);
            assert_eq!(index.check_key(page_size, &entry.key, ctor), Ok(()));
            entry.key = vec![Value::Bytes(vec![1; max - overhead + 1])];
            assert_eq!(
                index.check_key(page_size, &entry.key, ctor),
                Err(super::Error::TooLong { len: max + 1, max })
            );
            entry.key = vec![Value::Bytes(vec![0; (max - overhead) / 2 + 1])];
            assert!(index.check_key(page_size, &entry.key, ctor).is_err());
        }
    }
}
