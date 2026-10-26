---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 731
summary: "the write path lacks rejects into, statement rollback and persistent sequences"
closed: 2026-10-25
depends-on:
  - write-path
tags:
  - engine
---

# Feature: Write path follow-ups

## Report

My rulings:
1. `rejects into` and `reject` are implemented as `docs/spec/writes.md`
   states.
2. A failed statement rolls back its own changes and the transaction
   continues, as `docs/spec/transactions.md` states for interactive
   transactions.
3. A drawn sequence value is never reissued, even after a crash.
   Sequence blocks are reserved durably, as xids are, and
   `docs/design/mvcc.md` §12.10 says so.
4. Plans are cached across statements, keyed by the normalised term
   and the schema version.

## 2026-10-25: resolved

Items 1 to 3 landed: `reject` and `rejects into` with per-record
retry, statement rollback through an undo log and `Heap::restore`,
and durable sequence reservations stated in `docs/design/mvcc.md`
§12.10. Item 4 moved to [`plan-cache.md`](plan-cache.md).
Follow-ups: [`rolled-back-key-conflicts.md`](rolled-back-key-conflicts.md),
[`sequence-page-in-catalog.md`](sequence-page-in-catalog.md),
[`try-reject-totality.md`](try-reject-totality.md).
