---
created-at: 2026-10-29
status: resolved
closed: 2026-11-04
kind: feature
priority: 725
summary: "vacuum runs only by command, VacuumBudget is unread and sys::vacuums has no rows"
---

# Feature: Background vacuum

## Request

Slice of milestone 9; scope, tests and acceptance in work/briefs/review-m9-audit.md, S9. Needs scrub-worker.

## Proposal

## Open questions

## 2026-11-04: V1, V2 and V4 landed

Plan in four slices: V1 `sys::vacuums` rows for a running vacuum, V2 pacing under the maintenance budget, V3 the trigger and background worker, V4 one pass over each index for all of a relation's constructor heaps. V1, V2 and V4 landed as `feat: show running vacuums in sys::vacuums`, `feat: run vacuum and the scrub under one VacuumBudget pacer` and `feat: vacuum a relation's constructor heaps with one pass over each index`.

`vacuum r` and the scrub share one pacer taking the lesser of the io and cpu budgets; steps come before each scanned heap page, each pruned page, each history page kept, after each index leaf and each freed chunk of a large value, and the expiry of the commit-time record is paced; a final charge covers any tail. A stopped walk is a distinct outcome, so no slot is freed after a stop, and a stop during the closing sweep returns its refusal. One I/O handle serves a whole vacuum. Each heap's `sys::vacuums` row reaches `done` when its closing prune ends; the relation's out-of-line sweep runs once after. Each constructor row's `freed` text names the relation's chunk count.

Review found four blockers in V2: a large value's chunk chain freed in one step (12.5 ms of 26 ms), the retention loop and the record expiry unpaced, and a longest-step test blind to work after the last pace. All fixed; each reverted fix fails its test. Two mutants survive and are equivalent today: the final charge (no work follows the last pace) and the stop recomputed after the hold (a command's wait carries no statement limit, see command-statement-limit). Full check 3690 passed.

Next: V3, the retention-expiry trigger, the background worker sharing `Control` and the pacer, and how a background failure shows.

## 2026-11-04: V3 landed

`feat: vacuum a relation in the background when its dead versions reach a threshold`. A per-heap count of dead versions on `write::Database`: commit adds the rows deleted or replaced, abort adds the rows inserted, and the versions a failed call took back count too. A relation is due when its count reaches `max(50, pages after its last finished vacuum)` and the least watermark of its heaps has passed the one its last run began under; every relation is due once after open; with retention on, a relation is due on the clock a quarter of the window after its last vacuum that left history. A vacuum daemon beside the scrub shares `Control` and the pacer, picks due relations round robin, takes the vacuum lock only without waiting and yields the run to any waiter; a run subtracts what it freed per heap; a failure other than a cancel skips the relation for the process. `OpenOptions::no_background_vacuum` (doc hidden, unspecified) leaves the worker unstarted for tests of the command and the scrub and for the script runner.

Review found the snapshot-safety search passing with the worker disabled; it now asserts a worker run inside a reader's snapshot, and both that mutant and the watermark mutant fail it. Also fixed in review: all-visible pages counted twice in the threshold, a run that freed nothing resetting the count, relations starved in id order, a wake on every commit during a run. Full check 3379 passed.

Filed from V3: background-vacuum-failure-surface, maintenance-thread-fatal-io, background-vacuum-yield-restarts, dead-count-outside-statements, page-past-log-end-accepted (the reason `verify::a_page_past_the_log_end_is_an_unlogged_finding_of_its_relation` reopens with the worker off), verify-misses-nondurable-damage.
