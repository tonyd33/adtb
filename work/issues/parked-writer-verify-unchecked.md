---
created-at: 2026-11-11
status: open
kind: bug
priority: 490
summary: "a thread holding an open in-process transaction with writes can begin a verify (or a deferrable read) that waits on its own transaction, and no check names it"
tags:
  - class-admission-per-path
---

# A parked writer can begin a verify that waits on itself

## Report

## 2026-11-11

`begin_verify` and api `verified` require that no transaction with an id runs on the calling thread, since the verify waits for every running writer. structure-transactions step 12b asserts this under `latch-checks` with a thread-local count raised for the whole of `run_items`, which covers a `verify` command run inside a writer's statements. It misses an in-process thread that holds an open `AdHocTransaction` with writes (parked between calls) and then calls `verify` on the same database: the verify waits on that thread's own transaction and hangs. A count kept from begin to end would fire falsely on server workers, which move a `Txn` between threads. `begin_read(deferrable)` has the same wait-on-itself hazard with no stated precondition.

Direction: track the transactions an in-process handle holds open per thread (server sessions excluded), or refuse a verify or deferrable read on a handle that has an open writing transaction; a test that does exactly this and asserts a refusal, not a hang.

Defect class: Admission per path: a rule held by each caller's care instead of one gate.
