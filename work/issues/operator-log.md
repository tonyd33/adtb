---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 801
summary: "no operator log: schema operators, version hashes, data maps, inverses and revert do not exist in the engine"
closed: 2026-10-26
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

## 2026-10-25: second migration at open

`crates/adtb-sim/tests/api.rs` `a_second_migration_applies_at_open` fails until the catalog applies migrations after the first.

## 2026-10-26: stage 1 landed

Landed as nsmmrnqx: migrations after the first apply at open and through the API, with version hashes and a generic per-operator data map. Fills, routings, nest and unnest, rekey carries, constraint checks on the final state, and revert of a drop field are left; seven failing tests in `crates/adtb-sim/tests/oplog.rs` are their spec. The drop-index constraint test now drops a non-reference index, since the implied reference index returns when a declared one is dropped.

## 2026-10-26: resolved

## 2026-10-26: landed

Stages 2 and 3 landed as qpmmzlpu: fills, routings, nest, unnest, rekey carries, validation of constraints on the final state, and revert through `Source::steps`. Remaining gaps are in operator-log-follow-ups, split-inverse-rule, migration-path-indexes and migration-forward-refs.
