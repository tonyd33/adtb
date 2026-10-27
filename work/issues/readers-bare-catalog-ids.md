---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 858
summary: "reader pinning keys relations, constructors and constraints by bare u64 although CatRelId and CatCtorId exist"
closed: 2026-10-27
tags:
  - quality
  - engine
---

# Reader pinning passes catalog ids as bare u64 pairs

## Report

`Pinned` (`crates/adtb/src/catalog/readers.rs:56-72`), `blocking(cid: u64)` (`:297`), `in_set` (`:528`), `constraint_id` (`:596`) and the `uniques` keys (`:614`) hold catalog ids as `u64`. `writes: BTreeSet<(u64, u64)>` holds relation and constructor ids as a bare pair, so swapping them compiles. The crate has `CatRelId` and `CatCtorId` for this. `lang-id-follow-ups` covers adtb-lang only.

Fix: use the catalog newtypes, and add one for constraint ids if none exists.

## 2026-10-27: done

`Pinned`, `blocking`, `in_set`, `uniques` and the reader rules hold `CatRelId`, `CatCtorId`, `CatFieldId` and the new `CatConstraintId`, `CatIndexId` and `CatSetId` (`crates/adtb/src/catalog/ids.rs`). `constraint_id` is gone; `declares` matches by module and name.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qread track.
