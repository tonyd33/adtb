---
created-at: 2026-10-29
status: in-progress
kind: bug
priority: 832
summary: "the permission analyzer takes the product of change paths over a write sequence, so N updates that may match nothing give 2^N paths"
branch: permission-fixes
tags:
  - quality
  - lang
---

# The permission analyzer enumerates write paths exponentially

## Report

`parallel` (`crates/adtb-lang/src/permission/write.rs:212-235`) takes the cartesian product of each write's `Shape::Change` paths, and the checker lowers every `WriteKind::Seq` to `Kind::Par` (`crates/adtb-lang/src/check/write.rs:452`). A `replace` that may match nothing contributes 2 paths (`write.rs` about 94), a write-level `if` joins both arms' paths and an upsert adds one per new, old and otherwise combination, so a sequence of N such updates clones 2^N paths. `Value::join` deduplicates paths with a linear `contains` (`value.rs` about 228), quadratic in the path count. Only an upsert's `otherwise` (`upsert_bound`) reads the paths.

Not measured.

Fix: keep paths only where an upsert needs them, or merge `Par` and `If` into one change set. Add a test that analyzes a transaction of 30 updates and finishes.

## 2026-10-29: dispatched

A fresh writer in workspace permission-fixes on main 2125596c959f fixes this with permission-keyof-join-panic and permission-provider-follow-ups: no exponential product of change paths, no under-charge, a 30-update transaction test. Gates: adtb-lang suite, clippy, fmt, house-check, size-check; the lead runs the full check.
