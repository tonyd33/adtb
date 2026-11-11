---
created-at: 2026-11-11
status: open
kind: bug
priority: 480
summary: "lock orders in storage and the simulator, and the no-latch-across-wait rule of mvcc::Waits, are stated only in comments, outside the rank checks"
tags:
  - class-admission-per-path
---

# Lock orders outside the rank table

## Report

## 2026-11-11

structure-transactions step 12b made the write path's and the scheduler's lock orders one checked rank table (crates/adtb/src/ranks.rs, adtb-sys `Ranked`), with waits checked under `debug_assertions` or `latch-checks`. Still stated only in comments: the lock orders in storage (archive.rs, checkpoint.rs, pager.rs) and in adtb-sim/src/sim.rs, and `mvcc::Waits`' "no latch held across `wait`". A nesting that breaks one of them deadlocks (or hangs under the simulator) with nothing naming it.

Direction: give these locks ranks in the same table (or a storage table below it) and assert no page latch at `Waits::wait`'s entry; the sim suite runs with the checks on.

Defect class: Admission per path: a rule held by each caller's care instead of one gate.

## 2026-11-11: quality review of the transactions restructure

Two more of the same kind, found reviewing `reviewed..main`:

- The transactions' own locks, `mvcc::Shared`'s `state`, `clog` and `marked` (crates/adtb/src/mvcc/txn.rs:138-144), are unranked; their order, state before commit log, is stated only in the `Transactions` and `Begun` doc comments (txn.rs:91-94, :813-814). `write::Database.vacuuming` (crates/adtb/src/write/mod.rs:624) is unranked and is held across `session.waits.started` (rank WAITS) in reindex.rs `try_hold` (about line 351), so a cycle through it is not caught.
- Dropping a `Reader`, `Hold` or `Pinned` with no page latch held (mvcc.md §9) is unchecked: `Holding::drop` and `Pinning::drop` (txn.rs:854, :899) take the transactions' state lock, which is held while commit-log and history pages are written. Direction: assert `latched_here() == 0` in `Release::release` and the unpin path, as `refuse_named` does.
