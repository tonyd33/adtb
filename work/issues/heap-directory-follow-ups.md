---
created-at: 2026-11-03
status: open
kind: bug
priority: 380
summary: "after the directory gate: a lying directory damages the innocent page it names, holder walks record shape failures as damage against storage.md, and the gate clauses past kind and owner are untested"
---

# Heap directory follow-ups

## Report

From the review of `fix: refuse a foreign page in a heap directory instead of spinning`, most severe first.

- `refuse` (crates/adtb/src/heap/dir.rs near 146) and `find` (near 414) damage the page refused, not the directory page that names it. A directory entry naming another relation's healthy page, after a lost write or a freed and reused page, quarantines that innocent relation, or the whole database when the pid is a bitmap or header page. directory_damage.rs copies the other heap's page into the users page's own pid, so it never reaches this.
- docs/design/storage.md (near line 815) says a holder walk that meets a shape it cannot follow proves nothing and retries. Holder walks now read through `node`, so a shape failure is recorded as `pager.damage` and counted as a damage stop by `Holders::walk` (crates/adtb/src/storage/quarantine.rs near 135); a damaged large-store directory page then escalates damage on any other page to the database mark. Code or design text must change; this is a boundary contract, so decide it against the design file.
- `holds` and `names` in `find` are two unlatched reads: a page vacated, freed, reused, freed again and refilled at the same position between them is damaged though healthy. Not reproduced. Re-read `holds` after `names` answers true, or check under the leaf's latch.
- `shaped` does not bound the root's level: a corrupt root near level 65535 overflows `cap.pow(height + 1)` in `append` and `len`, a panic in debug and a wrong position in release.
- `refuse` from `node` unstages every staged page with that pid, including one the caller's stage changed, against `unstage`'s precondition. Reached only on an error path today.
- `LargeStore::open(0, Pid(0))` placeholders (crates/adtb/src/catalog/mod.rs near 823, history.rs near 109) would damage pid 0 if walked; nothing guards it.
- Tests: removing the capacity range, `count <= cap` or the parent level and capacity clause from `shaped` leaves every test green; no directory in directory_damage.rs grows past height 0. The rewrite of `a_page_whose_holder_is_unknown...` into `a_large_value_page_maps_to_the_database_while_another_walk_stops` lost two end-to-end properties (a call failing as `Storage` naming the page with no mark while a walk is stopped on shape; the next read mapping the page once the shape is restored); restore it on an allocated unheld page. Its `flip_owner` is incidental: the test passes without the stopped walk. The server_binding window moved 360..366 to 366..372 because the coverage floor "a registration that lost" reached 1 of 2 after the schedule shifted; safety held on every old seed. Same class as picked-seed-floors.
- A users data page relabelled `HeapFixed` still accepts an insert, and reads of it then refuse: a heap page contents check, not a directory one.

## 2026-11-04: fixed, two items left

`fix: blame the directory page that names a foreign page and keep walk shapes out of damage`. Class: identity trusted from an unvalidated pointer. A directory entry naming a page not its heap's damages the directory page holding the entry (the parent, or the root), never the named page; `find` confirms the leaf still names the page by an optimistic read of the leaf validated after the page is read, one latch at a time. Holder walks go through `Space::held`, which reports a shape failure as `Stopped::Shape` and records no damage, as storage.md states. `shaped` refuses a root whose level needs `capacity^level` of 2^32 positions or more. A space rooted at page 0 walks nothing. Tests: an entry naming another heap's page, a bitmap page and its own directory page quarantines only its heap; a large-value page fails as Storage unmarked while its store walk stops on shape, then maps to the database once the shape is restored; each `shaped` clause on a height-2 directory and a root consistent except for the bound; a staged page kept when its parent is refused; a search meeting an entry naming a map page holds no latch a map writer waits on (times out on a two-latch body). A fix round's two-latch body hung `find` against prune; a scoped latch review caught it. Untested: the revalidation itself (no hook forces the window). Full check 3342 passed.

Left: the `HeapFixed` relabel bullet and the server_binding window note above. Follow-ups filed as holder-walk-reach.

## 2026-11-05: heap page kind fixed

`fix: refuse a heap page of the wrong kind on the write path as on the read path`. Class: damage reported as nothing. The ticket undersold it: reads did not refuse a relabelled page either; `page::is_heap` checked only the kind tag, so a slotted page relabelled fixed read as zero slots: a scan returned none of its rows with no error, an insert routed around it, an index read blamed the index, vacuum skipped it; a fixed page relabelled slotted panicked an insert. A heap page is now one whose kind is a heap kind and whose slot table fits the page (`fixed::framed` O(1), `slotted::framed`), on every read and write path through `is_heap`; `dead_slots` refuses a named page that is not the heap's, as `scan_pages` does. Blame on the write path: a page whose header names a heap kind, this space's owner and the entry's position is damaged itself; any other header blames the level-0 directory page. Verify reports a bad slot table as a frame fault again (`page::has_heap_kind` before `page::check`). Tests: heap_kind.rs (four relabellings on a fixed and a slotted heap; seven write forms, each asserting the exact damaged page, the heap-only quarantine and unchanged versions), unit tests for verify and the fixed frame; seven mutants killed. `check --changed` 3349 passed.

Left:
- The read path (`refuse_if_named` for scan and vacuum, `refuse_named` for index reads) still damages the named page when its header does not claim the position (a large-value or map page under a heap entry); under the heap-dir rule it would blame the directory page naming it.
- Page validity is tied to today's `capacity` formula for fixed pages: changing it would make every existing fixed page fail `is_heap`. A format change must version the slot-table layout.
- The server_binding window note above.
