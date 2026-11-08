---
created-at: 2026-11-08
status: open
kind: bug
priority: 900
summary: "a numbered Plan::at Lose on a non-write is accepted when planned and panics on the submitting thread, a case runtime.md does not state"
tags:
  - quality
  - sim
  - design
---

# A numbered Lose on a non-write panics at submission, unstated in runtime.md

## Report

`Plan::at(n, Action::Lose)` (crates/adtb-sim/src/plan.rs:342) is accepted for any op number. When op `n` is not a write, the assert at crates/adtb-sim/src/sim.rs:1025 panics on the engine's submitting thread at submission. docs/design/runtime.md §5 states this case for a numbered `Fail` that comes to a `Close`, but for `Lose` only that a rule or first match that may take a non-write panics when planned.

Fix direction: state the numbered `Lose` case in runtime.md §5 beside the numbered `Fail`, or check it where the numbered action is taken (plan.rs:668), as the `Close` case is.
