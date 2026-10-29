---
created-at: 2026-10-29
status: open
kind: bug
priority: 1005
summary: no task test streams a set whose elements repeat across units or that runs the deferred-join drain unit
tags:
  - quality
  - test
---

# Streamed set dedup untested

## Report

Found by quality review 29 (range 0c489c2c..da5a38e0).

The streamed path dedups elements across production units in `Call::take` (`crates/adtb/src/api/server/call.rs:587-595`, the `Stream.sent` set) and ends with a `Unit::Drain` that runs a partitioned join's deferred rows (`crates/adtb/src/exec/stream.rs:129-134`). Every streamed task test in `crates/adtb-sim/tests/tasks.rs` reads `all` (`tasks.rs:38`, `from i in items select seen(i.n)`) or `from i in items ... select i.n`: one scan whose elements are already distinct per row and no join. `a_task_returns_what_the_call_returns` (`tasks.rs:343`) asserts each element once, which those queries satisfy without `sent`. So deleting the `sent` check, or dropping the drain unit's rows, fails no test.

Fix: add task tests that stream (a) a select repeating values across page ranges, e.g. `from i in items select i.n % 7` over enough rows for several `range_pages` units, asserting the exact set and that `held` counts each element once, and (b) a join the planner partitions, asserting the result equals `Registered::call`'s, under a small capacity so the drain unit runs after batches were handed out.
