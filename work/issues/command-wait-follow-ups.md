---
created-at: 2026-10-30
status: open
kind: bug
priority: 560
summary: "verify reruns from its pages phase on each block; polling jobs can be delayed without bound; a deferrable read with a command and verify_without_transaction wait on a worker, the second ignoring deadline and cancel; a stop in BTree::bulk causes a needless sweep; undo_overwrite always restores Checksum"
---

# Command wait follow ups

## Report

## 2026-10-30: from command-rerun

A non-suspending write's wait on another transaction now polls at `GATE_POLL` (`Apply::wait`) so it can see its stops. An embedded key wait therefore wakes up to 5 ms after its holder ends, where a park woke at once. Wake it when the holder ends and keep the bound for stops. Measure with the benchmarks.

A write waiting at the offline rebuild gate (`wait_for_rebuilds`) answers `Cancelled` on close, where `Apply::wait` answers `Closed`. A command's vacuum lock wait is charged to its item's limit, so a call can report `StatementLimit` with its setting changed.
