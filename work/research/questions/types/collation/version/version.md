---
title: Collation tables change between ICU and UCA versions
pick: any
raised-by: [types/collation/sortkey-fn, types/collation/typed, types/collation/column]
options:
  versioned-name:
    title: A collation name includes its version; moving to a new one is a schema operator that rebuilds
  reindex-op:
    title: The catalog records each collation's version and an upgrade rebuilds dependent indexes
  live:
    title: Live with indexes whose order no longer matches the library
    excludes: [types/collation/version/versioned-name, types/collation/version/reindex-op]
    assessment: {grade: broken, why: a unique index can admit duplicates and miss rows once order changes}
---
UTS #10: DUCET weights change between versions. Postgres warns on a
recorded version mismatch and leaves REINDEX to the operator.

## types/collation/version/versioned-name

`collate(name, "de@icu-74")`. The engine ships the tables for each
version it supports, so a collation never changes under stored data;
a registered query names the version it means.

## types/collation/version/reindex-op

Postgres `ALTER COLLATION ... REFRESH VERSION`, but the engine runs the
rebuild as an online staged job (`engine/ddl-online-txn/staged-job`)
before taking the new library.

## types/collation/version/live

As Postgres without REINDEX.
