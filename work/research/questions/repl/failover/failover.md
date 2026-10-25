---
title: How a replica becomes the primary
pick: one
raised-by: [repl/unit/physical, repl/unit/change-set, repl/unit/command]
options:
  manual:
    title: An operator promotes a replica; nothing is automatic
    links:
      - {rel: prior-art, to: prior/repl-pg-physical}
  external-lease:
    title: The primary holds a lease in an external coordinator; a replica takes it on expiry
    links:
      - {rel: prior-art, to: prior/repl-sqlite-family}
  raft:
    title: The log is replicated by Raft; election is built in
    needs: [repl/commit-ack/quorum]
    links:
      - {rel: prior-art, to: prior/repl-raft}
  vsr:
    title: Viewstamped Replication with a primary per view
    needs: [repl/commit-ack/quorum]
    links:
      - {rel: prior-art, to: prior/repl-vsr}
  reconfigure:
    title: A Paxos control group recruits a new log and sequencer on any failure
    needs: [repl/commit-ack/quorum]
    links:
      - {rel: prior-art, to: prior/engine-fdb}
---
Fixes the recovery time the spec can state and who prevents two
primaries. An embedded core with a server around it means the
consensus machinery, if any, lives in the server, not the library.

## repl/failover/manual

Postgres `pg_promote()`, Litestream restore. Recovery time is an
operator's reaction time.

```
promote replica r1    -- returns the position r1 starts from
```

## repl/failover/external-lease

LiteFS with a Consul lease; Patroni with etcd (unverified). The
engine stays single-node; safety depends on the old primary stopping
writes before its lease ends, which assumes bounded clock drift.

## repl/failover/raft

The redo log is the Raft log; a commit is durable once a majority has
it. openraft is an async Rust library with joint-consensus membership,
read index and leader lease; dqlite and rqlite are the embedded
precedent. Survives any minority of node losses with recovery in an
election timeout.

## repl/failover/vsr

TigerBeetle: replication, view-change and nack quorums; protocol-aware
recovery tells a corrupt local entry from one never committed, so
`buffer/faults/repair` becomes possible.

## repl/failover/reconfigure

FoundationDB: no in-place recovery; a new transaction system is
recruited and the recovery version is the end of the replicated log.
Built for many nodes, far from an embedded core.
