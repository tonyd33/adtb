---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 230
summary: "no runner for the script format of docs/spec/conformance.md, and no conformance suite"
closed: 2026-10-25
tags:
  - reference
  - test
---

# Feature: conformance runner and first suite

## Report

Nothing runs the script format of `docs/spec/conformance.md`. Milestone
2 of `docs/plan.md` needs the runner, over the reference interpreter,
and the first suite: one script directory per spec part, starting with
model, queries, writes and constraints. The milestone exits when that
suite passes on the interpreter.

## 2026-10-25: resolved

`adtb-conformance` runs the script format (point `spec5`) over the
reference interpreter, with suites for model, queries, writes and
constraints and the generator templates under `conformance/gen/`. The
14 failing directives are
[`conformance-checker-findings.md`](conformance-checker-findings.md).
