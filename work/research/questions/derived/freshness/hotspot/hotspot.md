---
title: An in-transaction aggregate row serializes every writer to its group
pick: any
raised-by: [derived/freshness/in-txn]
options:
  blind-delta:
    title: Maintenance writes are commutative increments that record no read
    links:
      - {rel: prior-art, to: prior/derived-noria}
  split:
    title: Each aggregate keeps partial rows per writer slot, summed on read
  lag-aggregates:
    title: Aggregate views lag behind; only non-aggregate views are kept in the transaction
    needs: [derived/freshness/lagging-snapshot]
  live:
    title: Live with conflicts on hot aggregate rows
    excludes: [derived/freshness/hotspot/blind-delta, derived/freshness/hotspot/split, derived/freshness/hotspot/lag-aggregates]
---
`open_by_user` has one row per user; every invoice write for that user
updates it. Under SSI an update reads the old count, so two concurrent
invoice inserts for one user conflict and one retries, though neither
read the count. Noria's Lobsters port used transactions only to avoid
these write-write conflicts on vote counts (`kb/noria.md`).

## derived/freshness/hotspot/blind-delta

A count or sum row takes `+1` deltas merged at commit; the maintainer
records no read, so only a transaction that reads the aggregate
conflicts with writers. Escrow-style commutative updates (O'Neil,
TODS 1986, unverified). Covers count and sum; min and max need an
index on the aggregated field for deletes.

## derived/freshness/hotspot/split

Each group stores N partial rows, chosen by writer; a read sums them.
Fewer conflicts, larger reads.

## derived/freshness/hotspot/lag-aggregates

```
store open_by_user lagging
```

Aggregate views are maintained after commit; constraints over
aggregates (`constraint/aggregates/maintained`) still need them fresh.

## derived/freshness/hotspot/live

The engine's own retry contract (`engine/retry/retry-block`) absorbs
the aborts.
