# Replication

This part defines what a client and an operator observe of replicas:
what a replica serves, how reads on it relate to commits on the
primary, what happens to writes sent to it, promotion and fencing,
leases, upgrades and schema changes at replicas, and the embedded
replica. Durability levels and the loss window of a failover belong to
[transactions.md §7](transactions.md#7-durability-levels); change
feeds to [wire.md §9](wire.md#9-feeds) and
[interop.md §5.2](interop.md#52-change-streams); backups and
point-in-time restore to
[interop.md §7](interop.md#7-backup-and-point-in-time-restore). The
protocol between a primary and its replicas is out of scope of this
edition.

One database is one node's serial history: it is never partitioned
across nodes. It scales by replicas and by many databases.

Rulings: repl/distribution/never, repl/unit/physical

## 1. Replicas

### 1.1 Physical replicas

A replica is a copy of one primary that follows the primary's log. It
holds the same constructor ids, key values, catalog, settings and
schema versions as the primary, and its stored pages are identical to
the primary's. After a compaction
of the primary, a replica starts again from a new base copy
([catalog.md §11.3](catalog.md#113-compaction)).

A replica serves read-only transactions only.

Rulings: repl/unit/physical

### 1.2 Reads on a replica

Every read-only transaction on a replica runs at a safe snapshot the
primary has marked in its log: a snapshot no concurrent read-write
transaction can make inconsistent
([transactions.md §6.4](transactions.md#64-read-only-transactions)).
It never fails with a serialization failure. Each read on a replica
reports the commit position of its snapshot and the snapshot's age.

A read-only transaction on a replica may carry a commit position `p`
([transactions.md §8](transactions.md#8-commit-positions)). It then
waits until the replica has applied the log up to `p` and has a safe
snapshot at or after `p`, and runs at that snapshot, so it sees every
transaction committed at or before `p`. The wait lasts at most the
setting `ReplicaWait`, or the call's deadline if that is sooner; when
it ends without a safe snapshot covering `p`, the read fails with
`Retry { reason: SafePointTimeout {..} }`, which it may retry.

Safe snapshots keep forming while short read-write transactions run,
bounded by the setting `SafePointDelay`: once no safe snapshot has
formed for `SafePointDelay`, one forms as soon as every read-write
transaction running then has ended, if they end within a further
`SafePointDelay`. A read-write transaction on the primary may start up
to `SafePointDelay` later than it is issued for this. While a
read-write transaction runs without bound, no safe snapshot may form.

A replica applies the log while its reads run. When applying it would
remove a row version that a running read's snapshot sees, the replica
waits for that read for at most the setting `ReplicaVacuumWait`, then
cancels the read with `Retry { reason: ReplayConflict {..} }`, which
it may retry. A read on a replica holds back the removal of dead
versions on the primary only for the constructor relations it reads.

Rulings: repl/standby-reads/safe-snapshot, repl/standby-reads/lag/force-point, repl/standby-reads/lag/report-age, repl/session/bookmark, repl/unit/replica-vacuum/feedback, spec/txn/safe-point/liveness

### 1.3 Writes at a replica

A server replica does not forward writes. A call that would write, and
a Begin without `read_only`, are refused with `Rejected { reason:
ReadOnlyReplica { primary } }`, which names the primary
([wire.md §6.6](wire.md#66-replicas)).

An embedded replica forwards writes to its primary (§4).

## 2. Failover

Promotion of a replica to primary is manual. Promoting a replica first
fences the old primary, then starts a new epoch of the history: the
epoch of every later commit position is greater than every epoch
before it. A commit that reaches the fenced old primary fails with
`Retry { reason: Fenced }`. A position from an old epoch past the
point where the new epoch began is refused with `Rejected { reason:
PositionNotOnHistory {..} }`.

The promotion reports the log position it starts from; which commits
it loses is stated by
[transactions.md §7.2](transactions.md#72-the-loss-window). The old
primary rejoins as a replica only after its pages are resynced to the
new history; its log past the promotion's position is never merged.

Rulings: repl/failover/manual, repl/failover/split-brain/epoch, repl/failover/split-brain/fence, repl/commit-ack/rpo/stated

## 3. Schema, leases and upgrades at replicas

A replica's safe snapshots hold the schema version and the data of one
point of the primary's history, so a replica never shows a row of a
constructor before the constructor's declaration
([evolution.md §9](evolution.md#9-replication-of-schema-changes)).

A client binding or calling at a replica renews its bundle's lease as
on the primary. The replica forwards each renewal to the primary,
retrying until the primary acknowledges it; a renewal applied twice
extends the lease once. How leases expire, at the primary and at
replicas, is in [applications.md §4.2](applications.md#42-leases).

Replicas upgrade to a new engine release before the primary: a primary
writes a new format only after every replica reports the new release
([catalog.md §12.2](catalog.md#122-upgrades-and-downgrade)).

Rulings: repl/schema/in-log, repl/schema/forward-leases, repl/upgrade/replicas-first

## 4. Embedded replicas and log shipping

The embedded library can ship its log and periodic base copies to
object storage, which is a continuous backup
([embedding.md §12.1](embedding.md#121-log-shipping)).

An application may hold an embedded replica of a server primary
([embedding.md §12.2](embedding.md#122-embedded-replicas)). Its reads
run locally at safe snapshots, as §1.2 states. Every read-write
transaction is forwarded to the primary and runs there: ad hoc writes
forward as `CallText`, and a load into one relation as an Append
([wire.md §7](wire.md#7-ad-hoc-text-and-appends)). A load into several
relations is refused at the replica with `Rejected { reason:
ReadOnlyReplica {..} }`. The replica forwards each write with a client
id and a request number and resends it until it learns the outcome
([transactions.md §9](transactions.md#9-idempotent-write-calls)), so a
lost acknowledgement neither loses nor repeats a commit, and a
forwarded call that returns an error has not committed.

Rulings: repl/embedded/ship-log, repl/embedded/embedded-replica, repl/embedded/local-writes/forward

## 5. Settings and errors

The file settings of [catalog.md §10.2](catalog.md#102-file-settings)
that govern replicas:

| Setting | Governs | Default |
| --- | --- | --- |
| `SafePointDelay` | how long safe snapshots may fail to form while short read-write transactions run, and how much later than issued a read-write transaction may start for them (§1.2) | `100ms` |
| `ReplicaWait` | the longest a replica read given a commit position waits (§1.2) | `5s` |
| `ReplicaVacuumWait` | the longest a replica's replay waits for a read before cancelling it (§1.2) | `30s` |
| `LeaseGrace` | how long after a lease's expiry the primary still treats it as live ([applications.md §4.2](applications.md#42-leases)) | `1h` |

The errors this part defines, values of the error sums of
[catalog.md §4](catalog.md#4-error-sums):

| Error | When |
| --- | --- |
| `Retry { reason: SafePointTimeout {..} }` | a replica read given a position found no safe snapshot covering it within its wait (§1.2) |
| `Retry { reason: ReplayConflict {..} }` | a replica read was cancelled so that replay could remove a version its snapshot sees (§1.2) |
| `Retry { reason: Fenced }` | a commit reached a primary fenced by a promotion (§2) |
| `Rejected { reason: ReadOnlyReplica {..} }` | a write reached a server replica, or a load into several relations reached an embedded replica (§1.3, §4) |
| `Rejected { reason: PositionNotOnHistory {..} }` | a position from an old epoch past the start of a new one (§2) |
