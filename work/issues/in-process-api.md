---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 750
summary: "no in-process API: Database::open, ad hoc text, transact, interactive transactions and retry do not exist"
closed: 2026-10-25
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

## 2026-10-25: resolved

## 2026-10-25: resolved

`crates/adtb/src/api/` gives `Database::open` (over `OsRuntime`) and `open_in` (any runtime), ad hoc text through `query_text`, interactive transactions (`AdHocTransaction`), and the retrying `transact`. Client errors mirror `docs/spec/catalog.md` section 4; `write::Error::Internal` maps to `Rejected { reason: Internal { what } }`. `write::Database::statements` makes a call of several items atomic, and `commit` returns its position. Tests: `crates/adtb-sim/tests/api.rs` (31) and `api_real_file.rs` (3). Failing on purpose: `a_deferrable_read_only_transaction_reads_and_commits` (`api-follow-ups`) and `a_second_migration_applies_at_open` (`operator-log`). Follow-ups: `api-follow-ups`.
