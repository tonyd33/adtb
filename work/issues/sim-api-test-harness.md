---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 797
summary: "api.rs, derived_index.rs and oplog.rs each define run, once, query and printed, and run's Outcome assertion inside seed loops omits the seed"
closed: 2026-10-26
tags:
  - quality
---

# Feature: one API test harness for the simulator tests

## Report

`crates/adtb-sim/tests/api.rs:60-100`, `crates/adtb-sim/tests/derived_index.rs:52-90` (new in this range) and `crates/adtb-sim/tests/oplog.rs:40-75` each define `run`, `once` or `opened`, `query`, `printed`, `in_txn` and `options` with the same bodies, differing only in whether rows are sorted. Each `run` asserts `assert_eq!(Outcome::Finished, outcome)` with no message, and the seed loops in api.rs (:140, :427, :615, :819, :850, :883, :962) and derived_index.rs (:127, :294, :316, :348, :392) call it through `once(seed, ..)`, so a deadlocked or hung run fails without naming its seed.

Fix: move the helpers to a module under `crates/adtb-sim/tests/support`, with `once(seed, body)` putting the seed in the outcome assertion, and have the three files use it.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed in wqmnkqkx, within the adtb crate and its tests. What remains in adtb-lang is `lang-id-follow-ups`.
