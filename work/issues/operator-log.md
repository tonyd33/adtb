---
created-at: 2026-10-25
status: open
kind: feature
priority: 801
summary: "no operator log: schema operators, version hashes, data maps, inverses and revert do not exist in the engine"
depends-on:
  - check-migration-operators
  - schema-first-migration
tags:
  - engine
---

# Feature: operator log and schema operators

## Report

Implement `docs/spec/evolution.md` §1 to §5 and §10 and
`docs/design/schema-change.md` §3: the operator log, version hashes,
every schema operator with its data map, inverse and declared loss,
and revert.

Prerequisites: `check-migration-operators.md`,
`schema-first-migration.md`.

## 2026-10-25: constraint operators

`crates/adtb-sim/tests/constraints.rs` `dropping_an_index_an_admitted_constraint_needs_is_refused_naming_the_constraint` fails until the engine applies migrations after the first; add constraint, drop index and drop constraint need it, with constraint validation and pending drops (`docs/spec/constraints.md` 9.2 and 9.3).
