---
created-at: 2026-10-25
status: open
kind: bug
priority: 525
summary: "the B-link model test slows faster than linearly: 12k operations take 4s, 30k take 81s"
depends-on:
  - blink-tree
tags:
  - engine
---

# Bug: B-link test time grows faster than linearly

## Report

The simulator model test in `crates/adtb-sim/tests/btree.rs` takes
about 4s for 12,000 operations but 81s for 30,000. The cause is not
known; it may be in the tree, the pager's pool or the simulator.
Expected: a profile names the cause, it is fixed, and a test bounds
the time per operation.
