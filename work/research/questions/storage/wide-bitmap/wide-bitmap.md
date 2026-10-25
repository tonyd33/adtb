---
title: A null bitmap spans every constructor's fields
pick: any
raised-by: [storage/wide-row]
options:
  per-ctor-format:
    title: Tag selects the row format, common prefix then constructor suffix
  live:
    title: Live with one bit per field of the whole sum
    excludes: [storage/wide-bitmap/per-ctor-format]
---
With Postgres-style tuples, every row carries a bitmap bit for every
field of every constructor, and the column limit (1600) counts the sum.
https://www.postgresql.org/docs/current/storage-page-layout.html

## storage/wide-bitmap/per-ctor-format

Shared fields at fixed offsets, then only the constructor's own fields
in its own layout, as Avro or protobuf records select by schema
(unverified as precedent in a heap engine).

## storage/wide-bitmap/live

About one bit per field per row.
