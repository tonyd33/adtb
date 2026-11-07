---
created-at: 2026-11-06
status: open
kind: bug
priority: 300
summary: a_crashed_tree_recovers_a_prefix_of_its_operations takes about 22 s alone and times out under full-suite load
tags:
  - class-none
---

# Bug: the btree crash-prefix test times out under load

## Report

`btree::a_crashed_tree_recovers_a_prefix_of_its_operations` takes about 22 s alone, both on main and on the damage-log change 2 tree (2026-11-06), and was terminated at 60 s in a full `check --changed` suite. A full check fails on it without any change to the btree.

Expected: the test fits its slow-timeout with margin under the full suite, by a smaller search, a bound on the model's work, or a declared slower group with the reason stated.
