---
created-at: 2026-10-25
status: open
kind: bug
priority: 559
summary: an allocation spanning two bitmap groups latches two bitmap pages, outside the latch budget
---

# Bug: allocation across bitmap groups exceeds the latch budget

## Report

The latch bound of five pages per thread budgets one bitmap page per
allocation. An allocation whose pages span two bitmap groups latches
two, so a write that allocates at its peak can take a sixth latch and
panic under `latch-checks`. Write a test that allocates across a group
boundary inside a heap write, then fit the allocation to the budget.
