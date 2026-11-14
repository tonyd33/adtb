---
created-at: 2026-11-14
status: open
kind: bug
priority: 330
summary: "catalog::indexes lists only the current schema's indexes, so a dropped index's row disappears instead of being kept with its dropped version"
tags:
  - engine
  - class-identity-by-spelling
---

# Dropped indexes vanish from the catalog

## Report

catalog.md §1.2: catalog rows are never deleted; a dropped object's row stays with its `dropped` version. After `rekey relation ps key end`, `catalog::indexes` no longer lists ids 1025 to 1028, and `dropped` is always None: crates/adtb/src/catalog/rows.rs builds index rows from the current schema only. Found in the review of the prelude-ref-relation track; reproduced, not kept. Check the other catalog relations built the same way.

Direction: keep catalog rows across versions (from the stored history), setting `dropped` when an object leaves; a test that drops an index and reads its row with `dropped`.

Defect class: Catalog rows and identity by position or spelling.
