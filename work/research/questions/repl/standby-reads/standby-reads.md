---
title: What a transaction reading on a replica is promised
pick: one
raised-by: [repl/unit/physical, repl/unit/change-set]
options:
  safe-snapshot:
    title: The primary marks safe snapshots in the log; replica readers run only on them
    needs: [engine/retry/safe-snapshot]
    links:
      - {rel: prior-art, to: prior/repl-ssi-standby}
  ordered-commit:
    title: The primary aborts whatever would make commit order differ from serial order
    assessment: {grade: costly, unverified: true, why: aborts writers to spare replica readers}
  primary-only:
    title: Replicas serve no reads; they exist for failover and backup
  snapshot:
    title: Replica reads run at snapshot isolation, named as such
    excludes: [engine/serializable-only]
---
Applying commits in commit order yields states a serial execution
never passed through: a read-only transaction can see T2's write
without T1's, while T1 is serialised before T2. One isolation level is
settled, so a replica reader is serializable or there are no replica
readers.

## repl/standby-reads/safe-snapshot

Grittner's 2011 proposal and Munro's withdrawn patch: when the
primary finds a snapshot with no concurrent read-write transaction
that could make it unsafe, it logs a marker. A replica read-only
transaction uses the latest marked snapshot or waits for the next. It
never aborts and takes no SIREAD locks, the same contract
`engine/retry/safe-snapshot` gives on the primary.

```
on replica r1:
transact read_only
  return count(i for i in identities)
-- runs at the latest safe-snapshot marker r1 has applied
```

## repl/standby-reads/ordered-commit

Every commit point is then a safe snapshot, so replicas read any
applied position. Costs extra aborts on the primary (unverified how
many).

## repl/standby-reads/primary-only

Postgres's current answer for serializable. Replicas still back
failover, backups and CDC.

## repl/standby-reads/snapshot

A second isolation level, ruled out by `engine/serializable-only`.
