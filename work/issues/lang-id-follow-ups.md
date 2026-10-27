---
created-at: 2026-10-26
status: open
kind: feature
priority: 786
summary: "CatalogIds in adtb-lang still holds bare u64 ids, and norm.rs present_step still carries a too_many_arguments expectation"
tags:
  - quality
  - lang
---

# Feature: Catalog id newtypes and argument structs in adtb-lang

## Report

Left by the catalog quality track, which could not touch adtb-lang:
- The adtb crate wraps catalog ids in `CatRelId`, `CatCtorId`, `CatFieldId` and `CatTypeId` (`catalog/ids.rs`) at lookup, but `CatalogIds` itself holds bare `u64`. Move the newtypes into adtb-lang and use them in `CatalogIds`.
- `present_step` in `norm.rs` still has its `too_many_arguments` expectation.

`fold` now type-checks the stored log, so a log that builds but does not check fails `Catalog::open` with `Error::Unaccepted`.
