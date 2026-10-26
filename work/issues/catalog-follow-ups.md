---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 703
summary: "catalog reads bypass snapshots, heap ids and order keys are not persisted, and a redone create reuses xids"
branch: adtb-mig
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
