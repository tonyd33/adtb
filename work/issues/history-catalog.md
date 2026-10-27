---
created-at: 2026-10-27
status: open
kind: feature
priority: 871
summary: "history heaps, history key indexes and the retention record are never created, persisted or used by Database::vacuum"
depends-on:
  - spec-retention-window-setting
tags:
  - engine
---

# Feature: history structures in the catalog

## Report

The history layer (`crates/adtb/src/mvcc/past.rs`, `crates/adtb/src/history.rs`, `vacuum_keeping` and `expire` in `crates/adtb/src/vacuum/run.rs`) is built and tested in `crates/adtb-sim/tests/history.rs`, but nothing outside the tests turns it on. Create and persist a history heap per constructor, a history key index per sum, their large value store and the retention record's tree root; list them in `inventory`; call `keep_history` at open when the retention window is not zero; and run `vacuum_keeping` and `expire` from `Database::vacuum`. Then lower `Kind::At` in the planner and executor, reading the current then the history heaps at the pinned marker and refusing with `HistoryUnavailable { oldest }` at run time.
