---
created-at: 2026-11-10
status: open
kind: bug
priority: 300
summary: "a panic on an engine host thread unwinds and releases its latches and gates before the process aborts, so other threads can act on half-done state"
tags:
  - class-failure-flattened
---

# A panic unwinds before the process aborts

## Report

## 2026-11-10

On the OS runtime a host thread runs under `abort_on_panic` (adtb-sys/src/os/mod.rs), which catches the unwind and then calls `process::abort()`. No production panic hook is installed and no profile sets `panic = "abort"`, so the panicking thread's drop glue runs first: a reindex job panicking inside `take_over` releases the index latch, `Offline`, `VacuumHold`, its read `Txn` and its job row before the abort. Meanwhile other host threads run: a writer can take the latch, write to a half-swapped tree and commit before the process ends. A plain crash never allows that, since the latch is still held at the crash. The simulator models the same order (its thread wrapper crashes after `catch_unwind`). Found reviewing structure-transactions' reindex panic change, which relies on fatal-split ending the process before anything a client reads changes.

Direction: end the process before any drop glue runs: a panic hook installed with the runtime that runs the default hook and then aborts (keeping the handling of `FatalIo` and the runtime's stop payloads), or `panic = "abort"` for the shipped binary; make the simulator crash at the panic point, not after the unwind; a test that a panic under a held latch leaves the latch held at the crash.

Defect class: Damage or failure gives a panic, hang or flattened error.
