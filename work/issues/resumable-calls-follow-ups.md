---
created-at: 2026-10-28
status: open
kind: feature
priority: 1004
summary: "a refused admit drops the call and any open transaction it owns, shutdown never stops waiting calls, and the resumable call tests miss multi-statement and deadline cases"
tags:
  - engine
---

# Feature: resumable call follow-ups

## Report

From the independent review of the resumable engine slice of server-owned-call-tasks (work/briefs/review-server-resumable.md, local). Fix the first before the result/task slice consumes `admit`.

- `crates/adtb/src/exec/sched.rs` (about 288-294): `admit` drops the refused call on `Overloaded`; a call owning a `write::Txn` (no `Drop`) leaks a running xid and the caller loses the call it must answer. Return the call inside the error.
- sched.rs (about 345-356): `shutdown` drops waiting calls without running `stop`, the deadline thread exits, and a cancel after shutdown queues a stop task no worker runs.
- sched.rs (about 142-158): `take` ignores a deadline already passed, so a call can start or resume late.
- sched.rs: the `stop` doc omits a cancelled running call whose step returns `Wait`; `CallId` is a bare `u64` alias.
- `crates/adtb/src/write/mod.rs` (about 651): `Database::waits()` exposes `started`/`ended`; expose `watch`/`withdraw` on `Database` instead.
- docs/design/mvcc.md (about 470-473) says a suspended wait undoes the statement and reruns it; the write layer undoes all the call's statements and the caller reruns them. About line 623 still says a deferrable transaction waits parked.
- Whether a rerun restarts the statement time limit.
- Tests: no call of two statements that resumes; the liveness tests never exceed the admission bound; the write-path tests run only in the simulator; the waiting-deadline path may be unexercised and no test covers a deadlock between suspended calls; `crates/adtb-sim/tests/calls.rs` (about 746) classifies errors by parsing `Debug` output.
