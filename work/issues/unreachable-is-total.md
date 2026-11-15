---
created-at: 2026-11-14
status: open
kind: bug
priority: 260
summary: "the totality analysis treats Unreachable as total, so a planner may hoist and reach it"
tags:
  - lang
  - class-none
---

# the totality analysis treats Unreachable as total, so a planner may hoist and reach it

## Report

Found by the translate track (ookywzux 9751992c). `Totality` (adtb-lang analysis/totality.rs) classifies `Unreachable` as total, so a consumer may evaluate it early: a closed `Unreachable` under a generator over an empty constructor set was hoisted out of the loop by the planner and reached, answering `Internal { what: "an evaluation reaches no `Unreachable`" }` in `every-read-of-removed-constructor-empty` and `aggregate-and-test-of-removed-constructor` (the translate track builds a `Case` with no arms there instead). kernel.md says reaching `Unreachable` breaks an invariant, so it is partial for hoisting, or a closed one never occurs; decide which and enforce it. Test: a closed `Unreachable` under a generator over an empty set is not hoisted (plan test).

Defect class: none (no other instance known).
