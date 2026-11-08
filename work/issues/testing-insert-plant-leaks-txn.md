---
created-at: 2026-11-08
status: open
kind: bug
priority: 800
summary: "a Plant::Insert whose heap insert fails drops its write transaction without commit or abort, leaving its xid in progress for the rest of the test"
tags:
  - quality
  - engine
  - test
---

# A failed insert plant leaves its write transaction in progress

## Report

`Plant::Insert` (crates/adtb/src/testing/plant.rs:159) begins a write transaction at :162 and returns through `?` at :165 when a `heap.insert` fails, dropping the `write::Txn` without commit or abort. `write::Txn` (crates/adtb/src/write/mod.rs:344) has no `Drop` and no `#[must_use]`, so the xid stays in progress in the commit log: `Testing::tracked` counts one too many and the snapshot pins the vacuum horizon for the rest of the test.

Repro: plant `Refuse` on a page of a heap, then `Plant::Insert` into that heap; `tracked()` stays one higher afterwards.

Fix direction: abort the transaction on the error path, and testing.md §3.2 states that a failed plant leaves no transaction running.
