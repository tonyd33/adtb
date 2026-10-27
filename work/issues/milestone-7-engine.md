---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 700
summary: "milestone 7 (engine, single version) has not exited: no catalog, planner, executor, write path, constraints or in-process API"
closed: 2026-10-27
depends-on:
  - admission-path-forms
  - engine-file-scans
  - write-path-follow-ups
  - exec-refusal-test-subject
  - catalog-follow-ups
  - exec-primitives
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
  - campaign-release-1-2-refused-by-engine
  - file-pin-dropped-on-failed-statement
  - vacuum-temp-relation-panics
  - explain-analyze-change-incomplete
  - csv-trailing-blank-line-row
  - file-scan-bom
  - differential-refused-agrees
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

## 2026-10-26: gate

heap-performance and batch-value-types are follow-up work, not exit blockers. The exit also takes the wrong-answer and panic bugs the 2026-10-26 quality review found in this milestone's code, and the campaign app's release 1.2 log opening in the engine.

## 2026-10-27: resolved

## 2026-10-27: exited

exec-follow-ups and planner-follow-ups hold performance and coverage work (Arrow kernels, grant estimates, work stealing, top-n over an ordered scan, range path tests), not exit blockers; they stay open outside the gate. Every other dependency is resolved, the quality review's wrong-answer and panic bugs in this milestone's code are fixed, and the campaign app's releases open in the engine.
