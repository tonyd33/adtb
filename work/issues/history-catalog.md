---
created-at: 2026-10-27
status: resolved
kind: feature
priority: 871
summary: "history heaps, history key indexes and the retention record are never created, persisted or used by Database::vacuum"
closed: 2026-10-27
depends-on:
  - spec-retention-window-setting
tags:
  - engine
---

# Feature: history structures in the catalog

## Report

The history layer (`crates/adtb/src/mvcc/past.rs`, `crates/adtb/src/history.rs`, `vacuum_keeping` and `expire` in `crates/adtb/src/vacuum/run.rs`) is built and tested in `crates/adtb-sim/tests/history.rs`, but nothing outside the tests turns it on. Create and persist a history heap per constructor, a history key index per sum, their large value store and the retention record's tree root; list them in `inventory`; call `keep_history` at open when the retention window is not zero; and run `vacuum_keeping` and `expire` from `Database::vacuum`. Then lower `Kind::At` in the planner and executor, reading the current then the history heaps at the pinned marker and refusing with `HistoryUnavailable { oldest }` at run time.

## 2026-10-27: done

The catalog keeps a history heap per constructor relation and a history key index per relation (keyed and constructed as its key index), over a large-value store of their own, and the record of the window: created at create and by every migration that rewrites a relation, named by roots rows `HISTORY_HEAP` and `HISTORY_INDEX` and two root page fields, opened at open (`Missing` when absent), listed in `inventory` (`crates/adtb/src/catalog/history.rs`). `Catalog::retained` binds them for the write path with the version each migration made and the version whose migration made each heap. Open calls `keep_history` when `RetentionWindow` is not zero; `set setting RetentionWindow` turns history on or changes the window (`Transactions::set_window`). `vacuum` runs `vacuum_keeping` and then `expire` when history is kept. The planner lowers `Kind::At` to `Eval::At` (body planned with scans and key lookups only, apart from its holding query); the executor pins the marker and evaluates the body over a store that reads current then history heaps, refusing with `HistoryUnavailable { oldest }`. Tests: `crates/adtb-sim/tests/past.rs`.

Open:

- A marker logged after the last durable commit is lost at a crash: recovery does not rebuild markers from the log, so a read at that position reads an earlier marker.
- A migration that rewrites a relation drops its history structures (`history-across-rewrites`); a read before it is refused.
- Versions deleted while the window was zero have no recorded commit time, so raising the window does not keep them.
- `sys::commit_times` and `sys::safe_snapshots` are not filled from the record.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the hcat track. What stays open is in `history-follow-ups`, `history-across-rewrites` and `heap-directory-interior-holes`.
