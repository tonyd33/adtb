---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 792
summary: "api/text.rs has_write copies the kernel's write test from analysis/volatile.rs and drifts from it on fail"
closed: 2026-10-26
tags:
  - quality
---

# Feature: one kernel predicate for whether a term writes

## Report

`has_write` in crates/adtb/src/api/text.rs:169 repeats `is_write` and `has_write` in crates/adtb-lang/src/analysis/volatile.rs:17-31, which are private. The copies already differ: the language's counts `Kind::Fail` as a write and the engine's does not, so ad hoc text whose only effect is a `fail` is treated as read-only by `Database::query_text` and never rerun on a serialization failure. The next write kind added to the kernel has to be added in both places.

Fix: make one pub predicate in adtb-lang (on `Terms` or in `kernel`), decide there whether `fail` counts, and call it from `Prepared::new` and from the volatile analysis.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as xtksymmw.
`fail` counts as a write: text whose only effect is a `fail` takes the write path and is refused in a read-only transaction.
