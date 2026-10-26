---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 406
summary: "the pager keeps the LSN of every page it ever evicted, in a table that never shrinks"
closed: 2026-10-25
depends-on:
  - vmcache
tags:
  - engine
---

# Bug: The evicted-page LSN table is unbounded

## Report

To catch a lost write when an evicted page is read back, the pager
keeps the LSN of every page it has evicted (`Meta::evicted` in
`crates/adtb/src/storage/pager.rs`). It gains one entry per page ever
evicted and never drops one. Expected: memory bounded by the pool or a
fixed size. Candidates are a fixed-size table, or a short LSN kept
beside the state word. The lost-write check of `docs/design/storage.md`
§3 should still hold for every page the bound keeps.

## 2026-10-25: resolved

Landed as `feat(storage): allocate and free inside an atomic change, catch self-deadlocks and bound evicted LSNs`.
