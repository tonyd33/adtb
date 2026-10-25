---
created-at: 2026-10-25
status: open
kind: feature
priority: 730
summary: "the engine cannot apply a change set: no inserts, deletes, retag, delete actions, upsert, key generation or defaults"
depends-on:
  - executor
  - unique-checks
  - ssi
  - commit-path
  - schema-first-migration
tags:
  - engine
---

# Feature: engine write path

## Report

Implement `docs/spec/writes.md` in the engine: change sets applied as
one multiple assignment, retag across heaps (`docs/design/mvcc.md`
§6), delete actions, upsert, key generation by `sequence` and
`uuidv7` (`mvcc.md` §12.10), defaults. The reference interpreter's
outcome is the oracle.

Prerequisites: `executor.md`, `unique-checks.md`, `ssi.md`,
`commit-path.md`, `schema-first-migration.md`.
