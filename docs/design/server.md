# Server

`adtb-server` implements the wire protocol of
[../spec/wire.md](../spec/wire.md) as a client of the `adtb` library.

## 1. Calls as tasks

Calls from the server become tasks on the engine's worker scheduler
([execution.md §7.1](execution.md#71-adaptive-compilation-and-morsels)),
the same scheduler that runs the engine's own pipelines.

The server boundary below is approved but not yet implemented. It is
separate from the synchronous embedded-owner entry points of §4; those
remain usable with their existing meaning. The names in the operation
tables describe ownership, not additional public Rust methods available
in the current crate.

### 1.1 Submission and ownership

Precondition: the server supplies an engine-issued role session and
call options. A registered call also supplies a binding (§3) and
arguments decoded using its immutable layout. Unbound ad hoc text is
submitted for the engine's schema and grant checks under the role
session. Wire decoding does not confer authority.

| Operation | Engine contract |
| --- | --- |
| Submit | Take ownership of arguments and options and return one owned task, or refuse without executing the call. Reserve a bounded queue entry before accepting queued work. |
| Observe | Return available metadata, a result batch, completion, or pending; never run a statement on the observing I/O thread. A pending observation registers a wake notification without a lost-wakeup window. |
| Supply capacity | Allow bounded result production under the call's memory grant. Capacity is a memory reservation, not wire credit. |
| Release batch | Release its reservation once the transport has consumed or discarded it. Holding a batch keeps its reservation charged. |
| Acknowledge drain | Mark the result delivered only after its last encoded bytes have been sent. Releasing an engine batch after encoding is not this acknowledgement. |
| Cancel or drop task | Request cancellation without waiting for a worker; resolve effects and release resources as §1.3 states. |

The task owns its admitted call, retry state, result buffers and
completion record. It owns a self-contained call's snapshot or an
operation claim on a registry transaction (§3.3). Sessions and bindings
are owned handles; a task retains what it needs without borrowing a
socket, connection callback stack or mutable embedded transaction.
None of these handles keeps a closed database usable.

### 1.2 States and result capacity

| State | Resources and progress |
| --- | --- |
| Queued | A bounded admission entry and owned request; no execution worker. Cancellation or deadline removes the entry. |
| Running | Engine workers execute finite units under the call's grant and bundle quota. |
| Waiting on a dependency | Retain the call's transaction state, release its worker, and resume when the dependency or a stop condition is ready. |
| Waiting on output | Retain the call and read snapshot, with bounded buffered output and reservations; resume production when capacity is available. After production ends, wait for drain acknowledgement or abandonment. |
| Committed with output remaining | Preserve the write's commit position and attempts while draining bounded result storage. No later cancellation changes the commit. |
| Finished | Retain completion metadata while the task handle exists; release execution, snapshot, admission and unused buffer resources. Outstanding handed-out batches retain their own reservations until released. |

Admission and grant accounting follow
[../spec/catalog.md §8](../spec/catalog.md#8-resource-control).
A task waiting for a lock, transaction, I/O, safe snapshot or capacity
must not occupy the worker needed to satisfy that dependency. Such
waits retain no page latch. Queue limits and deadlines apply while a
call is queued, as well as while executing where applicable. Owned
queued arguments and task metadata count against admission bounds;
a queue entry limit alone does not bound their storage.

The server owns compression, frame splitting, wire credit and its
encoding buffers. It must reserve those buffers and stop supplying
production capacity when downstream space is exhausted. Already
handed-out engine batches and server buffers remain charged to their
respective bounds. The engine stops producing at the bounded handoff;
materializing the complete read result before that handoff does not
satisfy the contract. A read snapshot stays held until its result is
drained or abandoned, including while wire credit is exhausted.

No result of a self-contained write reaches the server before commit.
Attempt-local output is charged to the grant, spilled or refused under
the existing resource limits; a failed retry publishes none of it.
Committed output can then drain without holding the write transaction
open. Completion identifies the pinned result layout and distinguishes
a self-contained read's snapshot position, a committed write's position
and attempt count, and an interactive call's provisional result. The
server counts elements actually sent for wire End.

### 1.3 Cancellation, completion and notification

Postconditions:

- Cancellation before a self-contained write commits discards the
  attempt and its output. Cancellation after commit preserves the
  committed completion; it never reports an uncommitted cancellation.
- A query can yield rows before a later failure; that failure is the
  completion. Cancelling or abandoning it releases its snapshot and
  unconsumed output. No success completion is invented for partial
  failed output.
- Cancelling an interactive call rolls back that call's effects and
  releases its active-operation claim, leaving its transaction open.
  A serialization failure instead aborts the transaction (§3.3).
- On disconnect the server drops its call handles, not its database's
  transaction registry. Begin, Commit and Rollback run to their own
  engine outcomes: wire Cancel is ignored, and losing their reply
  observer does not cancel the operation. A disconnect cannot reverse
  a completed commit.
- For stream idle expiry the server requests abandonment of remaining
  output. An uncommitted call fails with StreamIdle; a committed call
  retains its commit completion and marks its output incomplete, as
  [../spec/wire.md §5.8](../spec/wire.md#58-stream-idle-limit) states.
- Once an outcome is settled, status observations return the same
  outcome. Delivering batches consumes them once; observing completion
  does not replay them. Dropping the handle releases its completion
  record; durable request replay, when requested, is separate (§3).

The engine's commit decision and cancellation decision have one
ordering: the provider, rather than the transport, determines which
won. Final delivery needs no result capacity, so an exhausted stream
cannot prevent error or commit metadata from becoming observable.

Notifications signal that an observation may make progress; they
carry no execution ownership and may be coalesced or spurious. The
consumer rechecks state after registering interest. Notifications must
not invoke transport code while an engine latch or state lock is held.
The boundary accepts no socket-runtime executor or runtime-specific
future; adapters may connect its readiness to their own event loop.

Rulings: net/server/async, net/task-boundary/owned,
net/task-resumption-cost/implement, net/flow/held/idle-limit

## 2. Retry

The server delegates a self-contained registered transaction's retry
to the engine task. The engine reruns the whole unit at a fresh
snapshot on serialization failure, up to the effective retry limit;
the server must not wrap it in a second retry loop. Completion reports
the attempts made. The same applies to self-contained writing ad hoc
text as [../spec/wire.md §6.4](../spec/wire.md#64-server-retry) states.
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

### 3.1 Role sessions

Precondition: trusted server code establishes transport credentials
using a listener's accepted source and mapping. A Hello role name or
an asserted principal alone is not authentication. The embedded owner
handle authorizes the trusted server integration; it is not the
identity of an incoming network call.

The engine issues an opaque role session scoped to one database and
its authenticated role. It verifies role existence and state, derives
Service from that role, checks assertion authorization, and owns grant
enforcement and audit recording. The server supplies authentication
and connection context for successful and failed attempts; it does
not substitute an asserted Service for a role session. Registered
relation access uses the bundle owner's rights; the authenticated role
needs the call grant, not those relation grants. Authentication failure
disclosure follows
[../spec/security.md §2.2](../spec/security.md#22-sources).

Dropping a role session releases connection-local authority, not open
interactive transactions. Role and binding identity are checked by
the engine on every later submitted operation; cached handles do not
bypass lifecycle or grant changes.

Embedded open initializes the permanent owner when the roles relation is
empty. Owner initialization preserves an existing owner's identity without
a deployment write. A nonempty roles relation without the owner is refused
as malformed.

`Database::role_authority()` returns the trusted embedded owner's
administration provider. Its `create_role`, `drop_role`,
`grant_capability` and `revoke_capability` operations use the canonical
catalog roles and deployment grants. A successful mutation and its
`Grant` audit event commit together. A refused mutation leaves those
rows unchanged. Deployment identities are never reused after deletion
or reopening; failed operations may consume identities.

`RoleAuthority::session` issues an opaque session after the trusted
caller authenticates the supplied role. Sessions hold an immutable
role identity scoped to their database. `require` checks current role
existence, identity, capability and any requested grant option from
one serializable observation; dormant, log and targeted grants do not
authorize capabilities. Dropping and recreating a role does not
revive its sessions. A grant revocation affects later checks through
an existing session.

`RoleAuthority::principal` derives Service from the authenticated role
when no assertion is supplied. An assertion requires the current
`assert` capability, must have the schema's Principal type, and cannot
be Service. These providers check authority and derive identities;
they do not execute queries or authorize an operation performed later
under another handle.

### 3.2 Checked binding

Binding takes a role session, bundle name, expected schema position
and hash, and expected catalog position and hash when supplied. The
engine returns an opaque binding and immutable descriptors for the
bundle's edition, callable ordinals, argument/result types and stable
constructor identifiers. The server uses those descriptors for wire
conversion, never the later current Database::schema().

Postconditions:

- Validation checks bundle existence, both pinned version identities,
  lifecycle and role call grants as one binding decision. A mismatch
  returns the corresponding existing wire error and no binding.
- Only a successful decision makes renewal eligible. An invalid or
  denied bind writes no renewal. Renewal retains §4's throttling,
  concurrency and failure behavior; renewal eligibility and returned
  descriptors refer to the same validated bundle identity.
- Binding metadata includes the effective edition, lease expiry and
  engine-owned idle/transaction limits needed for Ready. Protocol,
  codec, transport limits and authentication-final bytes belong to the
  server. Metadata is an immutable observation, not a promise that a
  future call cannot expire or be refused.
- Before execution the engine checks the binding's lifecycle and the
  role's authority for that operation. Queue acceptance is no promise
  of future authorization. Arguments and assertions are validated
  before a refused call can renew a lease or execute.
- An ad hoc role session has no bundle binding. Ad hoc operations
  still undergo the engine's schema, grant and principal checks.

This boundary preserves the earlier-pin refusal in §4 until version
translation is implemented; a development listener does not replace
an older pin with current metadata to make it run.

### 3.3 Owned interactive transactions

The engine owns a registry of transactions, independent of sockets.
Its lookup key includes the database, authenticated role and canonical
binding identity (the same bundle and pins, or unbound), not the
particular role-session or connection handle. Begin stores the
asserted principal and effective limits with the transaction.

| Operation | Registry contract |
| --- | --- |
| Begin | Admit work and return an opaque transaction id after creation; no result borrows a connection stack. |
| Call, Commit or Rollback | Check role/binding identity, acquire the transaction's active-operation claim, and retain the claim while queued, running or draining that operation's result. |
| Same-connection continuation | Serialize requests in receive order without starting a second operation on that transaction. |
| Other-connection continuation | Refuse with TransactionBusy while another connection owns queued, running or result-draining work; otherwise permit the same role and binding to continue. |
| Connection loss | Cancel ordinary calls and release their claims after call rollback; control operations finish as §1.3 states. Retain the registry transaction until its own end or expiry. |
| Time or idle expiry | Abort using engine time and retain the expired outcome needed to answer later requests with TransactionExpired. |

Unknown, ended and wrong-role/binding lookups are indistinguishable
as UnknownTransaction. Check role and binding before disclosing an
expired transaction as TransactionExpired. A failed ordinary call
removes only its own effects, whereas a serialization failure ends the
whole transaction. Commit and rollback
release the transaction's resources at their engine decision. A
client-side transaction id grants no authority by itself.

The registry must admit transaction completion work without depending
on a worker parked by another call waiting for that transaction. Its
expiry and cancellation paths also progress while result capacity is
zero. Registry retention and durable request deduplication are
separate lifetimes; a lost non-idempotent commit reply is not proof
that the transaction did not commit.

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
- A bundle pinned at the version the database is at runs its calls. One
  pinned earlier is translated to that version at registration, and its
  calls run through the translation, their arguments carried forward
  and their results encoded at the pin. A registration with a query or
  transaction that has no translation is refused `Untranslatable`,
  naming `bundle::query`, or `BundleExpired` for a bundle whose lease
  ran out.
- Ordinals are declaration order from 1: each `import fn` in import
  order, then each query and transaction in item order. A bundle
  function takes no ordinal and is visible only to its registration.
- Binding a bundle and every call on it refuse a retired, superseded or
  expired bundle, and renew its lease when an eighth of the lease has
  passed since the renewal `catalog::bundles` records. A refused bind or
  call writes no renewal. A renewal is its own transaction, written
  while no other write of a bundle's row runs and skipped while one
  does; one that fails leaves the lease as it was and does not fail the
  bind or call.
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
| `Load::appender(target)` | the `Appender` of constructor relation `target`, a `CtorRelation`: `r.C`, or `r` of a record type, as ad hoc text names it; a `&str` is one |
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

### 4.5 Typed bundles

`adtb::typed` is the API generated code calls. It runs every call
through the entry points of §4.2 and §4.3.

| Entry point | Meaning |
| --- | --- |
| `HostValue` | a host type of a language type: `to_value(&self) -> Value`, `from_value(&Value) -> Option<Self>` |
| `BundleDef { Error, NAME, TEXT, PINS }` | a generated bundle: its error type, name, file and pinned version; `registration()` is `Registration::bundle(TEXT)` |
| `Callable { Bundle, Args, Output, InTxn, ORDINAL }` | a generated callable; `output(Called)` and `in_txn(InTxn)` convert what `Registered::call` and `AdHocTransaction::call` return |
| `Relation { KeyValue, NAME, ID }`, `Key<R>` | a generated relation variable and a key of it, holding its key value |
| `CtorRelation` | a generated constructor relation, by the name `Load::appender` takes |
| `Database::bundle(def)` | a `Bundle<B>`: `Database::registered(B::NAME)`, refused `VersionMismatch` when it pins another version than `B::PINS` |
| `Bundle::call`, `call_arrow`, `begin`, `transact` | `Registered::call` and `call_arrow` by `C::ORDINAL`; an interactive `Transaction<B>` whose `call` is `AdHocTransaction::call` |
| `rows`, `value`, `committed`, `rows_in_txn`, `value_in_txn`, `pending` | a set-valued result as `Rows<T, E>`, a result that is no set as `T`, a registered transaction's as `Committed<T>` or `Pending<T>` |
| `ctor_parts`, `ctor_fields`, `record_fields`, `next`, `of_ctor`, `of_record` | a value's parts and a value from parts, through `Textual` ([kernel.md §9](kernel.md#9-textual-syntax)) |

Host types: an integer type is `i8` to `i64`, `Text` is `String`,
`Bool` is `bool`, `Option a` is `Option<A>`, `Set a` is `Vec<A>`, a
record with no fields is `()`, `Uuid`, `Bytes`, `Decimal<S>` and the
time types are `adtb::typed`'s. A declared type is a generated struct
or enum, a `Tag` of a declared sum an enum of its constructors in name
order, and an anonymous record or sum one named after the callable
and field it occurs in.

Preconditions:

- A generated type's constructor `i` is constructor index `i` of the
  type at the version its module was generated at; its fields are the
  header fields then the constructor's own, each in declaration order,
  and an anonymous record's fields are sorted by name.

Postconditions:

- `from_value(&x.to_value()) == Some(x)` for every host value `x`
  whose every `Vec` holds distinct elements in ascending order; any
  other `Vec` comes back so.
- A call whose result is no value of the host type fails `Fatal {
  Malformed }`.
- `Rows` yields the result's elements in the order the call returns
  them.

## 5. Notes for the implementation

Non-binding.

### 5.1 Implementation status

The embedded facade already binds and renews registered calls, runs
transaction retries, and propagates stop options. It currently
materializes DynRows and ArrowStream and uses owner authority. The
role administration, capability checks and principal derivation of
§3.1 are implemented. Role-backed query execution, relation and field
grants, definer registration checks, call and family grants, checked
binding descriptors, authentication and complete audit recording
remain unimplemented. Existing bundle rows are owned by the permanent
embedded owner. Nonowner bundle and family ownership, including the
retirement-sensitive role-drop check, awaits that provider; current
role drops conservatively refuse any role named as a bundle owner.
The implemented authority provider is not an execution authorization
boundary for the existing owner facade.

There is no owned task handle or transaction registry. The scheduler
admits resumable calls within bounded queue entries and bytes, orders
them by priority class then transaction start, and stops a queued or
waiting call on cancellation or deadline. A transaction wait, including
a deferrable read's wait for a safe snapshot, can suspend its call and
release the worker; the synchronous facade still parks its thread.
I/O and capacity waits still block, and the memory grant does not yet
decide when the queue's head runs. The synchronous facade must retain
its public behavior while the providers are added.

### 5.2 Provider-first slices

The listed paths name ownership for future briefs, not concurrent
writers. Each provider lands before its consumers; one writer owns
shared api exports and scheduler signatures at a time.

| Slice | Ownership and acceptance |
| --- | --- |
| Authority and binding | `crates/adtb/src/api/server/{mod,session,binding}.rs`, `api/mod.rs` exports, catalog role/grant providers and simulator API tests. Verify authenticated role vs assertion, immutable pinned layouts, version/hash refusal, revoke/retire/expiry after bind, and no renewal on rejected arguments or denied operations. |
| Resumable engine work | `crates/adtb/src/exec/sched.rs`, executor handoff modules, `mvcc/wait.rs`, runtime readiness providers and simulator/real-thread tests. Verify bounded admission/priority, queued cancellation/deadline, lost-wakeup races and a queued commit making progress when every other call awaits a transaction. |
| Result/task ownership | `crates/adtb/src/api/server/{task,result}.rs` and executor batch handoff, preserving `api/{text,arrow,typed}.rs` facades. Verify slow readers stop actual production within grants, snapshots remain held through encoded-but-unsent data until drain/drop, transferred batches stay charged, no failed-attempt rows escape, and cancel/commit/idle races retain the correct outcome. |
| Interactive registry | `crates/adtb/src/api/server/transaction.rs`, `api/txn.rs` integration and simulator tests after task and authority APIs land. Verify same-role/binding reconnection, wrong-authority hiding, busy/FIFO behavior, call rollback vs serialization abort, limits and disconnect. |
| Transport consumer | `crates/adtb-server/src/connection/` and Unix transport modules after providers land. Verify actual peer authentication, wire credit/compression accounting and disconnect/cancel parity against provider tests. |

A first Unix development milestone uses an explicitly provisioned
current-version bundle and zero optional client request ids. It still
requires real role/grant checks, cancellation and bounded result
production. Durable request replay, complete protocol parity and TCP
TLS/authentication remain separate work; omitting their implementation
does not weaken their eventual contracts.
