# Server

`adtb-server` implements the wire protocol of
[../spec/wire.md](../spec/wire.md) as a client of the `adtb` library.

## 1. Calls as tasks

Calls from the server become tasks on the engine's worker scheduler
([execution.md §7.1](execution.md#71-adaptive-compilation-and-morsels)),
the same scheduler that runs the engine's own pipelines.

TODO: the interface by which the server submits a call to `adtb` and receives its batches is undecided.

## 2. Retry

The server reruns a registered transaction on a serialization failure
before replying ([../spec/transactions.md §5.5](../spec/transactions.md#55-retry)).
An interactive transaction is never rerun by the engine.

Rulings: engine/retry/retry-block

## 3. Request ids and binding

Idempotent write calls are deduplicated as
[../spec/transactions.md §9](../spec/transactions.md#9-idempotent-write-calls)
states. Postconditions the engine provides to the server:

- When a transaction carrying a request id commits, the engine records
  the id and the reply, with its result rows, in the same commit. Only
  committed outcomes are recorded: an error, including a constraint
  violation or a serialization failure, leaves no record.
- Every attempt the server makes for one call carries the call's
  request id, and only the attempt that commits is recorded.
- A client's low-water mark is one above the highest number of that
  client whose record has been purged at the end of the retention
  window.
- The recorded ids and low-water marks are a system relation.

TODO: how the server binds a connection to a bundle through `adtb` ([../spec/wire.md §3.4](../spec/wire.md#34-binding-and-ready)) is undecided.

## 4. Notes for the implementation

Non-binding.

The server has an asynchronous I/O front end: I/O threads read and
write frames, and each call becomes a task on the engine's morsel
scheduler.
