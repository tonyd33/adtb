---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 186
summary: "the surface has no way to take the one row out of a set, though the kernel has Kind::Only"
branch: default
tags:
  - spec
---

# Feature: surface only

## Report

No surface form takes the one row out of a set. The kernel has
`Kind::Only` (`crates/adtb-lang/src/kernel.rs`), which the upsert
`let` binding uses. Whether the surface needs it is undecided, and
goes through the design space.
