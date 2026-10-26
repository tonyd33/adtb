---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 590
summary: "the threaded B-tree crash test reaches an incomplete split by chance, and on main it reaches none"
closed: 2026-10-25
tags:
  - quality
  - test
  - sim
---

# Feature: seek the split window in the threaded B-tree crash test

## Report

`crates/adtb-sim/tests/btree_threads.rs` `threads_crashed_together_recover_a_prefix_of_each_threads_operations` asserts that some crash left a split incomplete. It reached 2 at switch rate 0.3 when written and 0 on main, since the simulator's random stream moved. Seek the crash point as the single-threaded B-tree test does, so the window is reached by construction.

## 2026-10-25: resolved

## 2026-10-25: resolved

Two cut rounds per seed record the run and crash at the first of up to 32 log forces inside an insert whose image leaves a split for `repair`. 4 rounds over 4 seeds reach one, all in the Whole shape; the bound is `share(SEEDS, 2)`. The timed-crash bound measured 9 to 11 and is now `share(SEEDS, 5)`. The Grouped shape stops splitting once its tree settles, so posting-leaf splits are not reached in this test.
