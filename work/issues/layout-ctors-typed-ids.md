---
created-at: 2026-10-26
status: open
kind: feature
priority: 787
summary: layout::ctors narrows catalog constructor ids to u32 and its consumers wrap them back into CatCtorId
tags:
  - quality
  - engine
---

# Feature: layout::ctors returns catalog constructor ids

## Report

`layout::ctors` (`crates/adtb/src/catalog/layout.rs:99`) narrows each id to a bare `u32` with `CatCtorId(*id).stored().0`, and its consumers `held_fills` (`crates/adtb/src/catalog/moved.rs:279`) and `held` (`crates/adtb/src/catalog/change.rs:271`) wrap it back with `CatCtorId(u64::from(*cid))`. The id crosses the newtype boundary and comes back.

Fix: return `CatCtorId` from `layout::ctors` and call `.stored()` only where a key or heap `CtorId` is built.
