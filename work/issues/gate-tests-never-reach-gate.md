---
created-at: 2026-10-30
status: in-progress
branch: quality-fixes33
kind: bug
priority: 897
summary: "the gated writer tests sleep 30 ms and never observe the writer at the gate, so they pass if it was refused before reaching it, and bound elapsed wall time"
tags:
  - quality
  - test
---

# Gate tests never reach gate

## Report

`a_gated_write_stops_at_its_deadline`, `a_gated_write_stops_when_cancelled` and `close_with_a_gated_writer_does_not_wait_out_the_rebuild` at `crates/adtb-sim/tests/reindex_review.rs:657`, `:680` and `:704` sleep 30 ms and assume the writer has reached `wait_for_rebuilds`. Each asserts only `is_err()`, with `the write ran through the gate` as its message.

A writer that had not reached the gate, or that failed for another reason, passes: a cancel before the gate is refused the same way. The error kind is never checked. Their bounds on `Instant::elapsed` (under 1 s) also depend on machine load.

Fix direction: assert the error is the deadline, `Cancelled` or closed one, and observe the writer at the gate, for example by a counter of waits or a deterministic sim run.
