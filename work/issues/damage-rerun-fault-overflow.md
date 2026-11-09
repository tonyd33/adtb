---
created-at: 2026-11-09
status: open
kind: bug
priority: 420
summary: "the damage search's rerun of a KNOWN pin that lands on a create write-out panics in Plan::fault instead of reporting that the pin writes another cell"
tags:
  - testing
  - class-failure-flattened
---

# Bug: Damage search rerun panics on a pin that lands on a create write-out

## Observed

`Plan::fault` (crates/adtb-sim/tests/storage/damage_search/case.rs, the `Injection::Lost` arm) computes `w.during + 1` and slices `self.ops[after..]` for the write-out at `at`. For a create write-out this overflows. `rerun` evaluates the fault before it checks the write-out's kind, so a KNOWN pin that a change shifts onto a create write-out panics with an overflow rather than failing with "writes another cell".

## Expected

`rerun` checks the write-out's kind and owner before computing its fault, and `Plan::fault` is total over every write-out the trace holds, so a moved pin reports which cell it now writes.

## Reproduction

Pin a KNOWN entry to the `at` of a create write-out in any seed and run the damage profile. Found while re-pinning KNOWN in structure-version-owner step 10.
