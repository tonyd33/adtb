---
created-at: 2026-10-25
status: open
kind: bug
priority: 499
summary: "after crashes beside a concurrent vacuum, an index read returns other rows than the heap at seed 5"
tags: [engine, sim]
---

# Bug: index and heap disagree after a crash during vacuum

## Report

`crates/adtb-sim/tests/vacuum_threads.rs` `crashes_beside_concurrent_vacuum_recover_clean`, at switch rate 0.3, panics at line 425 for seed 5: "Snapshot { xmin: Xid(78), xmax: Xid(78), in_progress: [] } reads other rows by index". It passed on the test-support track's base (luprkwrr) and fails on main after the internal error, real runtime, textual syntax, migration log and constraint tracks landed; the real runtime track changed the simulator's disk (directories, crash pruning) and the constraint track changed SSI reads in `exec/store.rs`.
