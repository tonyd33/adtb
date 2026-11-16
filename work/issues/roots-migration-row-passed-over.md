---
created-at: 2026-11-15
status: resolved
closed: 2026-11-16
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

## Resolved

Landed as qsupwyqs f6a76796 (structure-lang-identity step 11 part e, 2 rounds, full check 0, 3843 tests). Every roots row is written by `RootsRow::encode` and read by `decode` (crates/adtb/src/catalog/roots.rs), one `SHAPES` table building the heap row type, a const assertion that each id is its position, and readers matching every variant with no catch-all; `retained`, `kept`, `deployed` and `replace_kept` refuse a damaged row of any constructor with the `Rot` open gives (51 of 65 row and reader pairs passed a damaged row over before). A kept row's blob stays undecoded until `Catalog::kept`, so open and salvage behave as before. A literal fixture pins every constructor's stored form.
