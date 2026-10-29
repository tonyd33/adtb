---
created-at: 2026-10-27
status: in-progress
branch: server-resumable
depends-on: [server-engine-interface]
tags: [engine, server, test]
kind: feature
priority: 1003
summary: "server calls lack engine-owned bounded admission, resumable waits and result handles"
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
