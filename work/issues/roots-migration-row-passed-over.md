---
created-at: 2026-11-15
status: open
kind: bug
priority: 190
summary: "history's retained passes over a malformed MIGRATION or HEAP_ROOT roots row instead of refusing it"
tags:
  - engine
  - class-damage-unreported
---

# A malformed MIGRATION or HEAP_ROOT roots row is passed over

## Report

Found by structure-lang-identity step 10a. `retained` in crates/adtb/src/catalog/history.rs (around line 385) matches roots rows by `(ctor, fields)` and passes over a `MIGRATION` or `HEAP_ROOT` row of another arity with `_ => {}`, where `named` in crates/adtb/src/catalog/mod.rs refuses the same row. A damaged row is then silently left out of what history retains.

Fix direction: refuse a malformed row of those constructors as `Rot::RootsCtor`, with a test per constructor asserting the whole refusal.
