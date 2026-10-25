---
title: Native enforcement needs keys and indexes spanning constructors
pick: one
raised-by: [encoding/native]
options:
  union-index:
    title: Global index over every constructor's heap
    needs-one-of: [storage/table-per-ctor, storage/adaptive]
    links:
      - {rel: same-as, to: encoding/exclusive/union-index}
      - {rel: prior-art, to: prior/ch-index}
      - {rel: prior-art, to: prior/oracle-global-index}
      - {rel: prior-art, to: prior/pg-global-index}
      - {rel: prior-art, to: prior/tidb-global-index}
  single-heap:
    title: Sum stored as one relation, per-constructor partial indexes
    needs-one-of: [storage/wide-row, storage/columnar, storage/sparse-columnar]
    links:
      - {rel: prior-art, to: prior/oracle-substitutable}
  live:
    title: Live with probing every constructor's index on each write
    assessment: {grade: costly, why: write cost grows with the number of constructors}
---
Sum identity must be unique across constructors, and an FK to the sum
must find its target whatever the constructor. With one heap per
constructor no ordinary per-relation index covers both. With one heap
for the sum, a `unique` field of one constructor needs an index over
only that constructor's rows.

## encoding/native-cost/union-index

```
relation identities : Identity key id
-- key index: entries (id, ctor, tid) over heaps identities.EmailPass, identities.Google
```

One B-tree whose entries point into several heaps. Sum PK and FKs to the
sum probe it; constructor FKs check `ctor` too.

Closest precedent is the OODB class-hierarchy index, one B+-tree over a
class and its subclasses. Global indexes on partitioned tables (Oracle,
TiDB, Postgres proposals) are the same structure with partitions for
constructors.

The uniqueness check carries over from a single-heap B-tree if each TID
fetch is routed by the entry's constructor: conflicts are found through
the index alone, so an insert need not lock other constructors' heaps.
FK to a constructor probes the key and compares `ctor`; a key-share
lock goes on the tuple in the heap the entry names.

Entries name their heap by a stable constructor id resolved through the
catalog and never reused. Heap OIDs fail on drop and re-add of a
constructor and on OID reuse.

## encoding/native-cost/single-heap

The sum is one relation tagged per row. Sum PK and FKs to the sum are
ordinary. `unique` on `gid` is a partial index `WHERE ctor = Google`; a
constructor FK checks the tag of the found row. Postgres partial unique
indexes, SQL Server filtered indexes on sparse columns, and Oracle
substitutable tables are precedent. Retag is an in-place update.

## encoding/native-cost/live

On insert, look up `id` in each constructor's PK index; FKs to the sum
likewise. No new index structure.
