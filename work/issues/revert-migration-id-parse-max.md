---
created-at: 2026-11-13
status: open
kind: bug
priority: 220
summary: "revert reads an unparseable migration number as u64::MAX, a value standing in for an error"
tags:
  - class-failure-flattened
---

# Revert reads an unparseable migration number as `u64::MAX`

## Report

crates/adtb-lang/src/evolve/revert/mod.rs:377 and revert/inverse.rs:413 read a migration's number with `migration.parse().unwrap_or(u64::MAX)`, so a name that does not parse orders after every migration instead of being refused or typed. Found in the review of structure-lang-identity step 6b; not reproduced.

Direction: carry the migration's number as a typed value from the log, or refuse an unparseable one where it is read. The `id_sentinels` structure-check row could grow to count `unwrap_or(u64::MAX)`.

Defect class: Damage or failure gives a panic, hang or flattened error (an unparseable value flattened to a sentinel).
