---
title: Statistics relations change outside transactions
pick: any
raised-by: [ops/expose/relations]
options:
  snapshot:
    title: A transaction sees statistics as of its first read of them
    links:
      - {rel: prior-art, to: prior/ops-pg-stats}
  volatile-kind:
    title: A distinct relation kind outside serializable history, unusable in constraints and writes
  live:
    title: Live with it
    excludes: [ops/expose/volatile/snapshot, ops/expose/volatile/volatile-kind]
---
Every relation is read under serializable isolation (settled
`engine/serializable-only`). Counters move with every page read; making
each increment a transaction would serialise all work on them. A
statistics relation is therefore not a relation in the sense the rest
of the spec uses: two reads in one transaction differ, and SSI tracks
no dependency on it.

## ops/expose/volatile/snapshot

Postgres `stats_fetch_consistency = snapshot`: the first access copies
the counters for the rest of the transaction. Repeatable within a
transaction, not serializable with writes.

## ops/expose/volatile/volatile-kind

The type system marks system relations volatile. A constraint or a
change set computed from one is a type error, so no committed data
depends on a counter.

```
relation sys::heaps : HeapStats volatile
insert audit from (from h in sys::heaps select h)   -- error: volatile source in a write
```

## ops/expose/volatile/live

Reading statistics is outside the isolation guarantee; the spec says
so.
