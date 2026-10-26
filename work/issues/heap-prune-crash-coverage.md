---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 590
summary: the pruning crash test counts the runs that left entries to settle but never checks the count
closed: 2026-10-25
tags:
  - quality
  - sim
  - test
---

# The pruning crash test can pass without exercising an unsettled entry

## Report

`a_crash_while_pruning_raises_entries_leaves_what_repair_mends` (`crates/adtb-sim/tests/heap.rs:1076`) counts in `caught` (`:1078`, `:1138`) the runs whose recovered heap holds an entry promising less than its child, and only prints it (`:1143`). It also discards the outcome of `run_process` (`:1103`), so nothing checks that any of the 40 runs crashed. If the crash times drift past the pruning, every run finishes clean and the test still passes, never testing what its name claims.

Fix: count the crashed runs and assert both counts, as `a_crash_while_a_conversion_counts_its_values_leaves_no_count_too_low` does with `crashes > 4`; put the step in the message.

## 2026-10-25: resolved

## 2026-10-25: resolution

Fixed on the qtest track; the test asserts what it claims and its messages name the seed.
