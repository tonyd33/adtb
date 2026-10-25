---
title: Which external formats the engine reads and writes
pick: some
options:
  csv:
    title: CSV
  ndjson:
    title: Newline-delimited JSON
  parquet:
    title: Parquet
    links:
      - {rel: prior-art, to: prior/interop-duckdb-io}
  arrow-ipc:
    title: Arrow IPC stream and file formats
    links:
      - {rel: prior-art, to: prior/arrow-union}
      - {rel: gains, to: exec/value-sum/dense}
---
Each format is a reader for `interop/load` and a writer for export:

```
export (from i in identities.Google select i) to "google.parquet"
```

They differ in whether they can carry a sum at all.

## interop/formats/csv

Flat text cells; no types, no nesting. Every sum field needs an
encoding.

## interop/formats/ndjson

Nested objects; a sum is a JSON encoding choice (`interop/json-map`).

## interop/formats/parquet

Typed, columnar, nested structs and lists, filter and projection
pushdown. No union logical type; PARQUET-756 is open
(https://issues.apache.org/jira/browse/PARQUET-756).

## interop/formats/arrow-ipc

Schema message then record batches; dense and sparse unions are
serialized like other nested types
(https://arrow.apache.org/docs/format/Columnar.html). Value-level sums
are already dense unions in execution (`exec/value-sum/dense`), so
export is a copy of batches.
