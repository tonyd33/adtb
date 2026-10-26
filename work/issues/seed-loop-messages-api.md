---
created-at: 2026-10-26
status: open
kind: feature
priority: 795
summary: seed loops in the API and real-runtime fault tests assert without the seed
tags:
  - quality
---

# Feature: every seed loop names its seed in its assertions

## Report

Assertions inside `for seed in` loops without the seed in their message:

- crates/adtb-sim/tests/api.rs:623 and 624 (`assert_eq!(vec!["0"], in_txn(&mut a, count))` and the same for `b`).
- crates/adtb-sim/tests/io_contract/faults.rs:299, 301, 304, 309 (the unforced names test) and 342, 345, 347 (the directory test).

Fix: add `"seed {seed}"` to each message.
