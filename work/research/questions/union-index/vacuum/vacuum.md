---
title: Vacuuming each heap separately rescans the shared index
pick: any
raised-by: [encoding/native-cost/union-index]
options:
  batched:
    title: Collect dead TIDs from every constructor heap, one index pass
  live:
    title: Live with one index pass per heap
    excludes: [union-index/vacuum/batched]
links:
  - {rel: prior-art, to: prior/pg-global-index}
---
Removing dead entries from a B-tree is a full index pass. Vacuuming
constructor heaps independently does that pass once per constructor. In
the Postgres global index prototype, 1000 partitions took vacuum from
15 s to 45 min until the global index pass was deferred.

## union-index/vacuum/batched

Vacuum is per sum: scan each constructor heap, accumulate dead TIDs
tagged by constructor, then one bulk delete over the union index.

## union-index/vacuum/live

Acceptable when sums have few constructors.
