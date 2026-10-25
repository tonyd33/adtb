---
title: A safe snapshot can lag far behind the applied log under write load
pick: any
raised-by: [repl/standby-reads/safe-snapshot]
options:
  force-point:
    title: The primary periodically delays new read-write starts until a safe point forms
  report-age:
    title: Every replica read reports the commit position and age of its snapshot
  live:
    title: Live with unbounded staleness under sustained overlapping writers
    excludes: [repl/standby-reads/lag/force-point, repl/standby-reads/lag/report-age]
---
A snapshot is safe only once every read-write transaction concurrent
with it has finished without a dangerous structure. With long
overlapping writers no marker may be logged for a long time. Postgres
`DEFERRABLE` waits have the same shape on the primary (unverified how
long in practice).

## repl/standby-reads/lag/force-point

A short gate on new read-write transactions lets the running ones
drain, so a marker is logged at least every interval.

## repl/standby-reads/lag/report-age

```
let r = replica_read { ... }   -- r.position = 0/1A2B, r.age = 340ms
```

## repl/standby-reads/lag/live

No bound is stated.
