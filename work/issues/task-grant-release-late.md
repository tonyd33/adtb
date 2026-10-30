---
created-at: 2026-10-29
status: open
kind: bug
priority: 1025
summary: "a finished task releases its memory grant only after Done is observable, and an unreleased batch keeps the whole grant admitted"
tags:
  - engine
  - quality
---

# Bug: a task's grant outlives its finish

## Report

From the review of task-fixes (work/briefs/review-task-fixes.md, local, finding 5). Predates that track.

- docs/design/server.md 1.2 ("Finished") and execution.md 7.5 say a task's grant is released when it finishes. The code releases it only when the scheduler drops the call (crates/adtb/src/exec/sched.rs:555), and `Call::run` wakes the observer before that (crates/adtb/src/api/server/call.rs:709). Witness: with `query_memory_bytes` at 1 MiB, a `Registered::call` made right after observing `Done(Read)` is refused with `StatementLimit` Memory on sim seeds 9 and 10; with a 1 ms sleep first every seed passes.
- A batch handed out and not released keeps the whole grant admitted, not only its own bytes.
- `a_finished_task_holds_no_memory_grant` runs only seed 0.

## Fix

Release the grant before settling the outcome the observer sees (or settle after the drop), so a call after `Done` sees the budget free; decide whether an unreleased batch should hold only its bytes and state it in server.md. Run the grant test over the sim seeds.
