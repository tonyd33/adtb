---
title: Concurrency control mechanism
pick: one
raised-by: [substrate/new-engine]
options:
  ssi:
    title: MVCC with serializable snapshot isolation
    links:
      - {rel: prior-art, to: prior/engine-pg-ssi}
  occ:
    title: MVCC with commit-time validation of reads and scans
    links:
      - {rel: prior-art, to: prior/engine-fdb}
      - {rel: prior-art, to: prior/engine-hekaton-calvin}
      - {rel: prior-art, to: prior/engine-embedded-dbs}
  s2pl:
    title: Strict two-phase locking with key-range locks
  single-writer:
    title: MVCC readers and one writer at a time
    links:
      - {rel: prior-art, to: prior/engine-embedded-dbs}
      - {rel: prior-art, to: prior/engine-embedded-kv}
  deterministic:
    title: Transactions ordered before execution
    links:
      - {rel: prior-art, to: prior/engine-hekaton-calvin}
---
How the engine delivers the promised isolation over per-constructor
heaps and union indexes. In every MVCC option a visibility check on an
index entry is routed by the entry's constructor id to that heap.

## engine/ssi

Snapshot reads plus SIREAD predicate locks; a dangerous structure of two
rw-antidependencies aborts a transaction (Cahill et al. SIGMOD 2008,
Ports and Grittner VLDB 2012). A probe of a ctor-set index takes a range
lock on that index, so it conflicts with an insert into any constructor
in the set and with none outside it. A sum scan locks each heap it
merges. A retag must carry the row's tuple-level SIREAD locks to the
new heap, as Postgres carries them along an update chain (unverified).

```
from i in identities.(Google | Facebook) where i.user == u select i.id
-- range lock on index (user) in (Google | Facebook)
```

## engine/occ

Read set and scanned ranges recorded, validated at commit (Hekaton,
FoundationDB read conflict ranges, DuckDB for write-write only). Range
validation over a union index works as for SSI index locks.

## engine/s2pl

Shared and exclusive locks held to commit; phantoms blocked with
next-key locks on index ranges, as InnoDB SERIALIZABLE
(https://dev.mysql.com/doc/refman/8.0/en/innodb-next-key-locking.html, unverified).
Readers block writers.

## engine/single-writer

One write transaction at a time; readers on snapshots. Trivially
serializable if a reader whose snapshot is stale may not write
(SQLite WAL, LMDB, fjall single-writer mode).

## engine/deterministic

Calvin: a sequencer orders transactions, execution follows the order,
no aborts from conflicts. Read and write sets must be known before
execution.
