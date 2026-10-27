---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 893
summary: "forward-reference placeholders are relations with elem_def u32::MAX, which any walk over schema.relations can index"
closed: 2026-10-26
tags:
  - quality
---

# Ahead placeholder sentinel

## Report

## Report

`crates/adtb-lang/src/schema.rs` `declare_ahead` adds placeholder relations with `elem_def` set to `u32::MAX`. Evolve code walking `schema.relations` without a guard panics mid-migration. Hold placeholders apart from relations, or give them a type the resolver refuses everywhere but `Key r`.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as rvnzupow.
