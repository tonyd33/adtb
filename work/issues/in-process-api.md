---
created-at: 2026-10-25
status: open
kind: feature
priority: 750
summary: "no in-process API: Database::open, ad hoc text, transact, interactive transactions and retry do not exist"
depends-on:
  - write-path
  - constraint-maintenance
  - schema-first-migration
tags:
  - engine
---

# Feature: in-process API

## Report

Implement `docs/spec/embedding.md` §1, §2 (no migrations), §5 and §6
and `docs/spec/transactions.md` §3, §5 and §6: `Database::open`, ad hoc
text, `transact`, interactive transactions, retry on serialization
failure.

Prerequisites: `write-path.md`, `constraint-maintenance.md`,
`schema-first-migration.md`.

## 2026-10-25: internal errors

Map `write::Error::Internal` to `Rejected { reason: Internal { what } }` at the session boundary.
