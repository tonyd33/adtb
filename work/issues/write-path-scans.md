---
created-at: 2026-10-25
status: open
kind: feature
priority: 724
summary: "delete actions, reference checks after a delete, upserts on a unique and load values scan whole relations"
tags:
  - quality
  - engine
---

# Feature: index-bounded delete actions and upserts

## Report

The write path finds rows by scanning whole constructor heaps where an index would bound the work:

- `Stmt::boundary` (crates/adtb/src/write/eval.rs:801, 824) scans every row of every referrer relation once per cascade round and again for clear and retag actions, for any delete.
- `Stmt::check` (crates/adtb/src/write/check.rs:144) scans every referrer row when a target row is deleted or retagged.
- `Stmt::conflicts` (eval.rs:552) with `Conflict::Unique` scans the unique's scope once per upserted row, although the unique has an index.
- `Stmt::load` (eval.rs:453) scans `into` for the largest `load`.

A delete of one row in a relation referenced by a large one costs a full scan per statement. Fix: probe an index over each reference field, probe the unique's index in `conflicts`, and drop the `load` scan, since the `LOADS` counter already never reissues a value. Overlaps constraint-maintenance only for the reference check.

## 2026-10-25: from constraint maintenance

The native "moved target" reference check and delete-action cascades scan every referrer row, which breaks the large change set bound of `docs/design/maintenance.md` section 6. Moving them onto the delta compiler waits on `reference-index-rule`. Loads build no index from sorted runs either, so uniqueness at build does not apply yet.
