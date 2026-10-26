---
created-at: 2026-10-25
status: open
kind: feature
priority: 700
summary: "milestone 7 (engine, single version) has not exited: no catalog, planner, executor, write path, constraints or in-process API"
depends-on:
  - write-path-follow-ups
  - exec-refusal-test-subject
  - catalog-follow-ups
  - exec-primitives
  - exec-follow-ups
  - planner-follow-ups
  - batch-value-types
  - textual-syntax-in-engine
  - milestone-2-reference
  - milestone-6-transactions
  - id-assignment-gaps
  - schema-catalog-ids
  - schema-ctor-stages
  - bootstrap-catalog
  - schema-first-migration
  - batches
  - planner
  - executor
  - write-path
  - constraint-maintenance
  - in-process-api
  - engine-differential-tests
tags:
  - milestone
---

# Feature: Milestone 7, engine, single version

## Report

Milestone 7 of `docs/plan.md`, where the tracks meet. Exit: the whole
single-version suite passes on the engine; the differential,
constructor-partition, predicate-partition and plan-toggle tests pass
over a fixed number of seeds per run. Needs milestones 2 and 6. The
first application (`prealpha-app.md`) starts when this exits.

Open on 2026-10-25: `id-assignment-gaps.md`, `schema-catalog-ids.md`,
`schema-ctor-stages.md`, `bootstrap-catalog.md`,
`schema-first-migration.md`, `batches.md`, `planner.md`,
`executor.md`, `write-path.md`, `constraint-maintenance.md`,
`in-process-api.md`, `engine-differential-tests.md`.
