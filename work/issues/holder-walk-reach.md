---
created-at: 2026-11-04
status: open
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
