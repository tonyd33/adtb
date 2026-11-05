use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use adtb_sys::os::{Backend, OsRuntime};
use adtb_sys::runtime::Threads;
use adtb_testkit::scratch::Scratch;

use crate::heap::LargeStore;
use crate::storage::{Damage, Options, PageError, PageKind, Pager, Pid, Stopped};

use super::super::page::Arrangement;
use super::super::stage::Stage;
use super::{CAPACITY, COUNT, LEVEL, Space, entry, max_fanout, set_word};

fn pager(scratch: &Scratch) -> Pager<OsRuntime> {
    let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
    Pager::create(rt, &Options::new("dir.adt")).unwrap()
}

/// Returns a space of `pages` pages whose directory pages hold two entries each.
fn space(pager: &Pager<OsRuntime>, pages: usize) -> Space {
    let space = Space::create(pager, 7, 2, None).unwrap();
    for _ in 0..pages {
        space.append(pager, Arrangement::Slotted, 1).unwrap();
    }
    space
}

fn put_u16(pager: &Pager<OsRuntime>, pid: Pid, at: usize, value: u16) {
    pager
        .update(&[pid], |p| {
            p[0][at..at + 2].copy_from_slice(&value.to_le_bytes());
        })
        .unwrap();
}

fn refused<T>(pid: Pid) -> Result<T, PageError> {
    Err(PageError {
        pid,
        damage: Damage::Checksum,
    })
}

#[test]
fn a_directory_page_out_of_shape_stops_a_holder_walk_and_refuses_the_page_naming_it() {
    let scratch = Scratch::new("heap-dir", "shape");
    let pager = pager(&scratch);
    let s = space(&pager, 0);
    put_u16(&pager, s.root(), LEVEL, 32);
    let got = s.append(&pager, Arrangement::Slotted, 1);
    assert_eq!(
        refused(s.root()),
        got,
        "an empty root at level 32 of capacity 2"
    );
    let wide = max_fanout(pager.page_size()) + 1;
    let all = [0, 1, 2, 3, 4, 5];
    let cases: [(&str, &[usize], usize, u16, usize); 5] = [
        ("a capacity past the fanout", &all, CAPACITY, wide, 0),
        ("more entries than its capacity", &[5], COUNT, 3, 4),
        ("a level not one below its parent's", &[4], LEVEL, 0, 0),
        ("a capacity not its parent's", &[4], CAPACITY, 3, 0),
        ("a root level no file needs", &[0], LEVEL, u16::MAX, 0),
    ];
    for (what, at, offset, value, blamed) in cases {
        let s = space(&pager, 5);
        let nodes = s.nodes(&pager).unwrap();
        assert_eq!(6, nodes.len(), "{what}: the root is at level 2");
        assert_eq!(5, s.pages(&pager).unwrap().len(), "{what}");
        for &i in at {
            put_u16(&pager, nodes[i], offset, value);
        }
        let damaged = pager.damaged();
        assert!(matches!(s.held(&pager), Err(Stopped::Shape)), "{what}");
        assert_eq!(damaged, pager.damaged(), "{what}: the walk recorded damage");
        let got = s.append(&pager, Arrangement::Slotted, 1);
        assert_eq!(refused(nodes[blamed]), got, "{what}");
    }
    pager.close();
}

#[test]
fn an_inner_entry_naming_a_staged_page_refuses_its_directory_page_and_keeps_the_stage() {
    let scratch = Scratch::new("heap-dir", "staged");
    let pager = pager(&scratch);
    let s = space(&pager, 3);
    let (root, last) = (s.root(), s.pages(&pager).unwrap()[2]);
    let (_, category) = entry(&pager.read(root).unwrap(), 1);
    pager
        .update(&[root], |p| set_word(p[0], 1, last, category, false))
        .unwrap();
    let mut stage = Stage::new();
    let end = pager.page_size() - 1;
    stage.page(&pager, last).unwrap()[end] = 9;
    let got = s.set_category(&mut stage, &pager, last, 2, 0).map(drop);
    assert_eq!(refused(root), got);
    assert_eq!(9, stage.read(&pager, last).unwrap()[end]);
    drop(stage);
    pager.close();
}

#[test]
fn a_space_rooted_at_page_zero_holds_no_page_and_damages_nothing() {
    let scratch = Scratch::new("heap-dir", "placeholder");
    let pager = pager(&scratch);
    let held = LargeStore::open(0, Pid(0)).held_pages(&pager);
    assert!(matches!(held.as_deref(), Ok([])), "{held:?}");
    assert_eq!(Ok(Vec::new()), Space::open(0, Pid(0)).pages(&pager));
    assert!(pager.damaged().is_empty(), "{:?}", pager.damaged());
    pager.close();
}

#[test]
fn a_search_meeting_an_entry_that_names_a_map_page_holds_no_latch_a_map_writer_waits_on() {
    let scratch = Scratch::new("heap-dir", "latch-order");
    let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
    let pager = Arc::new(Pager::create(rt.clone(), &Options::new("dir.adt")).unwrap());
    let s = space(&pager, 1);
    let (leaf, vm) = (
        s.root(),
        pager.allocate(1, PageKind::VisibilityMap, 7).unwrap(),
    );
    let (_, category) = entry(&pager.read(leaf).unwrap(), 0);
    pager
        .update(&[leaf], |p| set_word(p[0], 0, vm, category, false))
        .unwrap();
    let held = pager.exclusive(vm).unwrap();
    let (found, answer) = mpsc::channel();
    let p = pager.clone();
    rt.spawn("search", move || {
        found.send(s.find(&p, 1, Arrangement::Slotted, 1)).unwrap();
    });
    std::thread::sleep(Duration::from_millis(200));
    let (latched, leaf_held) = mpsc::channel();
    let p = pager.clone();
    rt.spawn("map writer", move || {
        let g = p.exclusive(leaf).unwrap();
        latched.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        drop(p.exclusive(vm).unwrap());
        drop(g);
    });
    leaf_held.recv().unwrap();
    drop(held);
    let got = answer.recv_timeout(Duration::from_secs(3));
    assert_eq!(Ok(refused(leaf)), got.map_err(|e| format!("{e:?}")));
}
