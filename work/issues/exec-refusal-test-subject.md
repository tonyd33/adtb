---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 723
summary: "the executor's compile-time refusal test has no unsupported primitive left to refuse"
closed: 2026-10-25
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

## 2026-10-25: resolved

Landed as `fix(reference): print spans as span(a, b) and keep dates at the lower edge of Instant`. It also fixed an overflow at the lower bound of `Instant`, in both the executor and the reference.
