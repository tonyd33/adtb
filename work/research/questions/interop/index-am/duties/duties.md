---
title: A pluggable index must meet the engine's storage obligations
pick: any
raised-by: [interop/index-am/am-trait, interop/index-am/framework]
options:
  engine-pages:
    title: Index pages are written only through the engine's page and redo API
    links:
      - {rel: prior-art, to: prior/interop-pg-indexam}
  whole-lock:
    title: An index kind without predicate locks takes a whole-index SIREAD lock per scan
    links:
      - {rel: prior-art, to: prior/interop-pg-indexam}
  rebuild-on-recover:
    title: An index kind may be unlogged and is rebuilt from the heaps after a crash
  live:
    title: Live with extension indexes outside recovery and serializability
    excludes: [interop/index-am/duties/engine-pages, interop/index-am/duties/whole-lock, interop/index-am/duties/rebuild-on-recover]
---
Settled duties: entries per version (`mvcc/version-tid`), redo-only
logging with full-page images (`layout/torn/fpw`), batched per-sum
vacuum (`union-index/vacuum/batched`), SSI range locks
(`mvcc/next-key`), deterministic simulation (`buffer/test/dst`).

## interop/index-am/duties/engine-pages

Postgres generic WAL: the extension modifies a page copy and the core
logs the delta (https://www.postgresql.org/docs/current/generic-wal.html).

## interop/index-am/duties/whole-lock

Postgres: without `ampredlocks`, a serializable scan locks the whole
index and conflicts with any concurrent insert
(https://www.postgresql.org/docs/current/index-locking.html).

## interop/index-am/duties/rebuild-on-recover

Unlogged; a crash marks it invalid and queries fall back to scans until
rebuilt. Replicas never have it.

## interop/index-am/duties/live

An extension index can return rows a snapshot should not see.
