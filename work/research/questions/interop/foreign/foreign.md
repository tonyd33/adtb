---
title: Whether external data is queried in place
pick: one
options:
  none:
    title: External data is only loaded; queries see stored relations only
  file-scan:
    title: Immutable files read as relations in ad hoc queries and loads, never stored as relation variables
    links:
      - {rel: prior-art, to: prior/interop-duckdb-io}
      - {rel: prior-art, to: prior/interop-clickhouse-tablefn}
  attach:
    title: External sources attached as relation variables with pushdown
    links:
      - {rel: prior-art, to: prior/interop-pg-fdw}
      - {rel: prior-art, to: prior/interop-sqlite-vtab}
---
The engine promises serializable transactions over relations whose
constraints it checks (`engine/serializable-only`,
`constraint/timing/multi-assign`). An external file or database is
neither.

## interop/foreign/none

`interop/load/source` reads files only as the source of an insert.

## interop/foreign/file-scan

```
from r in read_parquet("s3://logs/2026-10/*.parquet", LogRow), u in users
where u.id == r.user
select { name: u.name, n: r.bytes }
```

DuckDB reads Parquet with projection and filter pushdown
(https://duckdb.org/docs/2.0/data/parquet/overview.html). Usable in
ad hoc queries (`surface/adhoc/scoped`); a registered query naming a
file is checked against the row type, not the file.

## interop/foreign/attach

```
attach parquet "s3://logs/" as logs : relation LogRow
attach postgres "postgres://old/app" as old
```

Foreign relations sit in the catalog; the planner negotiates pushdown
as SQLite `xBestIndex` does (https://www.sqlite.org/vtab.html).
