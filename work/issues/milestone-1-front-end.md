---
created-at: 2026-10-25
status: open
kind: feature
priority: 100
summary: "milestone 1, the front end: the spec corpus does not yet pass and no fuzzer has run"
depends-on:
  - migration-forward-refs
  - anon-sum-join-fix-writes
  - norm-fresh-let-substitution
  - unique-value-level-path
  - set-record-parses-as-sum
  - front-end-decimal-literals
  - front-end-ctor-comparison-join
  - anon-sum-ctor-index
  - agg-source-nonempty
  - adhoc-skips-grants
  - reindex-clause-unchecked
  - engine-catalog-modules
  - check-migration-operators
  - file-scans
  - temp-relations
  - user-defined-aggregates
  - explain-terms-dropped
  - spec-adhoc-statement-gaps
  - spec-impl4-followups
  - spec-fix-and-totality-rulings
  - surface-only
  - front-end-fuzzer
tags:
  - milestone
---

# Feature: Milestone 1, front end

## Report

Milestone 1 of `docs/plan.md`. Exit: the spec corpus passes; the
fuzzer has run for a fixed budget with no crash.

Open on 2026-10-25:

- checker bugs: `norm-fresh-let-substitution.md`,
  `unique-value-level-path.md`, `set-record-parses-as-sum.md`,
  `front-end-decimal-literals.md`, `front-end-ctor-comparison-join.md`,
  `anon-sum-ctor-index.md`, `agg-source-nonempty.md`,
  `adhoc-skips-grants.md`, `reindex-clause-unchecked.md`;
- spec blocks the checker refuses: `engine-catalog-modules.md`,
  `check-migration-operators.md`, `file-scans.md`, `temp-relations.md`,
  `user-defined-aggregates.md`, `explain-terms-dropped.md`;
- spec fixes and rulings: `spec-adhoc-statement-gaps.md`,
  `spec-impl4-followups.md`, `spec-fix-and-totality-rulings.md`,
  `surface-only.md`;
- `front-end-fuzzer.md`.
