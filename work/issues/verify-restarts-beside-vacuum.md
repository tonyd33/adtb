---
created-at: 2026-11-06
status: open
kind: bug
priority: 450
summary: a suspended verify beside the background vacuum takes 20x its time
tags:
  - class-unbounded-work
---

# Bug: verify restarts beside the background vacuum

## Report

Found by the sim-first-verify-task-deadlock fix (2026-11-06). A suspended `verify` task running beside the background vacuum took 6.3 s of simulated time against 0.3 s with no vacuum (300 deleted rows, VacuumBudget 0.01; 84 s at 0.001). It stops once the vacuum finishes. Not traced; the author's guess: the call restarts from the beginning after each yield, and the vacuum takes the lock back after `VACUUM_YIELD` before the restarted call reaches its next index hold, so verify's progress per yield shrinks as the budget falls.

Expected: a call that asks the vacuum to yield finishes in time bounded by its own work, not by the vacuum's remaining work.
