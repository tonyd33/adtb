---
created-at: 2026-10-27
status: open
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
