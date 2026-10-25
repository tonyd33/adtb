---
title: Physical storage of a relation-level sum
pick: one
raised-by: [substrate/new-engine]
options:
  table-per-ctor:
    title: One heap per constructor
  wide-row:
    title: One heap, rows sized to their constructor
    links:
      - {rel: prior-art, to: prior/oracle-substitutable}
  columnar:
    title: Columnar with discriminator and dense subcolumns per constructor
    links:
      - {rel: prior-art, to: prior/clickhouse-variant}
      - {rel: prior-art, to: prior/arrow-union}
  sparse-columnar:
    title: Columnar with discriminator and full-length subcolumns
    links:
      - {rel: prior-art, to: prior/arrow-union}
      - {rel: prior-art, to: prior/duckdb-union}
  adaptive:
    title: Chosen per sum by workload
    links:
      - {rel: prior-art, to: prior/h2o}
---
How the engine stores constructor rows, independent of the logical
encoding.

## storage/table-per-ctor

Scans of one constructor read only its heap; scans of the sum union the
heaps. Natural target for a union index. Postgres inheritance stores
children this way and as a result has no unique index or FK spanning
parent and children.

## storage/wide-row

Variable-length rows carry a tag and only their constructor's fields.
One heap, one primary index.

## storage/columnar

A tag column plus, per constructor field, a column holding values only
for rows of that constructor, as Arrow dense unions and ClickHouse
`Variant` do.

## storage/sparse-columnar

A tag column plus, per constructor field, a column spanning every row,
null outside its constructor. Arrow sparse unions and DuckDB `UNION`
(a STRUCT with the tag first). No offset maps; vectorizes directly.

## storage/adaptive

The engine picks a layout per sum and can change it, as H2O adapts
row, column and column-group layouts to the workload.
