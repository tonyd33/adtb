---
created-at: 2026-10-25
status: open
kind: feature
priority: 731
summary: "the write path lacks rejects into, statement rollback and persistent sequences"
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
