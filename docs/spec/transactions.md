# Transactions

This part defines how change sets are applied: the one isolation
level, the statements a transaction is made of, `transact` blocks,
registered and interactive transactions, aborts and retry, durability
levels, commit positions for reading on replicas, and idempotent write
calls. What a change set is belongs to [writes.md](writes.md); what
must hold at each statement boundary belongs to
[constraints.md](constraints.md). How the engine provides these
guarantees is described in [engine.md](engine.md), which is
informative.

## 1. Serializability

### 1.1 One level

Every transaction is serializable. The committed transactions of a
database have the effect of some serial order of them: every
transaction's reads, writes, constraint checks and result are those it
would have if it ran alone, in that order, against the state the
transactions before it left. There is no other isolation level and no
option that selects one.

The serial order is not promised to agree with the order in which
transactions began or committed. On the primary, a transaction that
begins after another's commit was acknowledged reads a state that
includes it. Across replicas, reading one's own writes is promised
only through commit positions (§8).

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
read on a replica (§8) wait, once, before they start. A write may wait
for another transaction writing the same row or inserting the same key
(§6.3). Instead of waiting, a conflict between reads and writes aborts
a transaction with a serialization failure (§6.1).

Rulings: engine/serializable-only, engine/ssi, engine/retag-fk/by-isolation, mvcc/retag-phantom/as-insert, constraint/concurrent/serial-checks, engine/write-skew/by-isolation

## 2. Statements

A statement is one change set applied as one multiple assignment
([writes.md §1](writes.md#1-change-sets)). Constraints hold at every
statement boundary and nowhere else is a state observable between two
of them.

| Kind | Statements | Committed by |
| --- | --- | --- |
| `transact` block (§3) | one | the end of the block |
| registered transaction (§4) | one | the end of its block |
| interactive transaction (§5) | one per call | the client's commit |
| ad hoc text of several items | one per item | the end of the text |

Several items in one ad hoc text form one transaction, each item one
statement, and their results are returned in order
([syntax.md](syntax.md)). The items of a `transact` block are not
statements; the block is one.

Every statement of a transaction reads one snapshot of the database
taken when the transaction began, together with the change sets of the
transaction's own earlier statements. Within a statement, every read
and every write reads the statement's pre-state.

Rulings: write/unit/multi, constraint/timing/multi-assign, write/surface/txdata

## 3. Transact blocks

### 3.1 Form

```
transact
  let u = insert users { name: n }
  let i = insert identities.Google { user: key u, gid: g }
  return key i
```

A `transact` block is a sequence of items:

- `let x = e`, where `e` is a query or a write expression; `x` names
  the query's result or the rows the write would write
  ([writes.md §2.2](writes.md#22-what-an-insert-returns));
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
against every constraint and applied as one statement. If any
constraint is violated, or any item faults, nothing is applied and the
block fails with that error. The result reaches the caller only after
the block's transaction commits.

### 3.3 Options

Options follow the keyword:

| Option | Meaning |
| --- | --- |
| `retry n` | rerun the block up to `n` times on a serialization failure (§6.2) |
| `read_only` | the block may not write; a write in it is a type error |
| `read_only deferrable` | as `read_only`, and the block waits for a safe snapshot and then never fails with a serialization failure (§6.4) |
| `durability local`, `durability quorum` | the durability its commit acknowledgement promises (§7) |

```
transact read_only deferrable
  return count(i for i in identities)

transact retry 5 durability quorum
  let i = insert identities.Google { user: u, gid: g }
  return key i
```

Rulings: write/transact/block, write/insert/returns/rows, engine/retry/retry-block, engine/retry/safe-snapshot, repl/commit-ack/local, repl/commit-ack/quorum

## 4. Registered transactions

```
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
block sets the limit for that transaction; otherwise the configured
limit applies ([catalog.md](catalog.md#10-configuration)).

A `transact` block submitted as a single call is rerun by the server
in the same way. Such a call is an ad hoc call (`CallText` on the wire,
[wire.md](wire.md)) by a principal holding `adhoc` and the write
privileges its statements need, or an in-process call
([embedding.md](embedding.md)). Constraints hold for it as for any
statement.

A call carries an optional statement limit, deadline and priority
within its bundle's class ([wire.md](wire.md)).

Rulings: write/transact/registered-txn, net/retry/server, engine/retry/retry-block

## 5. Interactive transactions

### 5.1 Form

An interactive transaction spans several calls on the client's side:

```
let t = db.begin()
let u = t.call(find_user, email)
t.call(add_identity, key u, g)
t.commit()
```

The client begins a transaction, makes calls of registered queries and
registered transactions within it, then commits or aborts it. The
transaction is named by an id the begin returns, and every call carries
it; no other state is held for it on a connection
([wire.md](wire.md), [embedding.md](embedding.md)).

### 5.2 Meaning

Each call is one statement (§2). A call reads the transaction's
snapshot together with the change sets of its earlier calls.
Constraints hold after every call. The calls' change sets become
visible to other transactions only when the commit succeeds; an abort,
or a failed commit, discards all of them.

A call that fails, by a constraint violation, a fault or a user `fail`,
has no effect; the transaction stays open with the effects of its
earlier calls. A serialization failure aborts the whole transaction.

A registered transaction called within an interactive transaction is
one statement of it. Its `transact` block does not commit on its own:
its change set becomes visible with the interactive transaction's
commit, and the server does not rerun it on a serialization failure,
which aborts the interactive transaction (§5.5).

Constraints are checked after every call, never deferred to the
commit, so no call may leave a constraint violated for a later call to
repair. A change that passes through such a state is made by one call
of a registered transaction:

```
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
that takes the sequence as a function and reruns it up to a limit
([embedding.md](embedding.md)).

Rulings: write/transact/interactive, net/interactive/time-limit, engine/retry/retry-block

## 6. Aborts, retry and waiting

### 6.1 Serialization failures

A transaction whose commit would leave no serial order equivalent to
the execution is aborted with a serialization failure. It may be
detected at any read, write or commit. A serialization failure is
always retryable: rerunning the same transaction at a new snapshot is
correct, and the client is expected to do so. The error names only the
caller's own transaction; the other transactions in the conflict are
recorded in a system relation readable with the monitoring privilege
([catalog.md](catalog.md#1-system-relations)).

Constraint violations, faults and user `fail` errors are not
retryable: rerunning against the same state gives the same error.

### 6.2 Retryable units

A `transact` block has no effect outside the database, so rerunning it
is always safe. A block with `retry n` is rerun up to `n` times on a
serialization failure by whatever runs it: the server for a call
(§4), the client library for a block run in process. A client library
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

A key violation (`KeyViolation`, [catalog.md](catalog.md#4-error-sums))
is raised only against rows in the snapshot or among the statement's
own inserted rows, and is reported at the statement boundary.

### 6.4 Read-only transactions

A transaction declared `read_only` may not write. One declared
`read_only deferrable` waits until it can take a safe snapshot, one no
concurrent read-write transaction can make inconsistent, then runs on
it. It never fails with a serialization failure and never causes
another transaction to fail with one. Read-only transactions on a
replica always run this way (§8).

Rulings: engine/retry/retry-block, engine/retry/safe-snapshot, engine/wait-inserter, engine/ssi

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

A transaction that names no level commits at `local`.

### 7.2 The loss window

Replicas follow the primary's log asynchronously at level `local`.
When a replica is promoted, the promotion reports the log position it
starts from. Every transaction committed on the old primary after that
position is lost: it is absent from the new primary's history, and the
old primary is fenced so that it accepts no further commits. Its log
past that position is a separate history, never merged.

At level `local`, the loss window of a failover is therefore every
commit acknowledged after the promoted replica's applied position. A
promotion starts from the most advanced log among a surviving majority
of the nodes, which holds every commit acknowledged at `quorum`. A
transaction acknowledged at `quorum` is therefore lost only when a
majority of the nodes is lost.

Rulings: repl/commit-ack/local, repl/commit-ack/quorum, repl/commit-ack/rpo/stated

## 8. Commit positions and replica reads

Every acknowledged commit of a read-write transaction returns its
commit position, a value of the prelude type `Position = { epoch: I64,
lsn: I64 }` ([prelude.md](prelude.md)): its place in the primary's log, where the epoch counts promotions (§7.2) and the log sequence
number is the place within the history of that epoch. Positions of one
history are totally ordered, and a transaction's position follows the
position of every transaction whose commit was acknowledged before it
began. A position that is not on the history a node serves, because it
names an epoch's tail lost at a promotion, is refused with `Rejected {
reason: PositionNotOnHistory {..} }`.

Replicas serve read-only transactions only, each run at a safe
snapshot the primary has marked in its log (§6.4). A read-only
transaction on a replica may carry a commit position `p`. It then waits
until the replica has applied the log up to `p` and has a safe snapshot
at or after `p`, and runs at that snapshot, so it sees every
transaction committed at or before `p`. A client that passes the
position of its own last commit reads its own writes on any replica.
The wait lasts at most the setting `ReplicaWait` (default 5 s), or the
call's deadline if that is sooner; when it ends without a safe snapshot
covering `p`, the read fails with `Retry { reason: SafePointTimeout {..} }`, which
it may retry. Safe snapshots may fail to form while long read-write
transactions run.

The primary marks a safe snapshot at least as often as the setting
`SafePointDelay` (default 100 ms) allows: when no safe snapshot has
formed for that long, it delays the start of new read-write
transactions until one forms, by at most `SafePointDelay`.

Rulings: repl/session/bookmark

## 9. Idempotent write calls

### 9.1 Request ids

A call that can commit a write may carry a request id: a call of a
registered transaction, a `transact` block submitted as one call, and
the commit of an interactive transaction. A request id is needed only
for idempotent retry. It is a client id together with a request
number; the numbers a client sends with one client id must increase.
A request id is carried by the call that commits, never by the begin
or the other calls of an interactive transaction.

### 9.2 Deduplication

When a transaction carrying a request id commits, the engine records
the id and the reply, with its result rows, in the same commit. Only
committed outcomes are recorded: an error, including a constraint
violation or a serialization failure, leaves no record. A later call
with a request id is handled in this order:

1. if a record of that request id exists, the call does not run again
   and returns the recorded reply with its rows;
2. otherwise, if a call with that request id is still running, it
   waits for it and returns its outcome;
3. otherwise, if its number is strictly below the client's low-water
   mark, it does not run and fails with `RequestExpired`; the
   low-water mark is one above the highest number of that client whose
   record has been purged at the end of the retention window;
4. otherwise it runs, as a first attempt or as a new attempt after one
   that ended without committing.

A client that loses its connection before a commit is acknowledged
resends the call with the same request id; the write is then applied
at most once, and the client learns its reply. Every attempt the server
makes for one call (§4) carries the call's request id, and only the
attempt that commits is recorded. Once a transaction has committed,
its reply stream is never cancelled for idleness: a client that stops
reading loses the stream, the commit stands, and a resend with the
request id returns the recorded rows.

The retention window is configuration, and the recorded ids and
low-water marks are a system relation
([catalog.md](catalog.md#1-system-relations)).

A call reusing a recorded request id with a different call or different
arguments is refused with `Rejected { reason: RequestReused }`.

Rulings: net/unknown/idempotency, net/retry/server
