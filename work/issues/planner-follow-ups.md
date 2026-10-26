---
created-at: 2026-10-25
status: open
kind: feature
priority: 716
summary: "the planner has no range probes, no early stop for exists and top, and no flattened nested results"
depends-on:
  - planner
tags:
  - engine
---

# Feature: Planner follow-ups

## Report

From the first planner:
- index probes take equality only, with no range probes on ordered
  indexes;
- no sink stops early, because an `exists` or `top` that stops could
  drop an abort. Carry totality facts in the plan, so a total pipeline
  may stop early;
- nested results run as per-row nested queries, not flattened into a
  fixed number of flat queries;
- joins are never hoisted above the union;
- expression indexes are left out of the catalog until the code
  identity TODO in `execution.md` §5 is settled.

Constraints, transition reads and writes are planned by
`write-path.md` and `constraint-maintenance.md`.
