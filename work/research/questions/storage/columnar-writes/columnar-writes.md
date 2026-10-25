---
title: Dense columnar makes point lookup and update expensive
pick: any
raised-by: [storage/columnar]
options:
  delta-store:
    title: Row-oriented write buffer merged into columns later
  live:
    title: Live with it for analytic sums
    excludes: [storage/columnar-writes/delta-store]
---
Finding row n's fields needs the row-to-subcolumn offset, which
ClickHouse rebuilds in memory per granule. A retag shifts dense offsets.
Uniqueness and FKs still need a row-addressed index.

## storage/columnar-writes/delta-store

Recent writes and retags go to a row store; compaction rewrites dense
columns.

## storage/columnar-writes/live

The engine targets analytics; OLTP sums use another layout.
