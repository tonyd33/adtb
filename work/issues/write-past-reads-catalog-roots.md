---
created-at: 2026-11-10
status: open
kind: bug
priority: 850
summary: "write::past builds Slots::Versions from catalog::ROOTS, the catalog's heap id made pub(crate) for it"
tags:
  - quality
  - engine
  - class-none
---

# Bug: the write path reaches into the catalog's roots heap id

## Report

`pub(crate) const ROOTS` (crates/adtb/src/catalog/mod.rs:94) is read by crates/adtb/src/write/past.rs:157, `Slots::Versions(crate::catalog::ROOTS)`, so the write path knows the catalog's internal heap id.

Fix direction: the catalog hands the migration rows' `Slots` through `Retained`, and `ROOTS` returns to private.

Found in the quality review of `reviewed..main` at 7bf40103.
