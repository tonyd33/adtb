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
units called registrations. A bundle carries one registration: its file,
header included. The bundle layer and the server call through these
entry points:

| Entry point | Meaning |
| --- | --- |
| `Registration::bundle(text)` | the registration of the bundle file `text`, named by its header; `Invalid` when the file does not parse or its header states a clause the engine does not run |
| `Registration::new(name, text)` | a registration named `name` that is no bundle; `text` holds the imports and items of a bundle file with no header |
| `OpenOptions::registrations` | the registrations open records, checks and registers once its migrations apply |
| `Database::registered(name)` | the handle of registration `name`, bound to it; else `Retired`, `BundleSuperseded`, `BundleExpired` or `UnknownBundle` |
| `Registered::call(ordinal, args, opts)` | runs callable `ordinal` with `args` as a transaction of its own: rows for a query or function, `Committed` for a transaction |
| `Registered::call_arrow(ordinal, args, opts)` | runs it as `call` does and returns its result as an `ArrowStream` of `RecordBatch`es of one `ArrowSchema` |
| `AdHocTransaction::call(reg, ordinal, args, opts)` | runs callable `ordinal` of `reg` as one call of the transaction: rows, or `Pending` for a transaction |
| `retire bundle B`, `supersede bundle B by B2` | commands of ad hoc text, returning the bundle's row of `catalog::bundles` |

Preconditions:

- `args` holds one value per declared parameter, in order, each a value
  of its parameter's type at the version the registration is checked
  at.
- `opts.assert`, and `TxnOptions::assert`, is a value of the schema's
  `Principal`.

Postconditions:

- Open records every given bundle in `catalog::bundles` and marks every
  other active bundle superseded, as one durable commit before its
  first migration. It then registers every given registration or none.
  A refused one fails open with `RegistrationRefused` naming it.
- A bundle's queries, transactions and functions are checked and
  normalised at the version its header pins, and one that is no
  bundle's at the version open leaves the file at. A call never parses
  or type checks.
- A bundle pinned at the version the database is at runs its calls. A
  call of one pinned earlier is refused `Untranslatable`, naming
  `bundle::query`, until version translation serves it.
- Ordinals are declaration order from 1: each `import fn` in import
  order, then each query and transaction in item order. A bundle
  function takes no ordinal and is visible only to its registration.
- Binding a bundle and every call on it refuse a retired, superseded or
  expired bundle, and renew its lease when an eighth of the lease has
  passed since the renewal `catalog::bundles` records.
- A call runs for `opts.assert`, else for `Service { name: "owner" }`,
  the schema-owner role the handle acts as. Asserting a `Service` value
  is refused with `Denied`. A schema function a call reaches sees the
  call's `caller`.
- A bundle's registration, its calls in `catalog::registered` and its
  lease survive the handle; a registration that is no bundle's lives as
  long as the database that opened with it.
- A `Registered` does not keep its database open. `Database::close`
  closes the database whatever `Registered` handles and `Database`
  clones remain, once every running call has returned; every later call
  through any of them, `begin` included, is refused with `Invalid`
  naming the database closed. An interactive transaction open at close
  ends uncommitted; its later calls and its commit are refused alike.

A code generator emits, for each bundle, its file's text and the
ordinals it computes from that file; the embedded open of
[../spec/embedding.md](../spec/embedding.md#21-open) passes each
generated bundle to open as `Registration::bundle` of that text, so
its ordinals are those `Registered::ordinal` returns.

### 4.1 Opening

`Database::open(path, log, options)` opens the file of the host's
runtime; `Database::open_in(rt, name, log, options)` opens file `name`
of runtime `rt`.

Postconditions:

- `Database::open` runs on `Backend::Uring` where `OsRuntime::try_new`
  admits it, else on `Backend::Pool`
  ([runtime.md §2](runtime.md#2-io-interface)).
- An open with `options.create` that creates the file and then fails
  removes the file and its log, and leaves nothing behind; an open of
  a file that existed removes nothing.
- Open checks `options.host_fns` against the schema before its first
  migration, and again once its migrations apply: a supplied function
  the schema declares with a host body at another determinism, at a
  type no batch column holds, or at another version fails open with
  `HostFnMismatch`, unless a migration open applies redefines it at the
  supplied version; a declared host function with no body that a
  constraint, check, domain, default, computed field, index or stored
  view reaches fails open with `HostFnMissing`.
- `sys::process_options` holds one row per open-time setting, fixed
  until the database closes.

### 4.2 Calls

| Entry point | Meaning |
| --- | --- |
| `CancelToken::new()`, `cancel()`, `is_cancelled()` | a token shared by its clones, cancelled once and for good |
| `CallOptions { cancel, deadline, limit, .. }` | the token, wall-clock deadline and statement time limit of one call |
| `TxnOptions { deadline, limit, .. }` | the deadline and statement limit of every call of an interactive transaction |
| `TxnCallOptions { cancel, deadline, limit, .. }` | those of one call inside it |

Postconditions:

- A call whose token is cancelled, or whose deadline has passed, when
  it starts or before it commits fails with `Cancelled` or
  `DeadlineExceeded` and has no effect. One running a query stops at
  its next morsel boundary, failing alike, or with `StatementLimit {
  Time }` once a statement runs past `limit`. A call that commits
  returns its commit whatever the token does after.
- A failed call inside an interactive transaction leaves the
  transaction open without its effects.

### 4.3 Arrow batches and loads

A `RecordBatch` is a `batch::Batch` under an `ArrowSchema`, a list of
named `Field`s each holding the `batch::Type` of its column; a sum's
type lists its constructors with their constructor ids.

| Entry point | Meaning |
| --- | --- |
| `Database::load(opts)` | a `Load`: a write transaction whose snapshot is taken now |
| `Load::appender(target)` | the `Appender` of constructor relation `target`, `r.C`, or `r` of a record type, as ad hoc text names it |
| `Appender::schema()` | the `ArrowSchema` of a batch holding every field of the relation's row |
| `Appender::push(batch)` | stages the batch's rows |
| `Load::commit()` | inserts every staged row as one change set and commits: a `LoadReport` of the rows and the position |

Postconditions:

- A result's columns are those of the wire's Arrow format: one per
  field of a record element, in declared order, one `row` column of a
  sum of two or more constructors, else one `value` column.
- `push` refuses a batch with `SchemaMismatch`, staging nothing, unless
  its fields are those of `schema()`, in order and at their types, less
  fields a key generator or a default fills.
- `commit` fails, inserting nothing, as an insert of the staged rows
  fails, and is not rerun. Dropping a `Load` discards what it staged.

### 4.4 Host functions

`HostFns::register(name, version, det, body)` supplies the body of the
host function declared as `host "name" version "version"`. A body takes
a `Batch` of one column per parameter and returns a `Batch` of one
column of the result type with as many rows; it may run on several
threads at once.

Postconditions:

- A statement calls a body once per batch of the rows it evaluates the
  call at. A body that returns an error, panics, or returns a batch of
  another type or length faults each of those rows with
  `FunctionFailed`.
- A call that reaches a host function with no body fails with `Fatal {
  MissingFunction }`.

## 5. Notes for the implementation

Non-binding.

The server has an asynchronous I/O front end: I/O threads read and
write frames, and each call becomes a task on the engine's morsel
scheduler.
