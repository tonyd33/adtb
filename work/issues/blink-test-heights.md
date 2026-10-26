---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 543
summary: "suffix truncation made test trees shorter, and three B-link tests lowered their height minimums instead of growing their workloads"
closed: 2026-10-25
tags:
  - test
  - engine
---

# Bug: B-link tests reach shallower trees

## Report

After suffix truncation, `ascending_and_descending_inserts_split_cleanly`
(2 to 1), `random_operations_match_a_btreemap` (3 to 2) and
`random_operations_match_a_btreemap_on_other_seeds` (2 to 1) lowered
their minimum tree heights. Restore the old minimums by growing the
workloads or the entry sizes, so the tests still reach multi-level
trees.

## 2026-10-25: resolved

Landed as `test(btree): restore the B-link tests' tree heights with long shared prefixes`.
