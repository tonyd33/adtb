---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 795
summary: "catalog ids are bare u64 in tuples of different meaning, and Root.changing packs none, an Xid and a naming marker into one u64"
closed: 2026-10-26
tags:
  - quality
---

# Feature: newtype catalog ids and a typed root change marker

## Report

Catalog ids are bare `u64` (`crates/adtb-lang/src/ids.rs:15-19`), and the engine keys maps by tuples of them whose meaning differs per map. `crates/adtb/src/catalog/change.rs` `Moved::fills` is keyed `(relation id, constructor id, field id)` and `Moved::values` (change.rs:121) `(type id, constructor id, field id)`, both `(u64, u64, u64)`; swapping one for the other compiles. `crates/adtb/src/catalog/moved.rs` builds both keys by hand at moved.rs:145, :233 and :517. `crates/adtb/src/catalog/layout.rs:110` `ctor_id` narrows a `u64` to `u32` with no type saying which id it is.

`crates/adtb/src/catalog/root.rs:8` adds `NAMING_SEQUENCES = u64::MAX`, so `Root::changing` is 0 for no change, an xid for a schema change, and `u64::MAX` while the counters page is named.

Fix: newtypes per catalog id kind (`CatRelId`, `CatCtorId`, `CatFieldId`, `CatTypeId`) in ids.rs, used by `Moved` and the catalog's heap and index maps; an in-memory `enum Changing { None, Schema(Xid), NamingSequences }` on `Root`, encoded to the same `u64` on the page.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed in wqmnkqkx, within the adtb crate and its tests. What remains in adtb-lang is `lang-id-follow-ups`.
