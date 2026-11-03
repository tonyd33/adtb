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
