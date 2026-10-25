---
title: How result rows are encoded
pick: some
raised-by: [surface/wire/binary]
options:
  rows:
    title: Row-wise binary records in the signature's field order
    links:
      - {rel: prior-art, to: prior/net-pg-protocol}
      - {rel: prior-art, to: prior/surface-spacetimedb}
  arrow:
    title: Arrow IPC record batches, one schema per result
    links:
      - {rel: prior-art, to: prior/net-flight}
      - {rel: prior-art, to: prior/arrow-union}
---
Execution produces Arrow batches with dense unions for value sums
(`exec/batch/arrow`, `exec/value-sum/dense`). Both may be offered, the
client choosing per call.

## net/results/rows

```
<- Batch [row: 02 | 07 "g-123"] [row: 01 | 0d "a@example.com"]
```

Cheap for a few rows; the server transposes batches.

## net/results/arrow

```
<- Schema { field 0: DenseUnion [2: EmailPass {email}, 3: Google {gid}] }
<- RecordBatch { types: [3, 2], offsets: [0, 0], children: ... }
```

Zero transposition from execution; analytic clients get batches they
can hand to DuckDB or Polars. A relation-level sum result is one dense
union column.
