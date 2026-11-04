---
created-at: 2026-11-04
status: open
kind: bug
priority: 470
summary: "A fatal I/O error on the scrub or vacuum thread unwinds uncaught and aborts the process, and no test covers it"
---

# Fatal I/O on a maintenance thread aborts the process

## Report

A write failure the driver treats as fatal (`fatal("write")` in crates/adtb/src/storage/driver.rs) panics with `FatalIo`. On a caller's thread the panic reaches the caller. On the scrub or background vacuum thread, `caught` in crates/adtb/src/api/maintenance.rs passes a `FatalIo` on up, and the thread's panic aborts the process. Background vacuum V3 found it: `real_file::uring::acknowledged_commits_survive_injected_failures_and_a_crash` aborted with SIGABRT when an injected write failure hit the vacuum thread, so that test now runs with the worker off. No test covers acknowledged commits surviving a write failure on a maintenance thread.

## Proposal

Decide what a fatal I/O error on a maintenance thread does to the database and to the host process: for example, halt the database as a crash does, so later calls refuse and a reopen recovers, without taking the host process down. Then test acknowledged commits through it with the worker and the scrub running.
