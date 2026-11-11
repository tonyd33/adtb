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
