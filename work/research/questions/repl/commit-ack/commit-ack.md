---
title: What durability a commit acknowledgement means
pick: some
raised-by: [repl/unit/physical, repl/unit/change-set, repl/unit/command]
options:
  local:
    title: Durable in the local log; replicas follow asynchronously
  quorum:
    title: Durable in the logs of a quorum of nodes
  applied:
    title: Durable on a quorum and applied on named replicas, visible to their readers
---
The spec states a recovery point per level: what an acknowledged
commit survives. Group commit already has one leader flushing the log
for all waiters; with replication that leader also waits for the
replicas' acknowledgement of the batch, so one network round trip is
shared by the batch. Levels may be chosen per transaction, as
Postgres `synchronous_commit` is.

## repl/commit-ack/local

Survives the primary's crash, not loss of its disk. Litestream,
LiteFS, D1 read replicas, Postgres async streaming.

```
transact durability local
  ...
```

## repl/commit-ack/quorum

Postgres `synchronous_commit = on` with quorum `synchronous_standby_names`
(unverified for the quorum form); Raft and VSR commit. Survives loss of
any minority.

## repl/commit-ack/applied

Postgres `remote_apply`; much larger commit delay since it waits for
replay. With `repl/standby-reads/safe-snapshot` the replica must also
have a safe snapshot covering the commit, which may wait further.
