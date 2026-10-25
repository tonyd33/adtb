---
title: What the engine's storage sits on
pick: one
raised-by: [substrate/new-engine]
options:
  own-pager:
    title: Own pager, heap files, B-trees and log
  lsm-kv:
    title: An embedded ordered LSM key-value store
    links:
      - {rel: prior-art, to: prior/engine-embedded-kv}
  cow-kv:
    title: An embedded copy-on-write B+tree key-value store
    links:
      - {rel: prior-art, to: prior/engine-embedded-kv}
  fdb:
    title: FoundationDB as a distributed transactional key-value store
    links:
      - {rel: prior-art, to: prior/engine-fdb-record-layer}
      - {rel: prior-art, to: prior/surface-fdb-record-layer}
---
The new-engine ruling chose a new logical engine over a Postgres or
DuckDB layer: sums, constraints, planner and language are ours. Whether
bytes reach disk through our own pager or through a storage library is
a separate choice. A library supplies ordered keys, snapshots and a log;
the layout of constructor rows and union indexes over those keys is
still the engine's.

## engine/own-pager

Postgres or SQLite shape: slotted heap pages per constructor, B-tree
union indexes holding `(key, ctor, tid)`, a WAL, a buffer pool. The
settled heap, TID and vacuum vocabulary applies literally.

## engine/lsm-kv

RocksDB, Pebble or fjall underneath. A row is a key-value pair; MVCC
from the store's sequence numbers; CockroachDB and TiDB (TiKV) store
relational data this way.

```
(heap identities, ctor Google, pk 7)    -> { user: 3, gid: "x" }
(index identity_pk, key 7)               -> (ctor Google, pk 7)
```

## engine/cow-kv

LMDB or redb: one writer, snapshot readers, no log. Fits
single-writer concurrency directly.

## engine/fdb

FoundationDB with our own layer in place of the Record Layer, which
already stores several record types in one extent with indexes across
them. Brings strict serializability, the five-second limit, and a
cluster to run.
