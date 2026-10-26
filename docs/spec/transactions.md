# Transactions

This part defines how change sets are applied: the one isolation
level, the statements a transaction is made of, `transact` blocks,
registered and interactive transactions, aborts and retry, durability
levels, commit positions, and idempotent write calls. What a change
set is belongs to [writes.md](writes.md); what must hold at each
statement boundary belongs to [constraints.md](constraints.md); reads
and writes at replicas belong to [replication.md](replication.md).

## 1. Serializability

### 1.1 One level

Every transaction is serializable. The committed transactions of a
database have the effect of some serial order of them: every
transaction's reads, writes, constraint checks and result are those it
would have if it ran alone, in that order, against the state the
transactions before it left. There is no other isolation level and no
option that selects one.

The serial order extends real time on the primary: when `T`'s commit
was acknowledged before `T'` began, `T` precedes `T'`, so `T'` reads a
state that includes `T`. Beyond that, the serial order is not promised
to agree with the order in which transactions began or committed.
Replica reads keep the guarantee of
[replication.md §1.2](replication.md#12-reads-on-a-replica): across
replicas, reading one's own writes is promised only through commit
positions.

A transaction that aborts has no effect. Results a transaction returns
before it commits are promised to be consistent only if it commits
(§5.3).

### 1.2 What serializability covers

Serializability covers everything a transaction reads, including the
reads a constraint check makes ([constraints.md §8](constraints.md#8-concurrent-transactions))
and the reads of delete actions and reference checks. In particular:

- concurrent transactions cannot jointly violate a constraint each
  satisfies alone;
- a scan of a constructor relation `r.D` conflicts with a concurrent
  retag into `r.D` as it does with an insert into `r.D`, and with a
  retag out of a constructor relation it read as with a delete;
- a retag of a row and a concurrent insert of a reference to it typed
  by the row's old constructor do not both commit;
- a concurrent write of a row by another transaction is ordered with
  this one whether the row is updated in place or retagged.

Every statement also reads the schema version it runs at, and applying
a migration writes it, so a transaction that ran at version `n - 1` is
ordered against the migration that produced version `n` like any other
conflicting pair. A schema change is a transaction like any other,
applied with the data writes of the same migration or not at all
([evolution.md](evolution.md#1-the-operator-log)).

### 1.3 Readers never block

A read never waits for a write and a write never waits for a read.
Reads on the primary run as soon as they are issued and never wait for
a safe snapshot; only a `read_only deferrable` transaction (§6.4) and a
read on a replica ([replication.md](replication.md#12-reads-on-a-replica)) wait, once, before they start. A write may wait
for another transaction writing the same row or inserting the same key
(§6.3). Instead of waiting, a conflict between reads and writes aborts
a transaction with a serialization failure (§6.1).

Rulings: engine/serializable-only, engine/ssi, engine/retag-fk/by-isolation, mvcc/retag-phantom/as-insert, constraint/concurrent/serial-checks, engine/write-skew/by-isolation, spec/txn/real-time/primary

## 2. Statements

A statement is one change set applied as one multiple assignment
([writes.md §1](writes.md#1-change-sets)). Constraints hold at every
statement boundary and nowhere else is a state observable between two
of them.

| Kind | Statements | Committed by |
| --- | --- | --- |
| `transact` block (§3) | one | the end of the block |
| registered transaction (§4) | one | the end of its block |
| interactive transaction (§5) | one per call, or one per item of a call of ad hoc text | the client's commit |
| ad hoc text of several items | one per item | the end of the text |

Several items in one ad hoc text form one transaction, each item one
statement, and their results are returned in order
([syntax.md](syntax.md)). The first item that fails, by a fault, a
user `fail`, a constraint violation or a rejection, ends the text: the
effects of none of its items are applied, the later items do not run,
and the error names the failing item's ordinal. Within an interactive
transaction the same holds for one call: each item is a statement, and
when one fails, the effects of every item of that call are undone and
the interactive transaction stays open (§5.2). The items of a
`transact` block are not statements; the block is one.

Every statement of a transaction reads one snapshot of the database
taken when the transaction began, together with the change sets of the
transaction's own earlier statements. Within a statement, every read
and every write reads the statement's pre-state.

Rulings: write/unit/multi, constraint/timing/multi-assign, write/surface/txdata

## 3. Transact blocks

### 3.1 Form

```adt
-- given n: Text, g: Text
transact
  let u = insert users { name: n }
  let i = insert identities.Google { user: key u, gid: g }
  return key i
```

A `transact` block is a sequence of items:

- `let x = e`, where `e` is a query, an insert, an upsert, an update or
  a delete; `x` names the query's result or the rows the write binds
  ([writes.md §1.1](writes.md#11-queries-are-pure));
- a write expression, whose change set joins the block's;
- at most one `return e`, last, whose value is the block's result.

A block with no `return` applies its change set and returns nothing.

### 3.2 Meaning

The block computes one change set and one result from one snapshot.
Every query and write in the block reads the pre-state; none sees the
rows another write of the block inserts or deletes, except through a
name bound by `let`. So `key u` in the second line above is the
generated key of the user the first line inserts, while a query over
`users` in the block does not find that user.

When the last item is computed, the block's change set is checked
against every constraint and applied as one statement. If any item
aborts, nothing is applied and the block fails with one of those
aborts ([writes.md §1.5](writes.md#15-faults)); otherwise, if any
constraint is violated, nothing is applied and the block fails with
the violation of
[constraints.md §7](constraints.md#7-checking-and-violation-reports). The result reaches the caller only after
the block's transaction commits.

### 3.3 Options

Options follow the keyword:

| Option | Meaning |
| --- | --- |
| `retry n` | run the block at most `n + 1` times, the first attempt included, while attempts fail with a serialization failure (§6.2) |
| `read_only` | the block may not write; a write in it is a type error |
| `read_only deferrable` | as `read_only`, and the block waits for a safe snapshot and then never fails with a serialization failure (§6.4) |
| `durability local`, `durability quorum` | the durability its commit acknowledgement promises (§7) |

A block declaring both `read_only` and `durability` is a type error.

```adt
-- given u: Key users, g: Text
transact read_only deferrable
  return count(i for i in identities)

transact retry 5 durability quorum
  let i = insert identities.Google { user: u, gid: g }
  return key i
```

Rulings: write/transact/block, write/insert/returns/rows, engine/retry/retry-block, engine/retry/safe-snapshot, repl/commit-ack/local, repl/commit-ack/quorum, spec/txn/retry-count/n-plus-one, spec/txn/durability/strongest, spec/writes/let-write/four-forms, spec/writes/abort-choice/any-abort

## 4. Registered transactions

```adt
txn sign_up (n: Text, g: Text) -> Key identities =
  transact
    let u = insert users { name: n }
    let i = insert identities.Google { user: key u, gid: g }
    return key i
```

A registered transaction is a named `transact` block with declared
parameters and result type, registered in a bundle like a registered
query ([applications.md](applications.md#1-registered-queries-and-transactions)). A client
calls it by name with arguments, in one round trip, and receives its
result after its transaction commits.

A registered transaction has no effect outside the database. When an
attempt fails with a serialization failure, the server reruns it from
the beginning at a new snapshot, before replying, up to a limit. The
client sees one reply: the result of the attempt that committed, or the
serialization failure once the limit is reached. The reply reports the
number of attempts ([wire.md](wire.md)). A `retry n` option on the
block allows at most `n + 1` attempts for that transaction; otherwise
the file setting `RetryLimit` applies
([catalog.md](catalog.md#102-file-settings)).

Ad hoc text that writes, a `transact` block or several items
submitted as one call outside an interactive transaction, is rerun by
the server in the same way: it sends no result rows until its
transaction commits, and its reply reports the number of attempts. Ad
hoc text that only reads streams its results as it computes them. Such
a call is an ad hoc call (`CallText` on the wire, [wire.md](wire.md))
by a principal holding `adhoc` and the write privileges its statements
need, or an in-process call ([embedding.md](embedding.md)).
Constraints hold for it as for any statement.

A call carries an optional statement limit, deadline and priority
within its bundle's class ([wire.md](wire.md)).

Rulings: write/transact/registered-txn, net/retry/server, engine/retry/retry-block, spec/txn/retry-count/n-plus-one

## 5. Interactive transactions

### 5.1 Form

An interactive transaction spans several calls on the client's side:

```text
let t = db.begin()
let u = t.call(find_user, email)
t.call(add_identity, key u, g)
t.commit()
```

The client begins a transaction, makes calls of registered queries and
registered transactions within it, or calls of ad hoc text, then
commits or aborts it. The
transaction is named by an id the begin returns, and every call carries
it; no other state is held for it on a connection
([wire.md](wire.md), [embedding.md](embedding.md)).

### 5.2 Meaning

A call of a registered query or transaction is one statement (§2). A
call of ad hoc text is one statement per item. Each statement reads the
transaction's snapshot together with the change sets of the earlier
statements. Constraints hold after every statement. The change sets
become visible to other transactions only when the commit succeeds; an
abort, or a failed commit, discards all of them.

A call is atomic. A call that fails, by a constraint violation, a fault
or a user `fail` in any of its statements, applies none of its
statements' effects, and the transaction stays open with the effects
of its earlier calls. A serialization failure aborts the whole
transaction.

Every statement of a transaction runs at one schema version and one
catalog version: ad hoc text at the schema and catalog versions
current at the transaction's snapshot, a registered call at its
bundle's pinned version and pinned catalog version. A call that would run at another
schema version or catalog version than the transaction's earlier
statements is refused with `Rejected { reason: MixedVersions }`, and
the transaction stays open. In particular, ad hoc text in a transaction
that has made a registered call, and a registered call in one that has
run ad hoc text, are refused.

A registered transaction or a `transact` block run within an
interactive transaction is one statement of it, and never commits on
its own: its change set becomes visible with the interactive
transaction's commit, and the server does not rerun it on a
serialization failure, which aborts the interactive transaction
(§5.5). A block with `read_only deferrable` run within an interactive
transaction is refused.

The interactive transaction commits at the strongest durability level
(§7) named by its begin, by its commit, or by any block run in it.

Constraints are checked after every call, never deferred to the
commit, so no call may leave a constraint violated for a later call to
repair. A change that passes through such a state is made by one call
of a registered transaction:

```adt
txn join (n: Text, g: Text) -> Key users =
  transact
    let u = insert users { name: n }
    insert identities.Google { user: key u, gid: g }
    return key u
```

Under `has_identity`, an interactive transaction that inserts a user in
one call and its identity in a later call fails at the first call; one
that calls `join` succeeds.

### 5.3 Provisional results

The results of calls made before the commit are consistent with the
serial order only if the commit succeeds. A client must not act outside
the database on them before the commit is acknowledged.

### 5.4 Time limit

An interactive transaction open longer than the transaction time limit,
or idle between calls longer than the idle limit, is aborted by the
engine; its later calls and its commit fail with that error. The limits
are configuration ([catalog.md](catalog.md#10-configuration)); the
server sends the limits in force when a connection becomes ready, and
a begin may carry a shorter statement limit, deadline and priority
([wire.md](wire.md)).

### 5.5 Retry

The engine does not rerun an interactive transaction. On a
serialization failure the client reruns the whole sequence of calls,
from the begin, as one retryable unit. Client libraries provide a form
that takes the sequence as a function and reruns it up to a limit the
caller gives ([embedding.md](embedding.md#52-retrying-a-closure)).

Rulings: write/transact/interactive, net/interactive/time-limit, engine/retry/retry-block, spec/txn/call-atomic/item-statements, spec/txn/one-version/one-pair, spec/txn/durability/strongest

## 6. Aborts, retry and waiting

### 6.1 Serialization failures

A transaction may be aborted with a serialization failure at any
read, write or commit. One whose commit would leave no serial order of
the committed transactions (§1.1) is aborted; one may also be aborted
although a serial order including it exists. A `read_only deferrable`
transaction is the exception: it is never aborted with a serialization
failure (§6.4). A serialization failure is
always retryable: rerunning the same transaction at a new snapshot is
correct, and the client is expected to do so. The error names only the
caller's own transaction; the other transactions in the conflict are
recorded in a system relation readable with the monitoring privilege
([catalog.md](catalog.md#1-system-relations)).

Constraint violations, faults and user `fail` errors are not
retryable: rerunning against the same state gives the same error.

### 6.2 Retryable units

A `transact` block has no effect outside the database, so rerunning it
is always safe. A block with `retry n` is run at most `n + 1` times,
the first attempt included, while its attempts fail with a
serialization failure, by whatever runs it: the server for a call
(§4), the client library for a block run in process. A block without
the option is rerun up to the file setting `RetryLimit`
([catalog.md](catalog.md#102-file-settings)), in process as on the
server. A client library
reruns only closures run in its own process. A sequence of
calls in an interactive transaction is the client's retryable unit
(§5.5).

### 6.3 Waiting on a key

A write is serializable at its edges: it never acts on a row the
transaction's snapshot does not hold.

- An insert, an upsert or a retag whose new row has a key or unique
  value that a concurrent, uncommitted transaction has written waits
  for that transaction to end. If it aborts, the waiting write
  proceeds as if the row never existed. If it commits, the waiting
  transaction fails with a serialization failure.
- A new row whose key or unique value conflicts with a row committed
  after the transaction's snapshot was taken fails the transaction
  with a serialization failure, never with a key violation.
- A write of a row that a concurrent, uncommitted transaction has
  written waits for that transaction to end. If it aborts, the write
  proceeds; if it commits, whether by update, retag or delete, the
  waiting transaction fails with a serialization failure. A write never
  re-evaluates its statement on the other transaction's new version.

Writers that wait on each other in a cycle never all wait forever: one
transaction of the cycle fails with a serialization failure (§6.1), and
the others go on waiting or proceed. Which transaction of the cycle
fails is not specified.

A key violation (`KeyViolation`, [catalog.md](catalog.md#4-error-sums))
is raised only against rows in the snapshot or among the statement's
own inserted rows, and is reported at the statement boundary.

### 6.4 Read-only transactions

A transaction declared `read_only` may not write. One declared
`read_only deferrable` waits until it can take a safe snapshot, one no
concurrent read-write transaction can make inconsistent, then runs on
it. It never fails with a serialization failure and never causes
another transaction to fail with one. Read-only transactions on a
replica always run this way
([replication.md](replication.md#12-reads-on-a-replica)).

Rulings: engine/retry/retry-block, engine/retry/safe-snapshot, engine/wait-inserter, engine/ssi, spec/txn/may-abort/may, spec/txn/retry-count/n-plus-one, spec/writer-wait-cycle/fail-one

## 7. Durability levels

### 7.1 Levels

Each read-write transaction commits at one durability level, chosen by
the transaction. A commit acknowledgement means the transaction is
durable at its level:

| Level | Acknowledged when the commit is | Survives | May be lost by |
| --- | --- | --- | --- |
| `local` | durable in the primary's log on its own storage | a crash or power loss of the primary | loss of the primary's storage; failover to a replica that has not applied it |
| `quorum` | durable in the logs of a majority of the primary and its replicas | loss of any minority of those nodes | loss of a majority of them |

A read-only transaction writes nothing and has no level. A database
without replicas has a majority of one: `quorum` is then the same as
`local`.

A commit whose write or flush to storage fails is never acknowledged:
the engine stops and recovers instead of reporting an error
([embedding.md §2.4](embedding.md#24-engine-failures)).

A transaction that names no level commits at `local`. An interactive
transaction commits at the strongest level named by its begin, its
commit or any block run in it (§5.2).

### 7.2 The loss window

Replicas follow the primary's log asynchronously at level `local`.
When a replica is promoted
([replication.md §2](replication.md#2-failover)), the promotion
reports the log position it starts from. Every transaction committed on the old primary after that
position is lost: it is absent from the new primary's history, and the
old primary is fenced so that it accepts no further commits. Its log
past that position is a separate history, never merged.

At level `local`, the loss window of a failover is therefore every
commit acknowledged after the promoted replica's applied position. A
promotion starts from the most advanced log among a surviving majority
of the nodes, which holds every commit acknowledged at `quorum`. A
transaction acknowledged at `quorum` is therefore lost only when a
majority of the nodes is lost.

Rulings: repl/commit-ack/local, repl/commit-ack/quorum, repl/commit-ack/rpo/stated, spec/txn/durability/strongest, buffer/fsync-fail/panic-replay

## 8. Commit positions

Every acknowledged commit of a read-write transaction returns its
commit position, a value of the prelude type `Position = { epoch: I64,
lsn: I64 }` ([prelude.md](prelude.md)): its place in the primary's log, where the epoch counts promotions (§7.2) and the log sequence
number is the place within the history of that epoch. Positions of one
history are totally ordered, and a transaction's position follows the
position of every transaction whose commit was acknowledged before it
began. A position that is not on the history a node serves, because it
names an epoch's tail lost at a promotion, is refused with `Rejected {
reason: PositionNotOnHistory {..} }`.

A read given a commit position reads a state that includes every
transaction committed at or before it; on a replica it waits for that
state ([replication.md §1.2](replication.md#12-reads-on-a-replica)).
A client that passes the position of its own last commit reads its
own writes on any node.

Rulings: repl/session/bookmark

## 9. Idempotent write calls

### 9.1 Request ids

A call that can commit a write may carry a request id: a call of a
registered transaction, ad hoc text that writes and an append of rows
([interop.md](interop.md#4-loading)), each outside an interactive
transaction, and the commit of an interactive transaction. A request
id on ad hoc text that only reads is ignored. A request id is needed
only for idempotent retry. It is a client id together with a request
number; the numbers a client sends with one client id must increase.
So a request whose number equals a recorded one is a resend and gets
the recorded reply, and one whose number is below the client's
low-water mark is refused with `RequestExpired` (§9.2). A request id is carried by the call that commits, never by the begin
or the other calls of an interactive transaction.

### 9.2 Deduplication

Requests are deduplicated by their request id. A call carrying a
request id that has committed is never applied again: a resend of it
returns the original reply, its result rows included. A call that
ended with an error, a constraint violation or a serialization failure
included, did not commit, and a resend of it runs again. A call with a
request id is handled in this order:

1. if a call with that request id has committed, the call does not run
   again: when it is the same call with the same arguments, it returns
   the committed call's reply with its rows, and otherwise it is
   refused with `Rejected { reason: RequestReused }`;
2. otherwise, if a call with that request id is still running, it is
   refused with `Rejected { reason: RequestReused }` when that call is
   another call or has other arguments, and otherwise it waits for it
   and returns its outcome;
3. otherwise, if its number is strictly below the client's low-water
   mark, it does not run and fails with `RequestExpired`; the
   low-water mark is one above the highest number of that client whose
   committed reply is no longer returned, past the retention window;
4. otherwise it runs, as a first attempt or as a new attempt after one
   that ended without committing. A resent commit of an interactive
   transaction that never committed and no longer exists fails with
   `Rejected { reason: UnknownTransaction }`, which then means
   definitely that the transaction did not commit.

A client that loses its connection before a commit is acknowledged
resends the call with the same request id; the write is then applied
at most once, and the client learns its reply. The attempts the server
makes for one call (§4) are one request: only the attempt that commits
has a reply a resend returns. Once a transaction has committed, its
reply stream is never cancelled for idleness: a client that stops
reading loses the stream, the commit stands, and a resend with the
request id returns its rows.

The retention window is configuration, and the committed request ids
and the low-water marks are readable in a system relation
([catalog.md](catalog.md#1-system-relations)).

Rulings: net/unknown/idempotency, net/retry/server, spec/txn/resend/promise, spec/txn/resend-unknown/not-committed, spec/txn/request-order/increasing
