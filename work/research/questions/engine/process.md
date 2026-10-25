---
title: Process model
pick: one
raised-by: [substrate/new-engine]
options:
  embedded:
    title: Library linked into the application
    links:
      - {rel: prior-art, to: prior/engine-embedded-dbs}
  server:
    title: Server process, clients connect over a protocol
  embedded-server:
    title: Embeddable core with an optional server wrapping it
  distributed:
    title: Clustered, data spread across nodes
    links:
      - {rel: prior-art, to: prior/engine-distributed-sql}
---
Decides who sees registered queries, how many processes write, and
whether retries cross a network.

## engine/embedded

SQLite, DuckDB, LMDB. One process writes; the schema and registered
queries are known to whoever opens the file.

```
let db = adtdb::open("app.db")?;
db.transaction(|t| t.run(queries::add_google, (u, g)))?;
```

## engine/server

Postgres shape. Registered queries arrive over the wire; many writers.

## engine/embedded-server

DuckDB with its remote protocol, libSQL over SQLite: the server is a
client of the library.

## engine/distributed

CockroachDB, TiDB, FoundationDB. A union index spans nodes; TiDB's
global index is the distributed precedent.
