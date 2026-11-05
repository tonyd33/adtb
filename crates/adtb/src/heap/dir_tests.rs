use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use adtb_sys::os::{Backend, OsRuntime};
use adtb_sys::runtime::Threads;
use adtb_testkit::scratch::Scratch;

use crate::heap::{Error, Fate, FieldType, Heap, LargeStore};
use crate::key::{Scalar, Value};
use crate::storage::{Damage, Options, PageError, PageKind, Pager, Pid, Quarantine, Stopped};

use super::super::page::{self, Arrangement, CONTENTS};
use super::super::stage::Stage;
use super::super::vm::{self, Vm};
use super::{CAPACITY, COUNT, LEVEL, Space, VM, entry, max_fanout, set_word};

fn pager(scratch: &Scratch) -> Pager<OsRuntime> {
    let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
    Pager::create(rt, &Options::new("dir.adt")).unwrap()
}

/// Returns a space of `pages` pages whose directory pages hold two entries each.
fn space(pager: &Pager<OsRuntime>, pages: usize) -> Space {
    let space = Space::create(pager, 7, 2, None).unwrap();
    for _ in 0..pages {
        space
            .append(pager, space.root(), Arrangement::Slotted, 1)
            .unwrap();
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

/// Returns a heap whose map has a root at level 1 over two leaves, and its directory's root then
/// its map's pages.
fn heap_over_two_leaves(pager: &Pager<OsRuntime>, id: u32) -> (Heap, Vec<Pid>) {
    let large = LargeStore::create(pager, id + 100).unwrap();
    let row = vec![FieldType::Scalar(Scalar::I64)];
    let heap = Heap::create_with_fanout(pager, id, row, large, 2).unwrap();
    let map = Space::open(id, heap.root()).vm(&mut Stage::new(), pager);
    let reach = vm::reach(pager.page_size(), 0);
    Vm::open(map.unwrap().unwrap(), id)
        .extend(pager, Pid(reach))
        .unwrap();
    let owned = heap.owned_pages(pager).unwrap();
    assert_eq!(
        4,
        owned.len(),
        "a directory root, a map root and two leaves"
    );
    (heap, owned)
}

fn set_map(pager: &Pager<OsRuntime>, root: Pid, map: Pid) {
    pager
        .update(&[root], |p| {
            p[0][VM..VM + 8].copy_from_slice(&map.0.to_le_bytes());
        })
        .unwrap();
}

/// Zero every byte of `pid` in memory, unlogged.
fn zero(pager: &Pager<OsRuntime>, pid: Pid) {
    let zeros = vec![0; pager.page_size()];
    pager.exclusive(pid).unwrap().inner().write(0, &zeros);
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
    let got = s.append(&pager, s.root(), Arrangement::Slotted, 1);
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
        let owned = s.owned(&pager).unwrap();
        assert_eq!(
            11,
            owned.len(),
            "{what}: six directory pages, the root at level 2"
        );
        let nodes = &owned[..6];
        assert_eq!(5, s.pages(&pager).unwrap().len(), "{what}");
        for &i in at {
            put_u16(&pager, nodes[i], offset, value);
        }
        let damaged = pager.damaged();
        assert!(matches!(s.held(&pager), Err(Stopped::Shape)), "{what}");
        assert_eq!(damaged, pager.damaged(), "{what}: the walk recorded damage");
        let got = s.append(&pager, s.root(), Arrangement::Slotted, 1);
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
fn a_space_opened_at_page_zero_reads_nothing_and_takes_no_page() {
    let scratch = Scratch::new("heap-dir", "placeholder");
    let pager = pager(&scratch);
    let large = LargeStore::open(0, Pid(0));
    let row = vec![FieldType::Scalar(Scalar::I64)];
    let heap = Heap::open(&pager, 9, Pid(0), row, large).unwrap();
    assert!(matches!(large.held_pages(&pager).as_deref(), Ok([])));
    assert!(matches!(heap.held_pages(&pager).as_deref(), Ok([])));
    assert_eq!(Ok(Vec::new()), heap.owned_pages(&pager));
    assert_eq!(Ok(Vec::new()), large.owned_pages(&pager));
    assert_eq!(Ok(Vec::new()), heap.verify(&pager));
    assert_eq!(Ok(Vec::new()), large.verify(&pager));
    assert_eq!(Ok(Vec::new()), heap.unsettled(&pager));
    let insert = heap.insert(&pager, 1, &[Value::I64(1)]);
    assert_eq!(Err(Error::NoDirectory), insert.map(drop));
    let s = Space::open(9, Pid(0));
    assert_eq!(Ok(None), s.find(&pager, 1, Arrangement::Slotted, 1));
    assert_eq!(Ok(false), s.names(&pager, 0, Pid(5)));
    assert_eq!(Ok(Vec::new()), s.touched(&pager));
    assert_eq!(Ok(()), s.settle(&pager));
    assert!(pager.damaged().is_empty(), "{:?}", pager.damaged());
    pager.close();
}

#[test]
fn a_heap_whose_root_names_no_map_fails_verify() {
    let scratch = Scratch::new("heap-dir", "no-map");
    let pager = pager(&scratch);
    let (heap, _) = heap_over_two_leaves(&pager, 7);
    set_map(&pager, heap.root(), Pid(0));
    let want = vec![format!("{:?} names no visibility map", heap.root())];
    assert_eq!(Ok(want), heap.verify(&pager));
    assert_eq!(refused(heap.root()), heap.all_visible(&pager, Pid(5)));
    pager.close();
}

#[test]
fn a_map_pointer_out_of_shape_stops_a_holder_walk_and_another_heaps_damage_marks_that_heap() {
    let scratch = Scratch::new("heap-dir", "map-shape");
    let pager = pager(&scratch);
    let (a, _) = heap_over_two_leaves(&pager, 7);
    let (b, b_map) = heap_over_two_leaves(&pager, 8);
    let damaged = pager.damaged();
    let cases = [
        ("its own directory root", a.root()),
        ("another heap's directory root", b.root()),
        ("another heap's map root", b_map[1]),
    ];
    for (what, named) in cases {
        set_map(&pager, a.root(), named);
        let held = a.held_pages(&pager);
        assert!(matches!(held, Err(Stopped::Shape)), "{what}: {held:?}");
        assert_eq!(damaged, pager.damaged(), "{what}: the walk recorded damage");
    }
    let e = pager.damage(b_map[3], Damage::Checksum);
    assert_eq!(b_map[3], e.pid);
    let holders = pager.holders_of(b_map[3], None, |h| {
        h.walk(Some(Quarantine::Heap(7)), || a.held_pages(&pager));
        h.walk(Some(Quarantine::Heap(8)), || b.held_pages(&pager));
    });
    assert_eq!(BTreeSet::from([Quarantine::Heap(8)]), holders);
    assert_eq!(
        refused(a.root()),
        a.verify(&pager),
        "a map root of another heap"
    );
    pager.close();
}

/// Returns what `f` returns on another thread, failing past 3 s.
fn within<T: Send + 'static>(rt: &OsRuntime, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (sent, got) = mpsc::channel();
    rt.spawn("bounded", move || sent.send(f()).unwrap());
    got.recv_timeout(Duration::from_secs(3))
        .expect("an answer within 3 s")
}

#[test]
fn a_map_root_naming_itself_in_child_slot_0_stops_each_walk_within_a_bound() {
    let scratch = Scratch::new("heap-dir", "map-cycle");
    let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
    let pager = Arc::new(Pager::create(rt.clone(), &Options::new("dir.adt")).unwrap());
    let slot_0 = CONTENTS + 8;
    let mut heaps = Vec::new();
    for id in [7, 8, 9, 10] {
        let (heap, map) = heap_over_two_leaves(&pager, id);
        let named = map[1].0.to_le_bytes();
        pager
            .update(&[map[1]], |p| {
                p[0][slot_0..slot_0 + 8].copy_from_slice(&named);
            })
            .unwrap();
        heaps.push((Arc::new(heap), map[1]));
    }
    let (p, h) = (pager.clone(), heaps[0].0.clone());
    let held = within(&rt, move || h.held_pages(&p));
    assert!(matches!(held, Err(Stopped::Shape)), "{held:?}");
    let (p, h) = (pager.clone(), heaps[0].0.clone());
    assert_eq!(refused(heaps[0].1), within(&rt, move || h.owned_pages(&p)));
    let (p, h) = (pager.clone(), heaps[1].0.clone());
    assert_eq!(refused(heaps[1].1), within(&rt, move || h.verify(&p)));
    let (p, h) = (pager.clone(), heaps[2].0.clone());
    let visible = within(&rt, move || h.all_visible(&p, Pid(5)));
    assert_eq!(refused(heaps[2].1), visible);
    let (p, (id, root)) = (pager, (heaps[3].0.id().0, heaps[3].1));
    let extended = within(&rt, move || Vm::open(root, id).extend(&p, Pid(5)));
    assert_eq!(refused(root), extended);
}

#[test]
fn an_all_zero_page_an_entry_names_stops_a_holder_walk_and_refuses_the_page_naming_it() {
    let scratch = Scratch::new("heap-dir", "zero");
    let pager = pager(&scratch);
    let s = space(&pager, 3);
    let owned = s.owned(&pager).unwrap();
    assert_eq!(
        6,
        owned.len(),
        "a root at level 1, two leaves and three pages"
    );
    zero(&pager, owned[2]);
    let damaged = pager.damaged();
    assert!(
        matches!(s.held(&pager), Err(Stopped::Shape)),
        "a directory leaf"
    );
    assert_eq!(damaged, pager.damaged(), "the walk recorded damage");
    assert_eq!(refused(s.root()), s.pages(&pager));

    let s = space(&pager, 1);
    let page = s.pages(&pager).unwrap()[0];
    zero(&pager, page);
    assert_eq!(
        refused(s.root()),
        s.find(&pager, 1, Arrangement::Slotted, 1)
    );

    for (what, at, blamed) in [("a map leaf", 3, 1), ("a map root", 1, 0)] {
        let (heap, map) = heap_over_two_leaves(&pager, 20 + at as u32);
        zero(&pager, map[at]);
        let damaged = pager.damaged();
        let held = heap.held_pages(&pager);
        assert!(matches!(held, Err(Stopped::Shape)), "{what}: {held:?}");
        assert_eq!(damaged, pager.damaged(), "{what}: the walk recorded damage");
        assert_eq!(refused(map[blamed]), heap.owned_pages(&pager), "{what}");
    }
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

#[test]
fn a_map_pointer_naming_the_page_prune_holds_refuses_the_directory_root_within_a_bound() {
    let scratch = Scratch::new("heap-dir", "prune-latch");
    let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
    let pager = Arc::new(Pager::create(rt.clone(), &Options::new("dir.adt")).unwrap());
    let (heap, _) = heap_over_two_leaves(&pager, 7);
    let page = Pid(heap.insert(&pager, 1, &[Value::I64(1)]).unwrap().page());
    let root = heap.root();
    set_map(&pager, root, page);
    let p = pager;
    let keep = |_: &Pager<OsRuntime>, _: &_| Ok(Fate::Keep);
    let pruned = within(&rt, move || {
        heap.prune(&p, page, keep, &[], false).map(drop)
    });
    assert_eq!(refused(root), pruned);
}

#[test]
fn verify_names_the_slot_table_fault_of_a_page_relabelled_to_the_other_heap_kind() {
    let scratch = Scratch::new("heap-dir", "relabel");
    let pager = pager(&scratch);
    let large = LargeStore::create(&pager, 107).unwrap();
    let heap = Heap::create(&pager, 7, vec![FieldType::Scalar(Scalar::I64)], large).unwrap();
    let data = Pid(heap.insert(&pager, 1, &[Value::I64(1)]).unwrap().page());
    large.write(&pager, 1, &[1; 100]).unwrap();
    let chunk = large.pages(&pager).unwrap()[0];
    put_u16(&pager, data, 14, PageKind::HeapSlotted as u16);
    put_u16(&pager, chunk, 14, PageKind::HeapFixed as u16);
    let frame = |pid| {
        page::check(&pager.read(pid).unwrap())
            .into_iter()
            .map(move |w| (pid, w))
    };
    let (data_fault, chunk_fault): (Vec<_>, Vec<_>) =
        (frame(data).collect(), frame(chunk).collect());
    assert_eq!(1, data_fault.len(), "{data_fault:?}");
    assert_eq!(1, chunk_fault.len(), "{chunk_fault:?}");
    assert_eq!(data_fault, heap.check(&pager).unwrap());
    assert_eq!(chunk_fault, large.check(&pager).unwrap());
}

#[test]
fn a_refusal_rechecks_an_entry_whose_leaf_changed_under_its_check() {
    for restored in [false, true] {
        let scratch = Scratch::new("heap-dir", "recheck");
        let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
        let pager = Arc::new(Pager::create(rt.clone(), &Options::new("dir.adt")).unwrap());
        let s = space(&pager, 1);
        let leaf = s.root();
        let (page, category) = entry(&pager.read(leaf).unwrap(), 0);
        let vm = pager.allocate(1, PageKind::VisibilityMap, 7).unwrap();
        pager
            .update(&[leaf], |p| set_word(p[0], 0, vm, category, false))
            .unwrap();
        let held = pager.exclusive(vm).unwrap();
        let (sent, answer) = mpsc::channel();
        let p = pager.clone();
        rt.spawn("refusal", move || {
            sent.send(s.refuse_misnamed(&p, vm)).unwrap();
        });
        std::thread::sleep(Duration::from_millis(200));
        let now = if restored { page } else { vm };
        pager
            .update(&[leaf], |p| set_word(p[0], 0, now, category, true))
            .unwrap();
        drop(held);
        let want = if restored { Ok(false) } else { refused(leaf) };
        let got = answer.recv_timeout(Duration::from_secs(3));
        assert_eq!(Ok(want), got.map_err(|e| format!("{e:?}")), "{restored}");
    }
}

#[test]
fn a_refusal_takes_the_position_from_the_path_past_a_short_leaf() {
    let scratch = Scratch::new("heap-dir", "short-leaf");
    let pager = pager(&scratch);
    let s = space(&pager, 4);
    let root = pager.read(s.root()).unwrap().to_vec();
    let (first, second) = (entry(&root, 0).0, entry(&root, 1).0);
    let named = entry(&pager.read(second).unwrap(), 0).0;
    assert_eq!(s.pages(&pager).unwrap()[2], named);
    put_u16(&pager, first, COUNT, 1);
    put_u16(&pager, named, 14, PageKind::HeapFixed as u16);
    assert!(!page::is_heap(&pager.read(named).unwrap()));
    assert_eq!(refused(named), s.refuse_misnamed(&pager, named));
}

#[test]
fn a_scan_refuses_the_leaf_naming_its_page_at_a_second_position() {
    let scratch = Scratch::new("heap-dir", "twice");
    let pager = pager(&scratch);
    let large = LargeStore::create(&pager, 107).unwrap();
    let heap = Heap::create(&pager, 7, vec![FieldType::Scalar(Scalar::I64)], large).unwrap();
    let data = Pid(heap.insert(&pager, 1, &[Value::I64(1)]).unwrap().page());
    let root = heap.root();
    put_u16(&pager, root, COUNT, 2);
    pager
        .update(&[root], |p| set_word(p[0], 1, data, 0, false))
        .unwrap();
    assert_eq!(refused(root), heap.scan(&pager, |_| {}));
}

#[test]
fn a_root_entry_past_the_last_position_refuses_the_root_within_a_bound() {
    let scratch = Scratch::new("heap-dir", "past-positions");
    let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
    let mut options = Options::new("dir.adt");
    options.page_size = 32768;
    let pager = Arc::new(Pager::create(rt.clone(), &options).unwrap());
    let s = Space::create(&pager, 7, 2048, None).unwrap();
    s.append(&pager, s.root(), Arrangement::Slotted, 1).unwrap();
    let data = s.pages(&pager).unwrap()[0];
    let fresh = || pager.allocate(1, PageKind::FreeSpaceMap, 7).unwrap();
    let (inner, leaf, empty) = (fresh(), fresh(), fresh());
    let node = |pid, level: u16, entries: &[Pid]| {
        let n = u16::try_from(entries.len()).unwrap();
        for (at, v) in [(LEVEL, level), (CAPACITY, 2048), (COUNT, n)] {
            put_u16(&pager, pid, at, v);
        }
        pager
            .update(&[pid], |p| {
                entries
                    .iter()
                    .enumerate()
                    .for_each(|(i, &e)| set_word(p[0], i, e, 0, false));
            })
            .unwrap();
    };
    node(leaf, 0, &[data]);
    node(inner, 1, &[leaf]);
    node(empty, 1, &[]);
    let mut root = vec![empty; 1025];
    (root[0], root[1024]) = (inner, inner);
    node(s.root(), 2, &root);
    let p = pager.clone();
    assert_eq!(refused(s.root()), within(&rt, move || s.pages(&p)));
}

/// Push `path::function` for each function under `dir` that records damage or builds a
/// `Damage`, skipping the storage layer and test files.
fn damage_sites(src: &std::path::Path, dir: &std::path::Path, out: &mut BTreeSet<String>) {
    for e in std::fs::read_dir(dir).expect("a source directory is readable") {
        let path = e.expect("a directory entry").path();
        let at = path
            .strip_prefix(src)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if path.is_dir() {
            if at != "storage" {
                damage_sites(src, &path, out);
            }
            continue;
        }
        if at.ends_with("tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("a source is readable");
        let mut function = String::new();
        for line in text.lines() {
            if let Some((_, rest)) = line.split_once("fn ") {
                let name = rest
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .next();
                function = name.unwrap_or_default().to_owned();
            }
            let builds = ["(", "damage: "].iter().any(|before| {
                ["Damage::", "storage::Damage::", "crate::storage::Damage::"]
                    .iter()
                    .any(|d| line.contains(&format!("{before}{d}")))
            });
            if line.contains(".damage(") || builds {
                out.insert(format!("{at}::{function}"));
            }
        }
    }
}

#[test]
fn damage_is_recorded_outside_storage_only_by_these_functions() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = BTreeSet::new();
    damage_sites(&src, &src, &mut found);
    let expected = [
        "api/error.rs::a_damaged_page_met_writing_catalog_rows_is_a_storage_failure",
        "api/mod.rs::damage_page",
        "api/verify.rs::damage_kept_store",
        "heap/dir.rs::refuse",
        "heap/large.rs::chunks",
        "heap/large.rs::read",
        "heap/large.rs::update",
        "heap/vm.rs::child",
        "mvcc/clog.rs::read_role",
        "mvcc/clog.rs::unresolved",
        "write/sequence.rs::open",
        "write/verify.rs::every_other_problem_is_a_finding",
    ];
    assert_eq!(BTreeSet::from(expected.map(String::from)), found);
}
