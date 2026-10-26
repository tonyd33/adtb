---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 740
summary: the engine neither admits constraints by the index-bounded rule nor checks them incrementally
closed: 2026-10-25
depends-on:
  - planner
  - write-path
tags:
  - engine
---

# Feature: constraint admission and incremental checks

## Report

Implement `docs/spec/constraints.md` in the engine and
`docs/design/maintenance.md` §1, §3 and §6: the admission rule, delta
queries over base indexes checked at each statement boundary, and the
large change set path.

Prerequisites: `planner.md`, `write-path.md`.

## 2026-10-25: resolved

## 2026-10-25: resolved

`crates/adtb/src/write/delta.rs` compiles each constraint into one delta query per constructor-set read, planned by `plan::plan_bounded` (index probes and lookups only, else `Error::Unbounded` or `Outside`). `migrate` and `Database::new` refuse an unadmitted constraint with `Refused`, naming the missing index. `write/check.rs` runs the deltas per statement, skipping one whose trigger input is empty. The track also fixed SSI: a read that sees a version an uncommitted writer deleted, or misses one it inserted, now records the rw-antidependency (`Reads::depends` in `exec/store.rs`; `docs/design/mvcc.md` 8.2). `crates/adtb-sim/tests/constraints.rs` has 23 tests; `dropping_an_index_an_admitted_constraint_needs_is_refused_naming_the_constraint` fails until the engine applies later migrations (`operator-log`). Follow-ups: `maintained-aggregates`, `interval-probes`, `transition-joins`, `write-path-scans`, `reference-index-rule`, `transition-negation-rule`, `front-end-deep-literal`, `delta-internal-errors`.
