---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 230
summary: "no runner for the script format of docs/spec/conformance.md, and no conformance suite"
branch: adtb-conf
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
