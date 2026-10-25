---
title: Replication shapes the embedded core supports
pick: any
raised-by: [engine/embedded-server]
options:
  ship-log:
    title: The library ships its log and snapshots to object storage
    links:
      - {rel: prior-art, to: prior/repl-sqlite-family}
  embedded-replica:
    title: An application process holds a local replica of a server primary, writes forwarded
    links:
      - {rel: prior-art, to: prior/repl-sqlite-family}
  peer-group:
    title: Application processes embedding the core form a consensus group among themselves
    needs-one-of: [repl/failover/raft, repl/failover/vsr]
    links:
      - {rel: prior-art, to: prior/repl-sqlite-family}
---
The core is a library and the server is a client of it, so replication
can live in the library (every embedder gets it) or only in the
server.

## repl/embedded/ship-log

Litestream around SQLite, here built in: the core already owns its
log, so no long-running read transaction is needed to stop
checkpoints.

```
let db = adtdb::open("app.db").archive_to("s3://b/app")?;
```

## repl/embedded/embedded-replica

libSQL embedded replicas, D1 sessions. Reads are local at a safe
snapshot; writes go to the server. The application's registered
bundles run on the replica, so the replica forwards their leases.

```
let db = adtdb::replica("app.db", "adtdb://primary")?;
db.sync()?;
```

## repl/embedded/peer-group

dqlite (WAL frames through Raft in the application nodes) and rqlite.
Every embedder links the consensus code.
