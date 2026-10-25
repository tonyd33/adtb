---
title: Isolation the spec promises
pick: one
raised-by: [substrate/new-engine]
options:
  serializable-only:
    title: Every transaction is serializable, no weaker level
    links:
      - {rel: prior-art, to: prior/engine-embedded-dbs}
  serializable-default:
    title: Serializable by default, weaker levels by explicit opt-in
    links:
      - {rel: prior-art, to: prior/engine-distributed-sql}
  snapshot-default:
    title: Snapshot isolation by default, serializable opt-in
    links:
      - {rel: prior-art, to: prior/engine-distributed-sql}
  strict-serializable:
    title: Strict serializable for every transaction
    links:
      - {rel: prior-art, to: prior/engine-fdb}
---
The engine enforces declarative constraints (exclusivity, totality,
uniqueness across constructors, at-least-one). Whether they hold under
concurrency depends on what isolation the engine gives.

## engine/serializable-only

One level. SQLite is serializable by construction (one writer, stale
snapshots cannot write). No anomaly catalogue in the spec.

```
transact
  delete i in identities where i.id == 7
```

## engine/serializable-default

As CockroachDB: serializable unless the transaction says otherwise.

```
transact isolation read_committed ...
```

## engine/snapshot-default

As Postgres REPEATABLE READ or TiDB: snapshot reads, first-committer
wins on write-write conflict, write skew allowed. Serializable is opt-in.

## engine/strict-serializable

Serializable plus real-time order: a transaction that starts after
another commits sees it. FoundationDB promises this. On one node with
one commit sequencer it costs nothing over serializable (unverified).
