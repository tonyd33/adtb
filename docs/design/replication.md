# Replication

The primary provides its redo log ([storage.md §5](storage.md#5-the-log))
to physical replicas, embedded replicas, change consumers and backups.
What a client observes of replicas, durability levels and replica reads
is in [../spec/replication.md](../spec/replication.md) and
[../spec/transactions.md §7](../spec/transactions.md#7-durability-levels).

## 1. The log stream

A replica receives the redo log itself and runs recovery
([storage.md §7](storage.md#7-recovery)) continuously.

Invariant: a replica's pages, TIDs, xids and constructor ids are
identical to the primary's, so the primary's safe snapshot markers mean
the same on it, and a page of one copy can stand in for the same page
of the other.

The primary keeps log segments while replication, archiving or a
change consumer still needs them
([storage.md §6](storage.md#6-write-ahead-and-checkpoints)).

Schema operators and stage transitions travel in the log, so a replica
applies a constructor's declaration before any of its rows
([schema-change.md §3](schema-change.md#3-log-records)).

Compaction gives versions new TIDs and pages new ids
([storage.md §10.5](storage.md#105-compaction)), so a replica cannot
continue from a log written after compaction without a new base.

Rulings: repl/unit/physical, repl/schema/in-log

## 2. Safe snapshot markers

The primary logs a marker record for each safe snapshot it finds
([mvcc.md §8.3](mvcc.md#83-deferrable-read-only-transactions)): a
snapshot no read-write transaction can make unsafe.

A replica's read-only transactions run only at safe snapshot markers:
at the latest marker the replica has applied, or by waiting for the
next. They take no SIREAD locks and never abort with a serialization
failure.

The primary emits safe points periodically: when no marker has been
logged for a configured delay, it delays the start of new read-write
transactions until the running ones finish and a marker forms, by at
most that delay. While a read-write transaction runs without bound, no
marker may form at all. Each replica read reports the commit position
and the age of its snapshot.

A replica read given a commit position waits until the replica has
applied it and a safe snapshot covers it, for at most a configured
wait ([../spec/replication.md §1.2](../spec/replication.md#12-reads-on-a-replica)).

Rulings: repl/standby-reads/safe-snapshot, repl/standby-reads/lag/force-point, repl/standby-reads/lag/report-age, repl/session/bookmark

## 3. Commit positions

A commit returns its log position, a `Position = { epoch: I64, lsn:
I64 }` whose values fit the wire's u32 epoch and u64 LSN.

## 4. Vacuum feedback

Each replica reports to the primary, per heap, the older of the oldest
snapshot its readers hold and the latest safe snapshot marker it has
applied, so a replica with no readers still reports a bound. The
primary folds these into each heap's watermark
([mvcc.md §9](mvcc.md#9-the-watermark)). A registered query on a
replica holds back only the heaps it reads.

Replay that would remove a version a replica reader can see waits for
that reader up to a configured wait, then cancels the reader.

Rulings: repl/unit/replica-vacuum/feedback

## 5. Commit acknowledgement

Each transaction chooses its durability level
([../spec/transactions.md §7](../spec/transactions.md#7-durability-levels)).
Under the local level the primary acknowledges a commit once it is
durable in its own log. Under the quorum level it acknowledges once the
commit is durable in the logs of a quorum of nodes; the group commit
leader waits for replica acknowledgements of the batch
([storage.md §10.1](storage.md#101-group-commit)).

Promotion reports the position it starts from, which is the loss
window for local commits.

Rulings: repl/commit-ack/local, repl/commit-ack/quorum, repl/commit-ack/rpo/stated

## 6. Epochs and fencing

Promoting a replica first fences the old primary from its storage or
network, then starts a new epoch: the file header and every log block
the new primary writes carry it
([storage.md §5.3](storage.md#53-blocks-and-segments)).

Invariant: a node that sees a log block or a peer with a higher epoch
stops writing.

Rulings: repl/failover/manual, repl/failover/split-brain/epoch, repl/failover/split-brain/fence

## 7. Logical change records

A transaction that writes a constructor relation of at least one
registered feed, a bulk load included
([storage.md §9](storage.md#9-bulk-loads)), writes logical change
records to the redo log for those relations: the inserted and deleted
rows per constructor, ahead of its commit record. Relations of no
registered feed get no logical records, so a feed's history starts at
its registration. A retag appears as a delete from the old constructor
relation and an insert into the new one in the same change set.

Recovery and physical replicas ignore logical records; the change
stream is read from them.

Rulings: repl/cdc/change-stream, repl/cdc/registered-consumer, repl/cdc/retag/ctor-pair, repl/cdc/retention/bounded, repl/cdc/retention/resnapshot

## 8. Forwarding to the primary

The embedded library ships its log segments and periodic base copies
to object storage, which is a continuous backup (§9). An embedded
replica of a server primary runs reads locally at safe snapshots and
forwards every read-write transaction to the primary, where it runs.
The replica forwards each with a client id and request numbers and
resends it until it learns the outcome, so a lost acknowledgement
neither loses nor repeats a commit. Ad hoc writes forward as
`CallText`, and a load into one relation forwards as `Append`.

A client connecting at a replica renews its bundle's lease, which is a
catalog write; the replica forwards the renewal to the primary,
retrying until it is acknowledged. Renewals are idempotent, so a
renewal applied twice extends the lease once. A lease's expiry is
computed from the last renewal record in the log, on the primary and on
replicas alike.

Rulings: repl/embedded/ship-log, repl/embedded/embedded-replica, repl/embedded/local-writes/forward, repl/schema/forward-leases

## 9. Backups

A backup is a page copy of the data file taken while writers run, plus
the log from the redo LSN of a checkpoint taken at the copy's start to
its end position. Full-page images are logged throughout, whatever the
device guarantees ([storage.md §5.2](storage.md#52-full-page-images)),
so replay from that redo LSN repairs torn and inconsistent pages. An
incremental backup copies only pages whose LSN exceeds the previous
backup's start. Backups copy pages and log as stored, encrypted or not.

Every backup carries a manifest of page and segment checksums, checked
without restoring, and is proven by a test restore: open, recover, then
run `verify` with every structural, cross and logical check over the
result ([../spec/catalog.md §11.1](../spec/catalog.md#111-verification)).

With a continuous log archive, a restore replays a base to a log
position, to a wall-clock time through the commit time map
([mvcc.md §10.3](mvcc.md#103-history-heaps-and-reads-of-the-past)), or
to just before a schema operator.

Rulings: repl/backup/base-plus-log, repl/backup/incremental, repl/verify/manifest, repl/verify/restore-check, repl/pitr/log-archive, repl/pitr/op-log-point

## 10. Notes for the implementation

Non-binding.

A process writes a lease renewal for a bundle at most once per eighth
of the lease duration.
