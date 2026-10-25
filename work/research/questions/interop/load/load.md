---
title: How bulk data enters the database
pick: some
options:
  source:
    title: An external file is a relation in a query; loading is an insert from it
    links:
      - {rel: prior-art, to: prior/interop-duckdb-io}
      - {rel: prior-art, to: prior/interop-clickhouse-tablefn}
      - {rel: variant-of, to: write/bulk/from-query}
  command:
    title: A dedicated load command naming a target relation and a file
    links:
      - {rel: prior-art, to: prior/interop-pg-copy}
  append-api:
    title: The host program streams Arrow batches into a constructor relation
    links:
      - {rel: gains, to: exec/batch/arrow}
---
CSV, JSON, Parquet and Arrow files, and host programs holding rows,
need a way in. Writes are change sets computed by queries
(`write/bulk/from-query`, `write/surface/txdata`); the question is
whether a load is one of those or a separate mechanism. These stack.

## interop/load/source

```
insert identities.Google from (
  from r in read_csv("google.csv", GoogleRow)
  select Google { id: r.id, user: Key users r.user, gid: r.gid })
```

`read_csv`, `read_parquet`, `read_ndjson`, `read_arrow` are relation
sources typed by a declared row type. The load is an ordinary write,
checked like any change set. DuckDB `read_csv` and replacement scans
(https://duckdb.org/docs/2.0/data/parquet/overview.html); ClickHouse
table functions (https://clickhouse.com/docs/sql-reference/table-functions).

## interop/load/command

```
load identities.Google from "google.csv" format csv header
```

Postgres `COPY FROM` (https://www.postgresql.org/docs/current/sql-copy.html):
columns map by position or name onto one relation, no transformation.
A second write path beside `insert`.

## interop/load/append-api

```rust
let mut w = db.appender(schema::identities::Google)?;
for batch in reader { w.push(batch?)?; }  // arrow RecordBatch, one schema
w.commit()?;  // one change set
```

Embedded only. A constructor heap has one schema with no validity
bitmap (`exec/batch/arrow`), so a batch matching it is copied, not
converted. DuckDB appender (unverified).
