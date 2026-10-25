---
title: SQLite and DuckDB concurrency
---
SQLite WAL mode: one writer, readers on snapshots; a reader whose
snapshot is stale cannot upgrade to writer (SQLITE_BUSY_SNAPSHOT), so
transactions are serializable. https://www.sqlite.org/isolation.html
DuckDB: MVCC with optimistic concurrency after Neumann et al. SIGMOD
2015; concurrent writers to one row conflict and the second aborts;
single process. https://duckdb.org/docs/2.0/connect/concurrency.html
