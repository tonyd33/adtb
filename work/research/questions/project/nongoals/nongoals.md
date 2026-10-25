---
title: What the project says it will not do
pick: any
raised-by: [substrate/new-engine]
options:
  distributed:
    title: No sharding or distributed transactions
    excludes: [engine/distributed]
  sql-compat:
    title: No SQL dialect, no Postgres or SQLite wire compatibility
  multi-process:
    title: No several OS processes writing one database file
  columnar-store:
    title: No columnar storage format
    excludes: [storage/columnar, storage/sparse-columnar]
  search-indexes:
    title: No full-text, vector or spatial indexes in the first spec
---
A spec states its non-goals so that rulings elsewhere are not reopened
by each new use case. Several follow from rulings already taken; this
records them as promises to users.

## project/nongoals/distributed

One node, optionally with replicas. `engine/process` did not take
`engine/distributed`.

## project/nongoals/sql-compat

`query/plain-sql` is defeated; this also rules out a compatibility
front end and the Postgres wire protocol for existing drivers.

## project/nongoals/multi-process

SQLite lets several processes share one file through file locks. Here
one process owns the file; others reach it through the server.

```
adtb::open("app.db")   // second process: Err(Locked { owner_pid })
```

## project/nongoals/columnar-store

Per-constructor row heaps are the only format; analytic speed comes
from execution, not layout.

## project/nongoals/search-indexes

Indexes are B+-trees and equality hashes over constructor sets
(`btree/hash/eq-unique`). Inverted, vector and R-tree indexes wait for
a later edition.
