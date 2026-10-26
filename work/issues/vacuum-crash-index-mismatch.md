---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 499
summary: "after crashes beside a concurrent vacuum, an index read returns other rows than the heap at seed 5"
closed: 2026-10-25
tags:
  - engine
  - sim
---

# Bug: index and heap disagree after a crash during vacuum

## Report

`crates/adtb-sim/tests/vacuum_threads.rs` `crashes_beside_concurrent_vacuum_recover_clean`, at switch rate 0.3, panics at line 425 for seed 5: "Snapshot { xmin: Xid(78), xmax: Xid(78), in_progress: [] } reads other rows by index". It passed on the test-support track's base (luprkwrr) and fails on main after the internal error, real runtime, textual syntax, migration log and constraint tracks landed; the real runtime track changed the simulator's disk (directories, crash pruning) and the constraint track changed SSI reads in `exec/store.rs`.

## 2026-10-25: resolved

## 2026-10-25: resolved

A real engine bug, exposed when the real runtime track's log directory calls moved the simulator's schedule (bisected to `nttptzmy`, which changed no engine logic). `heap::walk` dropped its latch between chain steps and reread the page, so a prune in between could send a redirect or `next` hop to another row: an index fetch could miss a visible row or return another. It now reads each page once per stretch of the chain on it. Two more bugs on the same path: `LargeStore::insert` could write into a store page a sweep had just freed (it now rechecks owner and names after latching), and `reserve_page` waited for a free page's latch under the alloc mutex every commit takes (it now reserves under the lock and latches outside). Tests in `heap_threads.rs` fail without each fix. `vacuum_runs_beside_writers_and_readers_holding_snapshots` saw progress only through lock contention the alloc fix removed; it now runs at switch rate 0.3. Left: `LargeStore::settle` still latches and commits a stale freed page unchanged (`large-settle-stale-page`).
