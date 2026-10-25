---
title: What a point in the past is named by
pick: one
raised-by: [derived/time-travel/window, derived/time-travel/versioned, derived/time-travel/everything]
options:
  version:
    title: The commit sequence number
  wall:
    title: The commit's wall-clock time
  both:
    title: Commit numbers, with a recorded map from wall time to commit number
    links:
      - {rel: prior-art, to: prior/derived-sql2011-temporal}
---
Serializable commits have a total order. Wall clocks do not give one
across machines.

## derived/time-travel/basis/version

`at version 4120`. Exact; DBSP's time counts transactions.

## derived/time-travel/basis/wall

`at time 2026-10-01T00:00Z`. SQL Server records transaction begin
time; several updates in one transaction get zero-length versions.

## derived/time-travel/basis/both

MariaDB transaction-precise history keeps transaction ids and maps them
in `mysql.transaction_registry`; Datomic takes a tx, a basis t or an
instant.
