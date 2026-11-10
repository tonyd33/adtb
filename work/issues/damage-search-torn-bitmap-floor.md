---
created-at: 2026-11-10
status: open
kind: bug
priority: 240
summary: "the damage search's torn-write test under the deep profile fails its per-kind floor with 16 Bitmap cases of 32"
tags:
  - testing
  - class-weak-tests
---

# The deep torn shard of the damage search misses its Bitmap floor

## Report

## 2026-11-10

Under `--profile deep`, the damage search's torn-write test fails with "kinds with fewer than 32 cases" (crates/adtb-sim/tests/storage/damage_search/mod.rs), Bitmap at 16. It fails the same way with the same counts at 077761eb, before structure-damage-policy step 10, so no change of that step caused it. The deep tier's torn cases are judged, but the floor that says each kind was reached does not hold.

Direction: find why the torn fault reaches fewer Bitmap pages than the floor asks (the workload writes too few bitmap pages, or the picker spends its draws elsewhere); make the search reach the floor, or state from the workload the bound that sets a lower floor for Bitmap.

Defect class: Tests asserting less than they claim (a gate that does not run).
