# In-process API

This part specifies the API through which a program links the engine
and uses a database file directly: opening a file, which also migrates
it and registers the program's bundles; calling registered queries and
transactions; interactive transactions; ad hoc text; cancellation;
configuration; bulk appends of Arrow batches; host functions; log
shipping and embedded replicas. Each operation is stated in
language-neutral terms, followed by its signature in the Rust
reference binding. Every binding for another language must offer the
same operations with the same meaning. The server is a client of this
API ([wire.md](wire.md#14-parity-with-the-in-process-api)).

Examples use the running schema and a bundle compiled into the
program by code generation
([applications.md](applications.md#11-what-is-registered)):

```adt
-- bundles/app.adb
bundle app-1.3 pins 12 (c41e...) edition "2026-10"
query login_key (u: Key users) -> Set Identity =
  from i in identities where i.user == u select i
txn sign_up (n: Text, g: Text) -> Key identities =
  transact
    let u = insert users { name: n }
    let i = insert identities.Google { user: key u, gid: g }
    return key i
```

Code generation emits `schema::LOG` (the operator log the program
ships), `queries::BUNDLE` (the bundle), `queries::Error` (the bundle's
error type, §4.3), and one value per query, transaction and imported
function (`queries::login_key`, `queries::sign_up`) carrying its
ordinal and its argument and result types. Code generation computes
ordinals and constructor ids itself: ordinals are declaration order in
the bundle file, and constructor ids follow from the operator log
([wire.md](wire.md#43-records-and-sums)), so the values it emits equal
those every database replaying the same log assigns.

## 1. The embedded engine

### 1.1 One process per file

The engine is a library. A database is one data file, whatever its
schema, plus its log directory beside it, named after the file:

```text
app.adt        the data file
app.adt-log/   the log segments
```

One OS process at a time has it open. Opening takes an exclusive lock on the file
that the operating system releases when the process exits; while it
is held, an open from any other process fails with `Locked`, carrying
the owning process id when the platform reports it. There is no
read-only sharing between processes. Other processes reach the
database through a server that has it open ([wire.md](wire.md)), or
through an embedded replica (§12.2). Opening a file the same process
already has open fails with `AlreadyOpen`; the process shares the one
handle instead.

The file must be on a filesystem that honours the lock and direct
I/O; open refuses one that does not.

### 1.2 Threads

A database handle may be used from any number of threads of the
owning process at once; calls from different threads run concurrently
as separate transactions under serializable isolation. A handle is
cheap to clone; clones share one engine. There is no connection
object: a call needs only the handle.

### 1.3 Platforms

Linux is the tuned platform. macOS and Windows are supported at lower
speed. Every guarantee of this
specification holds on every supported platform: a platform on which
the engine cannot bypass the OS page cache and force data to stable
storage is not supported.

Rulings: engine/embedded-server, surface/wire/embedded, project/nongoals/multi-process, project/concurrency/app-threads, buffer/platform/linux-first

## 2. Opening a database

### 2.1 Open

Open takes a path, the program's operator log, the program's bundles
and the open-time options (§8.1), and returns a database handle. It
performs these steps in order, and returns only when all succeed:

1. Take the file lock (§1.1). If the file does not exist and the
   options allow creation, create it with the format settings the
   options give ([catalog.md](catalog.md#10-configuration)), including
   whether it is encrypted ([security.md](security.md#9-encryption-at-rest)).
2. Obtain the key-encryption key from the options' key source, if the
   file is encrypted. A missing key fails with `KeyRequired`, a wrong
   one with `WrongKey`.
3. Recover: the file then reflects every commit acknowledged before
   the last close or crash.
4. Compare the shipped log with the file's log migration by migration.
   One must be a prefix of the other; otherwise open fails with
   `Diverged` naming the first version where they differ. Open never
   writes to the log anything the shipped log does not hold. Then
   refuse a downgrade, as below.
5. Record the given bundles, unchecked, as the bundles this open
   serves. A recorded bundle is not a reader until step 9 registers
   it.
6. Mark every other bundle registered by an earlier open superseded.
   A superseded bundle is not a live reader and holds back no change;
   it is not retired, and registering it again with identical content
   runs the checks of
   [applications.md §3.2](applications.md#32-bundles-are-immutable)
   and makes it live again. Steps 5 and 6 commit durably together,
   before step 8; a later failure of open does not undo them.
7. Bind every host function the options supply (§10), before any
   staged job or plan can call one. A missing or mismatched one fails
   open as §10.2 states.
8. Apply, in order, every migration of the shipped log beyond the
   file's log ([evolution.md](evolution.md#1-the-operator-log)). Each
   applies as one schema change and commits before the next starts. A
   migration with staged jobs commits its version only when every
   staged job has completed; open runs them to completion, and every
   check of [evolution.md §3.2](evolution.md#32-checks-at-apply) reruns
   at its final commit. A refused migration, one whose staged job
   fails, or one that fails a check at its final commit, fails
   open with `MigrationRefused`, leaving the file at the last migration
   that committed and the log unchanged beyond it.
9. Check and register every given bundle at its pin, renewing the lease
   of one already registered
   ([applications.md](applications.md#41-registration)), and plan its
   queries. A refused registration fails open with
   `RegistrationRefused`.

The registrations of step 9 are atomic: they take effect together when
open succeeds, and an open that fails makes none. The recording and
supersession of steps 5 and 6, and migrations that committed before a
failure, stay applied.

A downgrade, an open whose bundles pin an older version than the
file's, fails with `DowngradeRefused` and changes nothing when an old
bundle would read a constructor enabled since its pin, or when a host
function it reaches was redefined since; `cause` names which.

When the file's log is longer than the shipped log (a newer program
has migrated the file) and no `DowngradeRefused` cause holds, step 8
applies nothing and the bundles, which pin an older version, are
served at that version by version translation
([evolution.md](evolution.md#7-version-translation)).

```rust
let db = adtb::open("app.adt", &schema::LOG, &[&queries::BUNDLE], OpenOptions::default())?;
```

Explicit retirement (`retire bundle web-2.3`) is permanent, as on a
server ([applications.md](applications.md#43-retirement)).

```rust
pub fn open(
    path: impl AsRef<Path>,
    log: &Log,
    bundles: &[&dyn AnyBundleDef],
    options: OpenOptions,
) -> Result<Database, OpenError>;

pub enum OpenError {
    Locked { pid: Option<u32> },
    AlreadyOpen,
    NotFound,
    KeyRequired,
    WrongKey,
    FormatTooNew { written_by: String },
    Corrupt { detail: String },
    Diverged { version: u64 },
    MigrationRefused { version: u64, reason: adtb::Error },
    RegistrationRefused { bundle: String, reason: adtb::Error },
    HostFnMissing { name: String },
    HostFnMismatch { name: String },
    DowngradeRefused { cause: String },
    Io(std::io::Error),
}
```

Open takes a slice of bundles of any editions; every generated
`BundleDef` implements the object-safe `AnyBundleDef`.

`FormatTooNew` is returned by a release older than the one that last
finalized the file's format ([catalog.md](catalog.md#122-upgrades-and-downgrade)).
A new engine release keeps writing the old format until the program
finalizes the upgrade, which needs no `maintain` check in process. When
readers-first refuses a catalog operator the upgrade applies,
finalizing is refused with a report naming the blocking bundles; the
upgrade stays unfinalized and a downgrade stays possible
([catalog.md](catalog.md#122-upgrades-and-downgrade)).

```rust
impl Database {
    pub fn finalize_upgrade(&self) -> Result<(), adtb::Error>;
}
```

### 2.2 Identity of the handle

A handle opened in process acts as the file's schema-owner role and
holds every privilege, operational capabilities included
([security.md](security.md#3-grants)). It authenticates no one. A call
with no assertion runs with `caller` equal to `Service { name: "owner" }`,
the schema-owner role; a call or transaction may assert an
end-user principal in its options
([security.md](security.md#13-where-the-principal-comes-from)).

### 2.3 Close

Close waits for running calls to finish, fails with `Closed` a call that
is queued or waiting on a lock, ends open interactive
transactions by rolling them back, takes a checkpoint, and releases the
lock. Dropping the last handle closes the database. A process that
exits without closing loses nothing committed: the next open recovers
from the log.

```rust
impl Database {
    pub fn close(self) -> Result<(), adtb::Error>;
}
```

### 2.4 Engine failures

A failed write or flush of the data file or its log is not returned to
the program as an error, and neither is a broken internal invariant of
the engine. Either ends the process. Commits that were waiting on the
failed flush were not acknowledged: their calls never returned. The
next open recovers as step 3 of §2.1 states.

Rulings: surface/open-applies, engine/embedded-server, ops/admin/role/capabilities, buffer/fsync-fail/panic-replay, project/arch/errors/fatal-split

## 3. Bundles

A call names a registered query or transaction of a bundle. The
program obtains a bundle handle for each bundle it passed to open; a
bundle handle is bound to its bundle at the bundle's pinned version, as
a connection is ([wire.md](wire.md#34-binding-and-ready)). Results are
typed and encoded at that version.

```rust
impl Database {
    pub fn bundle<B: BundleDef>(&self, def: &B) -> Result<Bundle<B>, B::Error>;
}

let app = db.bundle(&queries::BUNDLE)?;
```

`bundle` fails with `UnknownBundle` for a bundle not registered,
`Retired` for a retired one, `BundleSuperseded` for a superseded one,
`BundleExpired` for an expired one, which is live again only once
registered again ([evolution.md §7.3](evolution.md#73-expired-bundles)), and
`VersionMismatch` when the registered bundle of that name pins another
version. Obtaining a bundle handle and every
call on it renew the bundle's lease; the engine writes a renewal for a
bundle at most once per eighth of the lease
([applications.md](applications.md#42-leases)). Once the bundle is
retired, calls on its handles fail with `Retired`.

Rulings: net/parity/mirror

## 4. Calls

### 4.1 Call

A call runs one registered query or transaction, or one schema
function the bundle imports, with its arguments and the call options, and returns its result as host values of the types
code generation emitted.

| Option | Meaning |
| --- | --- |
| `deadline` | Stop the call at this wall-clock time ([transactions.md](transactions.md#54-time-limit)) |
| `limit` | Statement limit for this call, at most the declared one |
| `priority` | Priority within the bundle's priority class ([catalog.md](catalog.md#81-admission)) |
| `cancel` | A cancellation token (§7) |
| `assert` | An end-user principal for this call |
| `durability` | Commit level of a registered transaction ([transactions.md](transactions.md#7-durability-levels)) |
| `after` | Read no snapshot older than this commit position |
| `trace` | The caller's trace context, joined by the engine's spans |
| `retry` | For a registered transaction, the most reruns on serialization failure: `retry n` allows at most `n + 1` attempts; by default the transaction's declared limit, else the file setting `RetryLimit` |

A call, and ad hoc text (§6), passes the engine's admission
([catalog.md §8.1](catalog.md#81-admission)) as a server call does: it
waits in the same queue, fails with `Retry { reason: Overloaded {..}
}` when the queue is full, and with `Rejected { reason: StatementLimit
{ limit: Memory { bytes } } }` when it alone holds more than the
queue's byte bound.

A query that only reads runs as a read-only transaction under
serializable snapshot isolation, without waiting for a safe snapshot,
and may fail with `Serialization`
([transactions.md](transactions.md#64-read-only-transactions)). It
returns a stream of result elements as they are produced; dropping
the stream cancels the call. A registered
transaction runs its block, retries it on serialization failure as the
server does ([wire.md](wire.md#64-server-retry)), commits, and returns
its result with the commit position. A `retry n` option allows at most
`n + 1` attempts; with none, the transaction's declared limit applies,
else the file setting `RetryLimit`
([catalog.md §10.2](catalog.md#102-file-settings)), in process as on
the server ([transactions.md](transactions.md#62-retryable-units)).

```rust
let mut rows = app.call(queries::login_key, (u,), &CallOptions::default())?;
for i in &mut rows {
    match i? {
        Identity::EmailPass { email, .. } => println!("{email}"),
        Identity::Google { gid, .. } => println!("{gid}"),
    }
}
let done = app.call(queries::sign_up, ("ann".into(), "g-123".into()), &CallOptions::default())?;
let k: Key<Identities> = done.value;
```

```rust
pub trait Callable {
    type Bundle: BundleDef;
    type Args;
    type Output;
    type InTxn;
    const ORDINAL: u32;
}

impl<B: BundleDef> Bundle<B> {
    pub fn call<C: Callable<Bundle = B>>(&self, c: C, args: C::Args, opts: &CallOptions) -> Result<C::Output, B::Error>;
    pub fn call_arrow<C: Callable<Bundle = B>>(&self, c: C, args: C::Args, opts: &CallOptions) -> Result<ArrowStream<B>, B::Error>;
}

pub struct Rows<T, E> { /* Iterator<Item = Result<T, E>> */ }
pub struct Committed<T> { pub value: T, pub position: Position, pub attempts: u32 }
pub struct Position { pub epoch: u32, pub lsn: u64 }

pub struct CallOptions {
    pub deadline: Option<std::time::SystemTime>,
    pub limit: Option<std::time::Duration>,
    pub priority: Option<u8>,
    pub cancel: Option<CancelToken>,
    pub assert: Option<Principal>,
    pub durability: Durability,
    pub after: Option<Position>,
    pub trace: Option<TraceContext>,
    pub retry: Option<u32>,
}

pub enum Durability { Local, Quorum }
```

For a query or a function `C::Output` is `Rows<T, B::Error>` of its
element type `T`, or the value itself for a function whose result is
not a set; for a registered transaction it is `Committed<T>` of its
result type.

### 4.2 Arrow results

`call_arrow` returns the result as a stream of Arrow record batches
with one schema, mapped from the result type as the wire's Arrow format
maps it ([wire.md](wire.md#54-arrow-format)), including the stable
constructor id metadata on every union child.

```rust
pub struct ArrowStream<B: BundleDef> { /* Iterator<Item = Result<arrow::record_batch::RecordBatch, B::Error>> */ }
impl<B: BundleDef> ArrowStream<B> {
    pub fn schema(&self) -> arrow::datatypes::SchemaRef;
}
```

### 4.3 Errors

Every call returns either its result or a value of the prelude sum
`Error` ([catalog.md](catalog.md#4-error-sums)), the same values the
wire sends ([wire.md](wire.md#11-errors)): `Retry`, `Rejected` or
`Fatal`, each with a reason. In the Rust binding each bundle has its
own error type, `B::Error`, generated from the edition the bundle
names, with the variants `Retry`, `Rejected` and `Fatal`, each holding
its reason enum, so a program holding bundles of two editions matches
each bundle's errors as its edition declares them. Operations not tied
to a bundle (open, ad hoc text, loads) return `adtb::Error`, the sum
of the newest edition the engine implements. Errors this part names, such as `Cancelled` or `SchemaMismatch`,
are reasons. In process there is no lost
acknowledgement: a call that returns an error has not committed, and
calls carry no request numbers. An embedded replica numbers the writes
it forwards itself (§12.2), and the same holds there.

A call, an interactive transaction's statement or commit, or a load on
a database after `close` (§2.3), through any handle, fails with
`Rejected` `Closed` and has no effect. Only the in-process API raises
`Closed`.

### 4.4 Feeds

A bundle's feeds ([wire.md](wire.md#9-feeds)) are subscribed in
process too: from a snapshot, after a position, or from a named
checkpoint. The subscription yields the snapshot rows, then one change
set per commit, encoded at the bundle's pinned version; acknowledging
a position stores it under the checkpoint name. Positions older than
the retention window fail with `TooOld`, and positions before the
feed's registration with `HistoryUnavailable`.

```rust
impl<B: BundleDef> Bundle<B> {
    pub fn subscribe<F: FeedDef<Bundle = B>>(&self, f: F, start: FeedStart) -> Result<Feed<F>, B::Error>;
}

pub enum FeedStart { Snapshot, After(Position), Checkpoint(String) }

impl<F: FeedDef> Feed<F> {
    pub fn next(&mut self) -> Result<FeedEvent<F::Change>, <F::Bundle as BundleDef>::Error>;
    pub fn ack(&mut self, position: Position) -> Result<(), <F::Bundle as BundleDef>::Error>;
}

pub enum FeedEvent<C> { Snapshot(C), SnapshotEnd(Position), Changes { position: Position, changes: C } }
```

### 4.5 Notices

The notices the server pushes ([wire.md](wire.md#10-pushes)) are
delivered in process to a receiver the bundle handle returns, as
values of the prelude sum `Notice` in the edition the bundle names,
so each bundle has its own notice type as it has its own error type.

```rust
pub trait BundleDef {
    type Error;
    type Notice;
}

impl<B: BundleDef> Bundle<B> {
    pub fn notices(&self) -> std::sync::mpsc::Receiver<B::Notice>;
}
```

Rulings: net/parity/mirror, surface/wire/embedded, net/results/arrow, net/feed/changes, net/feed/version/bundle, net/push/frames, spec/txn/retry-count/n-plus-one

## 5. Transactions

### 5.1 Interactive transactions

Begin opens an interactive transaction with its options (read-only,
deferrable, a deadline, a statement limit, a priority, an asserted
principal, a durability level, a bookmark). Only a `deferrable`
read-only transaction waits for a safe snapshot.
Calls on the transaction run inside it one at a time; commit applies
its change set and returns the commit position; rollback discards it.
Commit may name a durability level; the transaction commits at the
strongest level named by its begin, its commit or any block run in it
([transactions.md §7.1](transactions.md#71-levels)).
A transaction dropped without commit is rolled back. The interactive
transaction time and idle limits apply as on the server
([wire.md](wire.md#62-interactive-transactions)), and the engine never
retries an interactive transaction: a serialization failure is
returned and the transaction is aborted.

Calls inside a transaction take their own options, which hold only
what the wire allows on a call with `txn`: no assertion, durability,
`after` or retry, which belong to the transaction. A call that would
run at another schema or catalog version than the transaction's earlier
statements, such as ad hoc text after a registered call or a registered
call after ad hoc text, is refused with `MixedVersions`
([applications.md §5](applications.md#5-binding-a-connection)). A registered transaction
called inside a transaction returns its result as `Pending<T>`, which
has no commit position; the position is the one `commit` returns.

```rust
impl<B: BundleDef> Bundle<B> {
    pub fn begin(&self, opts: &TxnOptions) -> Result<Transaction<B>, B::Error>;
}

impl Database {
    pub fn begin(&self, opts: &TxnOptions) -> Result<AdHocTransaction, adtb::Error>;
}

pub struct TxnCallOptions {
    pub deadline: Option<std::time::SystemTime>,
    pub limit: Option<std::time::Duration>,
    pub priority: Option<u8>,
    pub cancel: Option<CancelToken>,
    pub trace: Option<TraceContext>,
}

pub struct Pending<T> { pub value: T }

impl<B: BundleDef> Transaction<B> {
    pub fn call<C: Callable<Bundle = B>>(&mut self, c: C, args: C::Args, opts: &TxnCallOptions) -> Result<C::InTxn, B::Error>;
    pub fn query_text(&mut self, text: &str, params: &[(&str, Value)], opts: &TxnCallOptions) -> Result<Vec<DynRows>, adtb::Error>;
    pub fn commit(self, durability: Option<Durability>) -> Result<Position, B::Error>;
    pub fn rollback(self) -> Result<(), B::Error>;
}

pub struct TxnOptions {
    pub read_only: bool,
    pub deferrable: bool,
    pub deadline: Option<std::time::SystemTime>,
    pub limit: Option<std::time::Duration>,
    pub priority: Option<u8>,
    pub assert: Option<Principal>,
    pub durability: Durability,
    pub after: Option<Position>,
}
```

Inside a transaction a registered transaction's block runs as part of
the enclosing transaction and commits with it. `C::InTxn` is `Rows<T,
B::Error>` for a query and `Pending<T>` for a registered transaction.
`Database::begin` opens an ad hoc transaction, the in-process form of
Begin in an ad hoc session; an `AdHocTransaction` offers `query_text`,
`commit` and `rollback` as a `Transaction` does.

### 5.2 Retrying a closure

`transact` runs a closure in a new interactive transaction and commits
it, rerunning the closure in a fresh transaction when the transaction
fails with a serialization error, up to a limit. The library reruns
only in-process closures; nothing reruns a closure across the wire. The closure must
have no effects outside the database, since it may run several times
([transactions.md](transactions.md#55-retry)).

```rust
impl<B: BundleDef> Bundle<B> {
    pub fn transact<T>(
        &self,
        opts: &TxnOptions,
        attempts: u32,
        f: impl FnMut(&mut Transaction<B>) -> Result<T, B::Error>,
    ) -> Result<Committed<T>, B::Error>;
}
```

Rulings: net/parity/mirror, spec/txn/one-version/one-pair, spec/txn/durability/strongest

## 6. Ad hoc text

The handle may run ad hoc text: queries, writes, `transact` blocks,
`temp relation` declarations and operational commands, among them
`register bundle`, `apply migrations`, `print schema`, `archive
audit`, `rotate key`, `dump`, `restore` and `backup`
([applications.md](applications.md#7-the-ad-hoc-role),
[catalog.md](catalog.md#91-commands)), checked against the schema
version of its transaction's snapshot. A free name in the text is a parameter that `params`
must supply; several items in one text form one transaction.
`query_text` returns one `DynRows` per item that yields a result, in
item order. The first item that faults, fails, violates a constraint
or is rejected ends the text: nothing of it commits, later items do
not run, and the error names the item's ordinal. Text that writes
returns no rows until it commits, and outside an interactive
transaction the engine reruns it on serialization failure as the
server does ([wire.md](wire.md#64-server-retry)); text that only reads
streams its rows. A temp relation lives in the interactive transaction
or the one text that declares it, and is gone at its end. Constraints
hold for ad hoc writes as for any other. Parameters are bound by name, so secrets such as a password
in `alter role web password p` never appear in the text. Results carry
their type, since no code was generated for them.

```rust
impl Database {
    pub fn query_text(&self, text: &str, params: &[(&str, Value)], opts: &CallOptions) -> Result<Vec<DynRows>, adtb::Error>;
}

pub struct DynRows { /* Iterator<Item = Result<Value, adtb::Error>> */ }
impl DynRows {
    pub fn ty(&self) -> &Type;
}
```

`Value` and `Type` are the binding's representation of any value and
any type of [model.md](model.md); `Type` carries the same information
as the wire's type descriptor ([wire.md](wire.md#72-resulttype)).

Rulings: surface/adhoc/scoped

## 7. Cancellation

A cancellation token is created by the program, cloned freely and
passed in the options of any number of calls. Cancelling it, from any
thread, stops every call carrying it promptly, without letting the call run
to the end of its statement; each returns `Cancelled`. A token once cancelled stays cancelled, and a call
started with a cancelled token returns `Cancelled` without running. A
cancelled call has no effect; inside an interactive transaction the
transaction stays open without the call's effects, as on the wire
([wire.md](wire.md#57-cancel)).

Dropping a result stream cancels its call. Dropping a transaction ends
it and rolls it back. Ending another session's work is an operational
command ([catalog.md](catalog.md#9-operational-commands)).

```rust
let tok = CancelToken::new();
let opts = CallOptions { cancel: Some(tok.clone()), ..CallOptions::default() };
let rows = app.call(queries::login_key, (u,), &opts)?;   // tok.cancel() from another thread

impl CancelToken {
    pub fn new() -> CancelToken;
    pub fn cancel(&self);
    pub fn is_cancelled(&self) -> bool;
}
```

Rulings: ops/cancel/token, ops/cancel/cancel, ops/cancel/terminate

## 8. Configuration

### 8.1 Open-time settings

Settings that must agree with the host and the process are given to
open and fixed until the handle closes: the page pool size, query
memory, worker threads, the directory for spill files, the key source,
the log archive target (§12.1), host functions (§10) and the metrics
callback (§13). Every resource size has a
default derived from the host, and the program may override it. Open-time
settings are shown read-only in `sys::process_options`; no write can
change them.

```rust
pub struct OpenOptions {
    pub create: Option<CreateOptions>,
    pub pool_bytes: Option<u64>,
    pub query_memory_bytes: Option<u64>,
    pub workers: Option<usize>,
    pub temp_dir: Option<PathBuf>,
    pub key: Option<KeySource>,
    pub archive: Option<ArchiveTarget>,
    pub host_fns: HostFns,
    pub on_metrics: Option<MetricsCallback>,
}

pub struct CreateOptions { pub encrypted: bool }

pub enum KeySource {
    Passphrase(String),
    Kms { uri: String },
    Bytes([u8; 32]),
}
```

By default the page pool is 25% of physical memory and there is one
worker per core.

### 8.2 Other scopes

Format settings are fixed when the file is created and change only by
copying into a new file ([catalog.md](catalog.md#10-configuration)).
File settings are rows of `catalog::settings`, changed by the command
`set setting`, which needs `configure`, and moved with the file
([catalog.md](catalog.md#10-configuration)). Per-call and per-transaction settings are the call and
transaction options (§4.1, §5.1).

Rulings: ops/config/scope/open, ops/config/scope/format, ops/config/scope/file, ops/config/scope/session, ops/config/home/process/split, ops/config/derived

## 9. Bulk append

### 9.1 Loads

A load streams Arrow record batches from the program into constructor
relations and commits them as one change set. The program opens a
load, obtains an appender for each constructor relation it fills, pushes
batches, and commits:

```rust
let mut load = db.load(&LoadOptions::default())?;
let mut google = load.appender(schema::identities::Google)?;
for batch in reader {
    google.push(&batch?)?;
}
let report = load.commit()?;
```

A batch pushed to an appender must have exactly the Arrow schema of
the constructor relation's row type, as the wire's Arrow format maps it
([wire.md](wire.md#54-arrow-format)): the same fields in the same order
with the same types, nullable only for `Option` fields. A field filled
by a key generator or a relation default may be left out, and is then
filled as an insert fills it. A batch with any other schema is refused
at push with `SchemaMismatch` and adds nothing. Text is normalized to
NFC and floats canonicalized as construction does
([model.md](model.md#1-values-and-types)), in process and on the wire
alike.

### 9.2 Commit

Pushed rows land in staging unconstrained. Commit inserts every staged
row of every appender of the load as one change set, checked
set-at-a-time like any change set: uniqueness, references and every
constraint ([interop.md](interop.md#4-loading)). If any check fails the
load aborts with the violation report and nothing is inserted. Two
staged rows with one key fail with `KeyViolation`, reported at commit;
a key collision with, or a wait on, a row committed after the load's
snapshot fails with `Serialization`
([transactions.md](transactions.md#63-waiting-on-a-key)). A load
dropped without commit discards its staging. A load is a write
transaction: it commits under serializable isolation, may fail with a
serialization error, and is not retried by the engine.

```rust
impl Database {
    pub fn load(&self, opts: &LoadOptions) -> Result<Load, adtb::Error>;
}

impl Load {
    pub fn appender<R: CtorRelation>(&mut self, r: R) -> Result<Appender<'_>, adtb::Error>;
    pub fn commit(self) -> Result<LoadReport, adtb::Error>;
}

impl Appender<'_> {
    pub fn push(&mut self, batch: &arrow::record_batch::RecordBatch) -> Result<(), adtb::Error>;
}

pub struct LoadOptions { pub durability: Durability, pub cancel: Option<CancelToken> }
pub struct LoadReport { pub rows: u64, pub position: Position }
```

A load into one constructor relation has a wire form, an Append stream
([wire.md](wire.md#73-append)). A load that fills several relations as
one change set has no single wire equivalent; over the server, the
client begins an interactive transaction, appends into temp relations
declared in it, moves the rows with one ad hoc `transact` block that
inserts from them, and commits ([wire.md](wire.md#73-append)).

Rulings: interop/load/append-api

## 10. Host functions

### 10.1 Binding

A schema function may be declared with a host body: the schema states
its signature, the name and version the body is registered under, and
whether it is deterministic, and the embedding
program supplies the body at open
([interop.md](interop.md#9-user-defined-functions)).

```adt
-- 0018
define fn slugify (t: Text) -> Text = host "slugify" version "1.0" deterministic
```

A host function is called with a batch: one Arrow array per parameter,
all of one length, and must return one array of that length of the
result type, mapped as the wire's Arrow format maps types
([wire.md](wire.md#54-arrow-format)). It may be called from several
worker threads at once. Returning an error, or panicking, is a fault of
the statement evaluating it ([queries.md](queries.md#15-faults)).

```rust
let mut fns = HostFns::new();
fns.register("slugify", "1.0", Determinism::Deterministic, |args: &[ArrayRef]| -> Result<ArrayRef, HostError> {
    let t = args[0].as_string::<i32>();
    Ok(Arc::new(t.iter().map(|s| s.map(slug::slugify)).collect::<StringArray>()))
});
let db = adtb::open("app.adt", &schema::LOG, &[&queries::BUNDLE],
                     OpenOptions { host_fns: fns, ..OpenOptions::default() })?;

impl HostFns {
    pub fn new() -> HostFns;
    pub fn register(
        &mut self,
        name: &str,
        version: &str,
        det: Determinism,
        f: impl Fn(&[ArrayRef]) -> Result<ArrayRef, HostError> + Send + Sync + 'static,
    );
}

pub enum Determinism { Deterministic, Volatile }
```

The version string the program registers must equal the version the
schema declares. The declared version is the function's code identity
([interop.md](interop.md#95-function-identity)), and the catalog
records it. Opening with a different version for a name the catalog
already binds fails
with `HostFnMismatch`, unless the shipped migrations contain a
`redefine fn` for that function, which open applies in step 8; open
never writes such an operator itself. Open still fails with
`HostFnMismatch`, naming the function, when a staged job or index
build of a migration before that `redefine fn` reaches it; otherwise
the supplied body is bound for the whole open. A host function has one code at
a time: the `redefine fn` is refused while a live bundle pins a
version whose code identity for the function differs
([evolution.md](evolution.md#7-version-translation)). Host functions
are bound before migrations run, so staged jobs and planning can call
them.

### 10.2 Missing and mismatched functions

At open, a supplied function whose name the schema declares with a
host body is checked against the declaration: the same determinism,
argument and result types the binding can map, and the version the
catalog records (§10.1). A mismatch fails open with `HostFnMismatch`. A declared host function that is not
supplied fails open with `HostFnMissing` when any index, constraint,
stored view or key generator reaches it; otherwise open succeeds and a
call that reaches it fails. Only a deterministic host function may be
reached by an index, a constraint, a stored view or a key
([interop.md](interop.md#9-user-defined-functions)).

A server process serves only the host functions its own program
supplies: a file whose indexes or constraints reach host functions is
served by a server built from the library with those functions.

Rulings: interop/udf/host-fn

## 11. Parity with the server

Every message of the server protocol is an operation of this API
serialized, with the same meaning
([wire.md](wire.md#14-parity-with-the-in-process-api)). Some
operations exist only in process, because they need the program's
memory or the file itself:

| Operation | Why only in process |
| --- | --- |
| Open, with migration and registration (§2) | Opening the file is what makes a process the owner |
| Host functions (§10) | Bodies are the program's code |
| Cancellation tokens (§7) | The in-process form of a Cancel message |
| Metrics callback (§13) | Counters go to the program's telemetry |
| Open-time settings (§8.1) | They belong to the process |

A bulk append into one relation has a wire form, the Append stream
([wire.md](wire.md#73-append)). Operations that exist only on the wire
are authentication, credit, compression, Ping, and request numbers for
idempotent writes: in process no reply is lost, so a call that returns
an error has not committed.

Rulings: net/parity/mirror

## 12. Log shipping and embedded replicas

### 12.1 Log shipping

With an archive target among the open-time settings, the engine ships
the redo log continuously, and base copies periodically, to the
target. The archive holds pages and log as stored, encrypted when the
file is ([security.md](security.md#95-backups-and-dumps)). A database is restored
from an archive to any commit position or schema version it covers
([interop.md §7.2](interop.md#72-point-in-time-restore)).

```rust
let db = adtb::open("app.adt", &schema::LOG, &[&queries::BUNDLE],
                     OpenOptions { archive: Some(ArchiveTarget::path("backups/app")?), ..OpenOptions::default() })?;

pub fn restore(archive: &ArchiveTarget, path: impl AsRef<Path>, to: RestorePoint, options: OpenOptions)
    -> Result<(), adtb::Error>;

pub enum RestorePoint { Latest, Position(Position), Version(u64) }
```

### 12.2 Embedded replicas

An embedded replica is a local file kept as a physical replica of a
server primary ([replication.md](replication.md)). The program opens
it with the primary's address and its bundles; the handle offers the
same operations as an opened file, with these differences:

- Reads run locally, at the newest safe point the replica has applied.
- Every registered transaction call, every interactive transaction that
  is not read-only, every ad hoc text that writes and every load into
  one relation is forwarded to the primary over the protocol and runs
  there; its result and commit position come back from the primary.
  Ad hoc writes forward as `CallText`, a load as an Append stream. A
  load that fills several relations is refused with
  `ReadOnlyReplica`.
- The replica forwards writes under its own client id with request
  numbers, and resends a write until it learns the outcome
  ([wire.md](wire.md#65-idempotent-writes)), so a forwarded call that
  returns an error has not committed.
- After a forwarded commit, later calls on the handle read no snapshot
  older than that commit: the handle passes the position as `after`
  and the replica waits until it has applied it, for at most the file
  setting `ReplicaWait`, after which the call fails with
  `SafePointTimeout`.
- Bundles must already be registered on the primary; the replica
  applies no operators of its own. Lease renewals of the replica's
  bundles are forwarded to the primary and retried until it
  acknowledges, so a partition delays a renewal but does not lose it
  ([replication.md §3](replication.md#3-schema-leases-and-upgrades-at-replicas));
  leases expire as [applications.md §4.2](applications.md#42-leases)
  states.
- `sync` waits until the replica has applied every commit the primary
  had when `sync` was called.

```rust
let db = adtb::replica("app.adt", "adt://primary.internal:7433/app", &[&queries::BUNDLE], ReplicaOptions::default())?;
db.sync()?;

pub fn replica(path: impl AsRef<Path>, primary: &str, bundles: &[&dyn AnyBundleDef], options: ReplicaOptions)
    -> Result<Database, OpenError>;

impl Database {
    pub fn sync(&self) -> Result<Position, adtb::Error>;
}
```

`ReplicaOptions` carries the open-time settings of §8.1 and the
credentials the replica authenticates to the primary with
([security.md](security.md#2-authentication)).

Rulings: repl/embedded/ship-log, repl/embedded/embedded-replica, repl/embedded/local-writes/forward

## 13. Metrics

The handle exposes the engine's counters to the program, as a snapshot
on demand and through a callback invoked at an interval set at open.
The counters are those of the statistics relations
([catalog.md](catalog.md#5-statistics)), per constructor relation.

```rust
impl Database {
    pub fn metrics(&self) -> Metrics;
}

pub struct MetricsCallback { pub interval: Duration, pub f: Box<dyn Fn(&Metrics) + Send + Sync> }
```

Rulings: ops/expose/callback
