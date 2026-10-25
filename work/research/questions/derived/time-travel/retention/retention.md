---
title: Retained versions bloat heaps and the union index
pick: any
raised-by: [derived/time-travel/window, derived/time-travel/versioned, derived/time-travel/everything]
options:
  history-heap:
    title: Vacuum moves retained dead versions to a history heap per constructor
    links:
      - {rel: prior-art, to: prior/derived-sql2011-temporal}
  hold-horizon:
    title: Retained versions stay in place; vacuum's horizon is held back
    assessment: {grade: costly, why: current-state scans and index probes pay for history}
  live:
    title: Live with retention slowing current reads
    excludes: [derived/time-travel/retention/history-heap, derived/time-travel/retention/hold-horizon]
---
`mvcc/long-reader` shows one old snapshot bloating every heap; a
retention window is a permanent old snapshot.

## derived/time-travel/retention/history-heap

```
heaps: accounts.Open, accounts.Closed, history(accounts.Open), history(accounts.Closed)
```

Current heaps and the union index stay small; a past read scans both
and filters by version range. SQL Server's history table; Datomic's
unfiltered db is fastest because it filters nothing.

## derived/time-travel/retention/hold-horizon

Postgres-style bloat for the length of the window.

## derived/time-travel/retention/live

As `hold-horizon`, unmanaged.
