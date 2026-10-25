---
title: Format of intermediate batches between operators
pick: one
raised-by: [substrate/new-engine]
options:
  arrow:
    title: Arrow record batches, one schema per constructor
    links:
      - {rel: prior-art, to: prior/arrow-union}
      - {rel: prior-art, to: prior/exec-datafusion}
  own-vectors:
    title: Own vectors with encodings and selection vectors
    links:
      - {rel: prior-art, to: prior/exec-velox}
      - {rel: prior-art, to: prior/duckdb-union}
  tuples:
    title: Fixed-offset row tuples materialized only at pipeline breakers
    needs-one-of: [exec/model/compiled, exec/model/adaptive, exec/model/volcano]
    links:
      - {rel: prior-art, to: prior/exec-hyper}
---
Every batch out of a constructor scan has the constructor's schema; no
column needs a validity bitmap, since no field is NULL. A batch that
mixes constructors (after a union index probe or a merge) carries a tag
column; see `exec/interleaved`.

```
batch of identities.Google, 2048 rows
  gid     utf8 view  2048 x 16 B   (no validity buffer)
  domain  utf8 view  2048 x 16 B
  user    u64        2048 x 8 B
  sel     u32[k]     optional, k <= 2048
```

## exec/batch/arrow

Arrow allows omitting the validity bitmap when the null count is 0;
buffers 64-byte aligned. Interop with arrow-rs, DataFusion, Parquet
writers for free. Selection vectors are not part of Arrow; filters
materialize or carry a separate index array.

## exec/batch/own-vectors

DuckDB: 2048-row vectors, flat, constant, dictionary (child plus
selection) and sequence types behind a unified view. Velox: flat,
dictionary, constant, RLE, bias, lazy loading. Keeps selection vectors
first-class (X100 primitives all take one).

## exec/batch/tuples

HyPer: inside a pipeline values live in registers; at a hash table or
sort they are written as rows at offsets fixed by the plan. Per
constructor the row has one layout:

```
google row (32 B)
  0   u64   user
  8   u32   gid offset  | u32 gid len
  16  u32   domain off  | u32 domain len
  24  u64   hash / next
```
