---
created-at: 2026-10-25
status: open
kind: bug
priority: 723
summary: "the executor's compile-time refusal test has no unsupported primitive left to refuse"
tags:
  - test
  - engine
---

# Bug: A refusal test without a subject

## Report

`a_plan_with_a_primitive_the_executor_lacks_is_refused_at_compile_time`
in `crates/adtb-sim/tests/exec.rs` fails because the executor now runs
every prelude primitive. Point it at what the executor still refuses: a
call of a Wasm or host function, which the planner refuses with
`Error::UserCode`. Rename it to match.
