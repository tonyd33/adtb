---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 592
summary: the delta compiler panics on normal-form shapes the checker guarantees instead of returning an internal error
closed: 2026-10-26
tags:
  - quality
---

# Feature: internal errors in the delta compiler

## Report

`crates/adtb/src/write/delta.rs` panics on five shapes the normaliser guarantees (a top-level read not a `For` source, rows or change-set inputs or prior relations not sets, a top-level binder not a `For` or `Let`). Return `plan::Error::Internal` there, as the rest of the plan and write paths do.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as qporyrxu. An internal error in the delta compiler reaches the user as a refusal detail; that is filed as admission-internal-error.
