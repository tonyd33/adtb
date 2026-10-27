---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 703
summary: "catalog reads bypass snapshots, heap ids and order keys are not persisted, and a redone create reuses xids"
closed: 2026-10-26
depends-on:
  - bootstrap-catalog
tags:
  - engine
---

# Feature: Bootstrap catalog follow-ups

## Report

- `Catalog::committed_rows` reads committed rows without a snapshot;
  catalog reads should go through snapshots like any other.
- Heap ids are numbered in memory at create, and the next free id is
  not stored, so a schema change has nothing to continue from. Pages a
  failed schema-change transaction allocated would leak.
- Index order keys are recomputed with `OrderKey::spread` on every
  open; `docs/design/indexes.md` §4 says the catalog keeps them.
- Redoing an unfinished create makes a fresh commit log, so xid 1 is
  assigned again. Nothing durable refers to the old one, but keep the
  next xid across the redo.
- `catalog/layout.rs` duplicates the type conversion in
  `crates/adtb-sim/tests/exec.rs`; make the test use the catalog's.

## 2026-10-25: progress

Done on main: catalog reads go through snapshots, the next heap id is stored, and kept order keys are read back rather than respread. Unverified: the next xid across a redone create, and the type conversion duplicated between `catalog/layout.rs` and the sim exec tests.

## 2026-10-26: done

All five items verified with tests in `crates/adtb-sim/tests/catalog.rs`. Catalog reads go through snapshots: `a_snapshot_taken_before_a_migration_reads_the_catalog_rows_of_the_version_before`. The stored next heap id: `a_migration_after_a_reopen_gives_its_new_heaps_ids_no_heap_had`. Kept order keys: `a_migrated_catalog_reopens_to_the_same_structures`. The next xid across a redone create: the redo opens the kept commit log, whose open assigns no id assigned before the crash; `a_crash_at_every_flush_during_create_reopens_to_a_whole_catalog` now checks that a create redone over a named catalog commits under an id above the uninterrupted create's. The type conversion: the sim tests use `catalog::layout` through `tests/support`. No engine code changed for this ticket.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed with the cat track.
