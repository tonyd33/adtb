---
title: How the server owns and resumes an engine call
pick: one
raised-by: [net/server/async]
options:
  owned:
    title: An engine-owned resumable task and result handle independent of the socket runtime
  runtime-future:
    title: An engine call is a future requiring the server runtime
  blocking-adapter:
    title: The server owns blocking calls and materializes their results
---
The async front end already submits work to the engine scheduler. Choose
which subsystem owns a queued, executing or credit-suspended call. The
question concerns the subsystem contract, not exact Rust method names.

## net/task-boundary/owned

The engine admits a call and returns an owned handle. It retains authenticated
role/binding authority, retry state, cancellation and snapshot/result lifetime.
It resumes on engine workers when dependencies or downstream capacity permit;
no thread is reserved for an idle connection or credit-suspended result.
A transport adapter receives bounded batches and completion metadata. Dropping
or cancelling the handle releases resources according to commit state.

```text
server: authenticate peer -> engine session -> checked pinned binding
engine: submit(binding, call) -> owned task
server: grant capacity -> receive batches or final outcome
engine: suspend/resume dependencies and result production on its scheduler
```

Authentication proof remains the trusted server's responsibility; grant and
principal enforcement belongs to the engine. Snapshot and metadata must describe
the actual pinned call, rather than a later Database::schema(). This does not
approve a new authentication dependency, wire format or public method spelling.

## net/task-boundary/runtime-future

The public server-facing engine call is a runtime-specific future. Its I/O,
wait and cancellation integration depends on that runtime. A simulation adapter
must reproduce it. Internal futures remain possible under the owned option;
this alternative makes the external runtime part of the engine contract.

```text
server runtime: spawn(engine.call(...)) -> runtime task/join handle
```

## net/task-boundary/blocking-adapter

The server submits existing synchronous owner calls to a separate blocking
executor and queues completed rows for socket delivery. This reuses today's
API but needs additional machinery to honor engine-owned role/grants, actual
credit-held snapshot lifetime and cancellation. Bounded workers can all park
waiting for transactions whose completion calls are still queued. Materialized
rows alone cannot implement the settled credit/snapshot contract.

```text
server: blocking_pool.submit(Database::call) -> completed rows -> socket queue
```
