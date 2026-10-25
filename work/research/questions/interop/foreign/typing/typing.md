---
title: How an external relation gets its type
pick: one
raised-by: [interop/foreign/file-scan, interop/foreign/attach, interop/load/source]
options:
  declared:
    title: The user states a row type and each read checks the file against it
  inferred:
    title: The type is inferred from the file's schema or a sample when the query is checked
    links:
      - {rel: prior-art, to: prior/interop-duckdb-io}
---
Queries are checked at registration (`exec/plan-cache/at-register`),
before any file exists.

## interop/foreign/typing/declared

```
type LogRow = { at: Instant, user: I64, bytes: I64 }
from r in read_parquet("logs/*.parquet", LogRow) select ...
```

A file whose schema does not convert fails at read, naming the column.

## interop/foreign/typing/inferred

`read_parquet("logs/x.parquet")` with no type: the checker opens the
file. The DuckDB CSV sniffer guesses dialect, header and types from a
sample (unverified). The query's type depends on a file, so it can
change without a schema change.
