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

## 4. Registrations

`adtb` registers queries, transactions and imported schema functions in
units called registrations. A bundle carries one registration: the
imports and items of its file, without the header. The bundle layer
and the server call through these entry points:

| Entry point | Meaning |
| --- | --- |
| `Registration::new(name, text)` | a registration named `name`; `text` holds the imports and items of a bundle file with no header |
| `OpenOptions::registrations` | the registrations open checks and registers once its migrations apply |
| `Database::registered(name)` | the handle of registration `name`, else `UnknownBundle` |
| `Registered::call(ordinal, args, opts)` | runs callable `ordinal` with `args` as a transaction of its own: rows for a query or function, `Committed` for a transaction |
| `AdHocTransaction::call(reg, ordinal, args, opts)` | runs callable `ordinal` of `reg` as one call of the transaction: rows, or `Pending` for a transaction |

Preconditions:

- `args` holds one value per declared parameter, in order, each a value
  of its parameter's type.
- `opts.assert`, and `TxnOptions::assert`, is a value of the schema's
  `Principal`.

Postconditions:

- Open registers every given registration or none. A refused one fails
  open with `RegistrationRefused` naming it; the reason is `Invalid`
  and names the callable and the failure.
- Every query, transaction and bundle function is checked and
  normalised at the schema version open leaves the file at. A call
  never parses or type checks.
- Ordinals are declaration order: each `import fn` in import order,
  then each query and transaction in item order. A bundle function
  takes no ordinal and is visible only to its registration.
- A call runs for `opts.assert`, else for `Service { name: "owner" }`,
  the schema-owner role the handle acts as. Asserting a `Service` value
  is refused with `Denied`. A schema function a call reaches sees the
  call's `caller`.
- A registration lives as long as the handle that opened with it. Open
  registers only what it is given.

TODO: how a bundle's pin, translation and lease reach a registration is undecided.

## 5. Notes for the implementation

Non-binding.

The server has an asynchronous I/O front end: I/O threads read and
write frames, and each call becomes a task on the engine's morsel
scheduler.
