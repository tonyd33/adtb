---
created-at: 2026-10-29
status: resolved
kind: bug
priority: 999
summary: "an idle expiry or cancel after a task's output is drained still changes its settled outcome: a committed write turns incomplete and a drained read fails"
closed: 2026-10-29
tags:
  - quality
  - engine
---

# Task outcome changes after drain

## Report

Found by quality review 29 (range 0c489c2c..da5a38e0), checking docs/design/server.md section 1.3 ("Once an outcome is settled, status observations return the same outcome") against `crates/adtb/src/api/server/`.

Committed write. `Shared::request` (`crates/adtb/src/api/server/task.rs:161-186`) calls `State::abandon` (`task.rs:111-117`) for any `Idle` or `Drop` request once `s.committed`, without looking at `produced` or `drained`. `abandon` rewrites the settled `Completion::Committed { incomplete: false, .. }` to `incomplete: true`. So: a write task commits, every batch is observed, the transport sends `drained()` (`task.rs:270-275`) and may already have observed `Done(Committed { incomplete: false })` (`task.rs:251`); an `expire_idle` racing that acknowledgement (or arriving after it) makes the next `observe` return `Done(Committed { incomplete: true })`. The outcome changes after it was settled, and reports output abandoned that was fully delivered.

Read. After `drained()` and before the call's `drain` step commits the read and settles `Completion::Read` (`crates/adtb/src/api/server/call.rs:604-627`), `request` finds the task neither settled nor committed, records the request and stops the call (`task.rs:173-185`); `Call::fail` (`call.rs:638-649`) then settles `Failed(StreamIdle)` or `Failed(Cancelled)` for a read whose whole result was sent.

Repro (crates/adtb-sim/tests/tasks.rs style): submit `bump` (ordinal 3), drive to `Done(Committed { incomplete: false, .. })`, call `task.expire_idle(Duration::from_secs(1))`, observe again: `Done(Committed { incomplete: true, .. })`.

Fix: treat a request after the drain acknowledgement as a no-op (in `request`, return early once `s.drained`; `abandon` only while output remains), and add the repro and its read variant to tasks.rs.

## 2026-10-29: resolved

Landed in bb507838..b085c460 (task-fixes) after a clear fresh review (work/briefs/review-task-fixes.md, local) and a lead follow-up guarding on `drained && produced`. Branch check on 30bc4d3b passed (root 2782 passed, 7 skipped; campaign 47); main check skipped, the landed parent differing from the base only in tickets.

## 2026-10-29: resolved
