---
created-at: 2026-10-26
status: open
kind: bug
priority: 595
summary: the reference interpreter still panics on mistyped values in boundary.rs and stmt.rs
---

# Reference boundary panics

## Report

## Report

`crates/adtb-reference/src/boundary.rs` (about lines 64, 214, 259, 270, 279, 424) and `stmt.rs:606` (`tag().expect`, sum and record shape panics) still panic when a checker bug hands them a mistyped value. Return `Abort::Internal` there.
