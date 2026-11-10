---
created-at: 2026-11-10
status: resolved
kind: bug
priority: 230
summary: "the damage search's two-fault test aborts at the 1.5 GiB test heap cap under the deep profile, so the deep tier's pairs are never judged"
closed: 2026-11-10
tags:
  - testing
  - class-weak-tests
---

# The deep pairs shard of the damage search aborts at the heap cap

## Report

## 2026-11-10

`two_faults_lose_no_acknowledged_row_and_hide_no_stale_page` (crates/adtb-sim/tests/storage/damage_search/mod.rs) under `--profile deep` is killed by SIGABRT at the test heap cap after about 86 s. It does the same at d5999805, before structure-damage-policy step 9, so no change of that step caused it. The damage profile's pairs run passes; only the deep tier's pairs go unjudged.

Direction: find what holds the memory (every case's trace kept to the end, or the cases built up front); bound it so the shard fits the default cap. If it cannot, raise the cap for this test with `adtb_heap_cap::raise_cap` and `threads-required` in `.config/nextest.toml`, stating the bound it comes from.

Defect class: Tests asserting less than they claim (a gate that does not run).

## 2026-11-10: fixed

A planned `Action::Run` holds a clone of the `SimRuntime`; when no write-out took it, the cycle leaked the whole simulation, about 31 MB a case. `Drop for Sim` now takes the plan's rules and actions out (`Planner::forget`) and drops them outside the plan lock, for every sim test. The deep pairs tests pass under the default cap with no `raise_cap` (landed with damage-search-fault-pairs, a741ceaa).

## 2026-11-10: resolved
