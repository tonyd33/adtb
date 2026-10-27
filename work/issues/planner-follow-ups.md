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

## 2026-10-26: user aggregates

A user aggregate folds sequentially: its `merge` part is unused, so no parallel fold, and it skips the fused group-aggregate rewrite in `crates/adtb/src/plan/lower.rs` (about line 1769). `exec/ir.rs` `agg()` keeps an unreachable `Agg::User` arm. `docs/design/planner.md` does not exist, though the planner is a boundary other subsystems call.

## 2026-10-26: per-constructor fork by cost

A join whose constructor set no index covers now forks into one indexed join per constructor, but only when no covering index probe exists. An unbounded plan could weigh that fork against other access paths by cost.
