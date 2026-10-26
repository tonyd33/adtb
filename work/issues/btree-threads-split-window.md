---
created-at: 2026-10-25
status: open
kind: feature
priority: 590
summary: "the threaded B-tree crash test reaches an incomplete split by chance, and on main it reaches none"
tags: [quality, test, sim]
---

# Feature: seek the split window in the threaded B-tree crash test

## Report

`crates/adtb-sim/tests/btree_threads.rs` `threads_crashed_together_recover_a_prefix_of_each_threads_operations` asserts that some crash left a split incomplete. It reached 2 at switch rate 0.3 when written and 0 on main, since the simulator's random stream moved. Seek the crash point as the single-threaded B-tree test does, so the window is reached by construction.
