---
title: One long reader holds back reclamation everywhere
pick: any
raised-by: [mvcc/watermark]
options:
  ctor-scoped:
    title: A transaction whose constructor relations are known holds back only their heaps
    links:
      - {rel: gains, to: evolution/breaks/invisible/registered}
      - {rel: prior-art, to: prior/mvcc-steam}
  snapshot-too-old:
    title: Versions past a retention limit are reclaimed and a reader needing one fails
    links:
      - {rel: prior-art, to: prior/mvcc-undo-engines}
  live:
    title: Live with bloat and long chains while a long reader runs
    excludes: [mvcc/long-reader/ctor-scoped, mvcc/long-reader/snapshot-too-old]
---
Wu et al. sect. 5.3 and Steam sect. 1: versions accumulate, chains
lengthen, the long reader slows further. InnoDB's history list grows;
Postgres heaps bloat.

## mvcc/long-reader/ctor-scoped

A registered query is compiled against constructor relations, so its
transaction publishes its snapshot only to the horizons of the heaps it
can touch. A report over `invoices` does not pin `identities`. HANA has a
table-level GC for statements whose tables are known (Steam sect. 3).
Interactive transactions, whose reads are unknown, still pin every heap.

```
transact read_only
  return sum(v.total for v in invoices.(Paid | Void))
-- horizon pinned: heaps invoices.Paid, invoices.Void only
```

## mvcc/long-reader/snapshot-too-old

Oracle: undo is circular; a query whose undo was overwritten fails with
"snapshot too old"; `undo_retention` sets a minimum. Postgres had
`old_snapshot_threshold` from 9.6 and removed it in 17
(https://www.postgresql.org/docs/release/17.0/).

## mvcc/long-reader/live

InnoDB and Postgres default behaviour.
