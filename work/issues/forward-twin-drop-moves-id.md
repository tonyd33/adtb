---
created-at: 2026-11-13
status: open
kind: bug
priority: 310
summary: "dropping the first of two identical clauses gives the surviving clause the dropped one's id"
tags:
  - class-identity-by-spelling
---

# Dropping one of two identical clauses moves the survivor's id

## Report

With two identical clauses (`index (n) in ts.B` twice, ids i1 then i2), `drop index (n) in ts.B` drops the first, and the survivor takes i1: twins share an object key and differ only by ordinal, and the reconcile matches by key. catalog.md §1.2 keeps a kept object's id. The revert path avoids this since structure-lang-identity step 5c-3 (`Source::forget_id`); forward `drop index` and `drop unique` still have it.

Direction: forward drops retire the dropped id and shift later twins' ordinals as `forget_id` does, in one place both paths call. Test: the survivor keeps i2.

Defect class: Catalog rows and identity by position or spelling.
