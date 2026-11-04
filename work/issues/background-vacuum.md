---
created-at: 2026-10-29
status: in-progress
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
