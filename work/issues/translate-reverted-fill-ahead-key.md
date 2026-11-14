---
created-at: 2026-11-14
status: open
kind: bug
priority: 320
summary: "translating a reverted fill whose terms hold a key of a relation the revert adds later may panic in Schema::declared"
tags:
  - engine
  - class-failure-flattened
---

# Translating a reverted fill holding a later relation's key may panic

## Report

`Pinned::over(fill.schema, ..)` for an `Origin::Reverted` `AddField` fill (crates/adtb-lang/src/translate/mod.rs) gets the revert step's schema, whose `ahead` holds the relations the revert adds later. crates/adtb-lang/src/translate/map.rs:208 (`ty_at`), :344 and :346 (`whole_in`) and walk.rs:894 (`key_value`) call `Schema::declared` on the term types they meet, which panics on a key of such a relation. The checker refuses deref, `value`, `match` over the key and `T.parse` through one, but admits a fill whose terms hold the type otherwise: a match over `o.r : Option (Key zs)` with `Some { value: _ }`, or `print(o.r)`. Found in the ahead-declared track; not reproduced.

Direction: translate treats such a key as reaching nothing (the 6b rule) or as a loss, instead of calling `declared`; or the committed-schema-type ticket makes the call unreachable by type. Test: a bundle pinned before a revert whose fill matches over such a field, as a bundles script.

Defect class: Damage or failure gives a panic, hang or flattened error.
