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

## 2026-10-26: done

Range probes, early stop and flattening are in, each behind a rewrite (`range_probe`, `early_stop`, `flatten`) that the plan-toggle variants and the differential `settings` cover.

- Range probes: `End::Range` with `RangeBound`s in `crates/adtb/src/plan/mod.rs`; the planner takes the first lower and upper comparison on the index field after the equality prefix of an ordered index, keeping every comparison as a route filter. The executor reads it through `Store::range` (`crates/adtb/src/exec/store.rs`) over the new byte-bounded `btree::Index::scan`, swapping bounds for a descending field.
- Early stop: `Query.total`, set from the totality of the source and the sink's per-element terms. The executor stops an `Exists` or `IsEmpty` query once a value is emitted and skips the pipelines of a `Top` whose count is zero or less. Top-n does not stop early over an ordered source yet.
- Flattening: a correlated nested query inside a chain becomes a `Group` query bound by a `Spine::Let` of the enclosing query and an `Eval::GroupRows` lookup per row, when the fixed cost model prefers it.

`docs/design/execution.md` §1 and §7 state the new nodes and the changed evaluation rule. Hoisting joins above the union and expression indexes stay open.

## 2026-10-26: landed, and what stays open

Range probes, early stop and flattening landed with the plan track. Still open, besides hoisting joins above the union, expression indexes, user aggregates and the per-constructor fork above:
- A top-n over an ordered index scan in key order does not stop early.
- No test reaches a descending index or a float index holding NaN through the range path; the fixtures have neither.
- `store.scan` reads every batch of a heap before morsels run, so an early stop saves evaluation but not I/O.
