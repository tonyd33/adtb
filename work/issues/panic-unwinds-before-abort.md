---
created-at: 2026-11-10
status: open
kind: bug
priority: 300
summary: "a panic in the engine, on a host thread or a caller's thread, unwinds and releases its latches and gates before (or without) the process aborting, so other threads can act on half-done state"
tags:
  - class-failure-flattened
---

# A panic unwinds before the process aborts

## Report

## 2026-11-10

On the OS runtime a host thread runs under `abort_on_panic` (adtb-sys/src/os/mod.rs), which catches the unwind and then calls `process::abort()`. No production panic hook is installed and no profile sets `panic = "abort"`, so the panicking thread's drop glue runs first: a reindex job panicking inside `take_over` releases the index latch, `Offline`, `VacuumHold`, its read `Txn` and its job row before the abort. Meanwhile other host threads run: a writer can take the latch, write to a half-swapped tree and commit before the process ends. A plain crash never allows that, since the latch is still held at the crash. The simulator models the same order (its thread wrapper crashes after `catch_unwind`). Found reviewing structure-transactions' reindex panic change, which relies on fatal-split ending the process before anything a client reads changes.

Direction: end the process before any drop glue runs: a panic hook installed with the runtime that runs the default hook and then aborts (keeping the handling of `FatalIo` and the runtime's stop payloads), or `panic = "abort"` for the shipped binary; make the simulator crash at the panic point, not after the unwind; a test that a panic under a held latch leaves the latch held at the crash.

Defect class: Damage or failure gives a panic, hang or flattened error.

## 2026-11-11

Wider than host threads: on a caller's (embedder's) thread nothing aborts at all, so an engine-invariant panic unwinds to the caller, and an embedder that catches the unwind or runs calls on a worker thread keeps the engine serving after drop glue released the `Txn`, latches and `VacuumHold`. Since structure-transactions step 9 the write path's engine-state invariants panic on the caller's thread (`vacuum_held`, `reindexed_ids`, `conflicts`, `rels` at open, `From<heap::Error>`, `Engine::statements`' `Blocked` arm), relying on embedding.md §2.4's "ends the process". The fix covers every thread an engine call runs on: a process-wide hook installed when an engine opens, or an abort guard around each api entry point.
