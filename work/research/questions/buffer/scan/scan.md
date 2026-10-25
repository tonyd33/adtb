---
title: Scans and vacuum displacing the hot set
pick: any
raised-by: [buffer/cache/hash-pool, buffer/cache/swizzle, buffer/cache/vmcache]
options:
  ring:
    title: Large scans and vacuum recycle a small private ring of frames
    links:
      - {rel: prior-art, to: prior/buffer-pg}
  plan-hint:
    title: The planner marks each access as one-shot or reusable
  ctor-priority:
    title: Per-constructor heap cache priority
    links:
      - {rel: gains, to: storage/table-per-ctor}
  live:
    title: Live with scans flushing the pool
    excludes: [buffer/scan/ring, buffer/scan/plan-hint, buffer/scan/ctor-priority]
---
A sum scan reads every constructor heap; batched vacuum per sum reads
every heap and one union index pass. Each touches more pages than the
pool holds on a large sum.

## buffer/scan/ring

Postgres: seq scan ring 256 KiB, vacuum ring `vacuum_buffer_usage_limit`,
bulk write ring 16 MiB capped at 1/8 of the pool. A merge-append sum
scan holds one ring per heap stream or one shared ring.
https://github.com/postgres/postgres/blob/master/src/backend/storage/buffer/README

## buffer/scan/plan-hint

The executor tags fetches `once` (sum scan, vacuum) or `keep` (index
inner pages, probe side of a join); `once` frames enter at the cold end.

## buffer/scan/ctor-priority

Per-constructor heaps separate hot and cold rows by type: a retag to
`Archived` moves the row into a heap the pool can deprioritise as a
whole. Oracle's KEEP and RECYCLE pools assign whole segments
(unverified). A single-heap layout cannot express this.

```
type Order = Open     { customer: Key customers }
           | Shipped  { customer: Key customers, shipped_at: Time }
           | Archived { customer: Key customers }
relation orders : Order
  key id
  cache cold in Archived
```

## buffer/scan/live

A nightly sum scan or vacuum evicts the OLTP working set.
