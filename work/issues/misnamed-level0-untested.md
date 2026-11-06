---
created-at: 2026-11-06
status: open
kind: bug
priority: 470
summary: "Space::misnamed && to || survives: no test repoints an entry while its leaf stays level 0"
tags:
  - class-weak-tests
---

# Test Space::misnamed on a level-0 leaf

## Report

## 2026-11-06

cargo-mutants survivor at crates/adtb/src/heap/dir.rs:732:47 (`&&` to `||`): a leaf entry repointed while the leaf stays level 0 answers Held instead of Changed. Missing test in dir_tests.rs: the interleaving of a_refusal_rechecks_an_entry_whose_leaf_changed_under_its_check with the leaf still level 0 and its entry repointed.
