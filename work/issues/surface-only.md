---
created-at: 2026-10-25
status: wontfix
kind: feature
priority: 186
summary: "the surface has no way to take the one row out of a set, though the kernel has Kind::Only"
closed: 2026-10-25
tags:
  - spec
---

# Feature: surface only

## Report

No surface form takes the one row out of a set. The kernel has
`Kind::Only` (`crates/adtb-lang/src/kernel.rs`), which the upsert
`let` binding uses. Whether the surface needs it is undecided, and
goes through the design space.

## 2026-10-25: wontfix

Ruled: no surface `only`. Taking the one row of a set is a `match` over its key, and the kernel keeps `Kind::Only` for upsert bindings.
