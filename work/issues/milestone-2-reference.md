---
created-at: 2026-10-25
status: open
kind: feature
priority: 200
summary: "milestone 2 (reference interpreter and conformance runner) has not exited: no runner and no first suite"
depends-on:
  - spec-catalog-model-gaps
  - spec-dump-damaged-error
  - spec-writer-wait-cycle
  - spec-text-gaps
  - reference-regex-collator-cache
  - spec-row-too-large
  - milestone-1-front-end
  - reference-interpreter-writes
  - conformance-runner
  - textual-value-syntax
  - reference-unicode-text
  - spec-textual-specials
  - spec-take-negative
  - spec-field-eval-order
  - spec-constraint-naming-gaps
tags:
  - milestone
---

# Feature: Milestone 2, reference interpreter and conformance runner

## Report

Milestone 2 of `docs/plan.md`. Exit: the first suite passes on the
interpreter; typing soundness holds over the spec corpus and over
type-directed generated terms. Needs milestone 1.

Open on 2026-10-25: `reference-interpreter-writes.md` (awaiting
landing), `conformance-runner.md`, `textual-value-syntax.md`,
`reference-unicode-text.md`, and the spec rulings the interpreter
needs: `spec-textual-specials.md`, `spec-take-negative.md`,
`spec-field-eval-order.md`, `spec-constraint-naming-gaps.md`.
