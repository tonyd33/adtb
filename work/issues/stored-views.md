---
created-at: 2026-10-26
status: open
kind: feature
priority: 870
summary: "the engine has no stored views, so a view with no index-bounded maintenance is not refused"
tags:
  - engine
---

# Feature: stored views in the engine

## Report

`store` has no resolved form, so the engine refuses it with "this operator has no resolved form yet". Admission belongs to the engine's bounded planner (`docs/design/maintenance.md` section 3): a stored view is admitted only when every relation it reads has an index-bounded maintenance plan, and the refusal names the missing index. The checker has no part of this to do.

Fix: give `store` a resolved form and a catalog row, maintain the view through the bounded planner, and refuse an unbounded one naming the index. The ignored test `storing_a_view_with_no_index_bounded_maintenance_is_refused_naming_the_index` in `crates/adtb-sim/tests/constraints.rs` states the refusal.

## 2026-10-27: resolved forms

Every operator now has a resolved form (resops track). `store` writes catalog rows and the view is computed on read; `script`, `install` and `adopt tzdb` are still refused, see `script-in-engine` and `extensions-and-tzdb-rows`.
