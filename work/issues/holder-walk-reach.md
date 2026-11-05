---
created-at: 2026-11-04
status: resolved
closed: 2026-11-04
kind: bug
priority: 430
summary: "Holder walks still trust the visibility map root and read all-zero pages as damage; the page-0 guard covers walks but not vm, verify, find, append or owned_pages"
---

# Holder walk shape checks stop at the directory

## Report

## Report

From the review of heap-dir r1, 2026-11-04. None reproduced.

- `Heap::held_pages` (crates/adtb/src/heap/handle.rs, the VM part) takes the visibility map root from the root's VM field with no kind or owner check, and `Vm::pages` (heap/vm.rs) follows the children of whatever page it reads. A bad VM pointer lists foreign pages as held by this heap, so another object's damage also marks it.
- `holds` and the holder walks read through `read_formatted`, which records an all-zero page as damage. An entry naming an all-zero page damages the named page instead of the directory page that names it, the class the heap-dir fix closed for foreign pages.
- The page-0 guard is in `Space::walk` only. `vm`, `verify`, `find`, `append`, `len`, `names`, `touched` and `Heap::owned_pages` still read the root of a placeholder `Heap::open(.., Pid(0), ..)` or `LargeStore::open(0, Pid(0))` (catalog/mod.rs, history.rs) and would damage pid 0. No current call path reaches this.
- `BTree::flip_owner` has no callers.

## Proposal

Give the VM walk the directory's shape check, returning `Stopped::Shape`. Read directory children through a path that blames the directory for an all-zero page. Keep placeholders out of `Space`: an `Option` root, or refuse `HOLE` at `Space::open`. Delete `flip_owner` or use it.

## 2026-11-04: fixed

`fix: check the visibility map's shape in holder walks and keep placeholders out of a space`. Class: identity trusted from an unvalidated pointer. The map root is checked for kind and owner (`Space::map_page`, through the caller's stage) and every map child for kind, owner and level one below its parent (`vm::shaped`, `Vm::child`) on every read path: holder walks (a shape stop, no damage), `leaf`/`get`/`put`, `marked`, `extend`; a page out of shape blames the page naming it. An all-zero directory or map page reached through an entry blames the entry's page. `Space.root` is `Option<Pid>`, so a space opened at page 0 reads nothing; inserting into one is `heap::Error::NoDirectory` (internal `Broken::HeapNoDirectory`). A real heap whose root names no map fails verify, and `Heap::vm` returns the root's refusal instead of panicking. `BTree::flip_owner` is deleted. Tests in heap/dir_tests.rs: map pointers out of shape; another heap's damage marks only that heap; all-zero inner, leaf, map leaf and map root; a self-naming map root answering within a bound on walk, verify, `all_visible` and `extend` (it looped on main); the placeholder; no map; a map pointer naming the page prune holds (it self-latched). Reviews: the level check was killed only by a timeout and verify and `all_visible` still hung on a self-naming map (fixed); a scoped latch review of `extend` found no race (1000-round stress) and one self-latch on damage, fixed. `check --changed` 3356 passed.

Left: an all-zero page reached by `check_node`, `node_mut` or `Stage::page` still damages the page named; a heap's map field naming another heap's damaged page directly marks both heaps (the read fails as damage before the shape check); `BTree::open(Pid(0))` in `Kept::unread` is a placeholder outside `Space`.
