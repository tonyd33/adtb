---
created-at: 2026-10-27
status: in-progress
kind: feature
priority: 1003
summary: "server calls lack engine-owned bounded admission, resumable waits and result handles"
branch: server-results
depends-on:
  - server-engine-interface
tags:
  - engine
  - server
  - test
---

# Feature: engine-owned server call tasks and results

## Report

The approved contract in docs/design/server.md sections1–3 is not implemented.
exec/sched.rs queues unbounded run-to-completion closures, mvcc/wait.rs parks
workers, and api/text.rs plus api/arrow.rs materialize results. A socket wrapper
cannot provide genuine bounded credit suspension, held read snapshots or
cancellation while queued/waiting. Wire framing/primitives landed b27a1eb1,
not a running database server.

Implement provider-first slices behind the adopted runtime-independent owned
task/result boundary; preserve synchronous embedded API behavior. One writer
owns shared exports and scheduler signatures; land providers before transport
consumers. roles-and-grants tracks role authority, which gates authenticated
end-to-end use. Section5 of the design assigns candidate paths and ordering.
Exact Rust API signatures need provider review before consumers branch.

Acceptance includes bounded queued bytes/entries and priority, queued deadline/
cancel, resumable transaction/IO/capacity waits without held page latches,
queued commit progress when other calls wait, lost-wakeup races, actual slow-
reader production bounds, transferred batch reservations, snapshot lifetime
through encoded-but-unsent data until drain/drop, no failed-attempt write rows,
and correct cancel/commit/idle outcomes. Registry continuation additionally
requires role/binding ownership, busy/FIFO/expiry and disconnect tests.
Use simulator plus real threads and full branch/main check, clippy/fmt/house.
No runtime dependency or fake materialize-then-throttle implementation approved.
First Unix milestone may use current pins and zero optional request IDs; durable
request replay, TLS TCP and full parity remain separate required later work.

## 2026-10-28: dispatched

A fresh writer owns the second slice of design server.md section 5.2, resumable engine work, in workspace server-resumable on main 1eb25c53: bounded admission by priority class then transaction start, queued cancel and deadline, transaction waits that release the worker and hold no latch, and a queued commit progressing while every other call waits. It plans the scheduler and wait signatures first (work/briefs/plan-server-resumable.md, local) and owns those signatures. The other slices are not started. Gates: targeted adtb and adtb-sim tests with simulator and real-thread cases, clippy, fmt, house-check, size-check; the lead runs the full check. Local brief: work/briefs/brief-server-resumable.md.

## 2026-10-28: resumable engine slice frozen

Frozen c75425120a8ac4e23161d36d563ef4071261c105 on main 1eb25c53 (workspace server-resumable). Calls run in steps on the existing workers, ready by priority then transaction start then admission order; admission bounds entries and bytes of calls not yet started; cancel and deadline stop a queued, ready or waiting call; `Waits::watch` registers a waker under the lock `ended` takes, and the blocking wait is built on it; a suspending statement that reaches a transaction wait fails `Error::Blocked`, its call's statements are undone, and the call reruns them on the same transaction and snapshot when woken. The synchronous facade keeps `suspend: false`. Tests: crates/adtb-sim/tests/calls.rs, simulator and real-thread pairs.
Lead rulings: rerunning a call's statements on the same snapshot after a wait is accepted (a registered transaction has no outside effects under net/retry/server, and a retry already reruns them); the admission bound covers only calls not yet started (ops/admission/queue: the grant decides when the head runs).
Left for later slices: the memory grant deciding when the head runs, bundle worker quotas, I/O and capacity waits (`commit` still blocks in `wait_durable`), connecting `Admitted::cancel` to a running statement's cancel flag, a real-thread test of the write path, and whether the statement time limit restarts on a rerun. Running: the lead's branch check and a fresh review.

## 2026-10-29: resumable engine slice landed

Landed as 7d7aa7d5 after review fixes; the main check passed on b241565d. Slice 2 of design server.md section 5.2 is done. The ticket stays open for the result/task slice, which consumes `Scheduler::admit`, `Resumable` and `Waits::watch`, and for the items left above. The next slice also fixes `admit` dropping refused calls; follow-ups are in resumable-calls-follow-ups.

## 2026-10-29: for the result/task slice, from resumable-calls-follow-ups

- Map `Refusal::TooLarge`, `Refusal::ShutDown` and `Stopped::ShutDown` to wire errors; the two refusals are not retryable.
- On a rerun, earlier statements' rerun time counts against the limit of the statement that suspended (crates/adtb/src/write/mod.rs about 857-866). This is stricter than the blocking path and never looser; state it in docs/design/mvcc.md near line 474 with a test, or exclude rerun time if a call is refused that blocking would finish.
- No path enforces the statement limit during a wait; `Rerun.since` is private, so the slice cannot set `Entry::deadline` to when the limit runs out.
- `Scheduler` has no `Drop`; one dropped without `shutdown` never stops its calls.
- Tests: `a_waiting_write_stops_at_its_deadline_real` (crates/adtb-sim/tests/calls.rs about 1185) asserts exact events against a 100 ms deadline and can see only `[Stopped]` under load; the blocking deadlock variant (about 1304-1316) relies on the simulator's interleaving and ran 8 seeds, not the deep profile's 64; no test races a cancel against shutdown. The `ShutDown` doc comments in exec/sched.rs (about 65 and 76) restate the name.

## 2026-10-29: result/task slice dispatched

A fresh writer owns slice 3 of design server.md section 5.2, result/task ownership, plus the items carried from resumable-calls-follow-ups, in workspace server-results on main d8d9c197. It writes a plan first (work/briefs/plan-server-results.md, local): task and result handle signatures over `Scheduler::admit`, where the executor materializes today and the smallest change that stops production at a bounded handoff, capacity accounting, the cancel/commit ordering, tests, and open questions with recommendations. The lead rules on the plan before implementation; rulings are recorded here. Out of scope: interactive registry, transport. Gates: targeted adtb and adtb-sim tests with simulator and real-thread cases, clippy, fmt, house-check, size-check; the lead runs the full check.

## 2026-10-29: lead rulings on the result/task plan

Plan: work/briefs/plan-server-results.md (local). Rulings, by the plan's question numbers:

- Q1: a streamed set result keeps the set of sent elements, charged to the call's grant. A planner distinctness fact that removes it is later work.
- Q2: only single-item read-only calls whose root is a `Set` query stream; every other call is formed whole and handed out in batches as capacity allows.
- Q3: the engine starts its scheduler lazily on the first task submit; the embedded facade never starts it.
- Q4: a streamed read commits its read-only transaction at drain acknowledgement, or releases it on drop or cancel; `Read.snapshot` is its snapshot position.
- Q5: supplied capacity is additive bytes, zero at first, not restored when a batch is released.
- Q6: rerun time is excluded from the statement limit: earlier statements rerun without the limit (still under deadline and cancel), and the suspended statement's start shifts by the rerun's duration, so a suspended call and a blocking call reach the limit alike. Stated in docs/design/mvcc.md with a test. This replaces the stricter rule landed in 2f2c2f61.
- Q7: `Refusal::Overloaded` maps to `Retry { reason: Overloaded }`; `Refusal::TooLarge` to `Rejected { reason: StatementLimit { limit: Memory { bytes } } }`, `bytes` the admission byte bound; both `ShutDown`s to `Closed`. The TooLarge mapping is filed for the spec in spec-admission-too-large.
- Q8: the queue key is the priority class (the bundle's, `AdHoc` for text), then the call's `u8` priority within that class, higher first (wire.md 5.1), then transaction start. The `u8` never changes the class.
- Q9: ad hoc commands run within one step as today; recorded as deferred.
- Q10: write/stream.rs and the write/mod.rs changes are in this track; or-patterns owns only write/eval.rs there.
- Q11: no owner fallback. A task submitted under any role other than the embedded owner is refused with `Denied` until role-backed execution lands; the owner's session runs as today.

## 2026-10-29: result/task review blocks 91168f3c

The lead's branch check on 91168f3c (base d8d9c197) passed: root 2667 passed, 7 skipped; campaign 47 passed, 1 skipped. A fresh review (work/briefs/review-server-results.md, local) blocks on five findings, all ruled to fix as proposed:

- B1: a finished task keeps its whole memory grant while its handle lives; four held handles starve every later call.
- B2: a deferrable read task parks its worker in `begin_read`; with one worker this deadlocks. Use `begin_deferrable`, `defer` and `watch`.
- B3: a streamed read's statement limit counts time waiting for capacity, so a slow reader fails `StatementLimit` where `Registered::call` returns every row. Ruling: the limit counts execution only; slow readers are governed by the stream idle limit (wire.md 5.8).
- B4: a call stating no priority ranks lowest in its class. Ruling: it ranks highest (`u8::MAX`); a stated priority can only lower it (catalog.md 8.1).
- B5: observer wakes run under the `Engine.open` read guard; an inline waker re-entering the engine deadlocks against `close`.

Also fixed in the same pass: production continuing after abandon, a concurrent `close` returning before the file is released, a caller cancel token not stopping a queued task, admission bytes leaving out owned `Prepared` terms, the streamed ad hoc layout source, the priority class decided by a string comparison, the duplicated retry-count and limit construction, and timing-dependent tests.

Left for later: a pipeline with no heap source (index lookup, unnest) is one production unit, bounded only by the grant; an alarm wake drops its watch without `withdraw`; during `Engine::drop` a call whose upgrade fails drops its `Txn` without abort; `Overloaded` carries `retry_after: 0`; commit still blocks in `wait_durable`; streamed units run serially; ad hoc commands occupy a worker for one step.
