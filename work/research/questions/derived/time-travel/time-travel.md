---
title: Whether queries can read past states
pick: one
options:
  none:
    title: Only the current state; versions die at the reclamation horizon
  window:
    title: Any snapshot within a retention window
    links:
      - {rel: prior-art, to: prior/derived-materialize}
      - {rel: variant-of, to: mvcc/long-reader/snapshot-too-old}
  versioned:
    title: Relations declared versioned keep every version until pruned
    links:
      - {rel: prior-art, to: prior/derived-sql2011-temporal}
  everything:
    title: Every relation keeps its full history
    links:
      - {rel: prior-art, to: prior/derived-datomic-time}
      - {rel: prior-art, to: prior/derived-xtdb}
---
Versions live in the heap and are reclaimed by batched per-sum vacuum
(`engine/in-heap`, `union-index/vacuum/batched`). Keeping them longer
gives time travel; what is kept, and for how long, is the question.

```
at version 4120 { from i in invoices.Paid select i }
```

## derived/time-travel/none

Snapshots exist only for running transactions.

## derived/time-travel/window

The horizon is the older of the oldest snapshot and `now - window`.
Materialize `AS OF` needs history within the retention window;
Oracle flashback is bounded by undo (unverified).

## derived/time-travel/versioned

```
relation accounts : Account key id versioned
```

SQL:2011 system-versioned tables: system time set only by the engine,
so history cannot be tampered with. SQL Server keeps a separate
history table; MariaDB partitions history and has `DELETE HISTORY`.

## derived/time-travel/everything

Datomic: one immutable log, `as-of`, `since`, `history`; XTDB: every
table bitemporal. Removal needs excision.
