---
title: Adding a constructor with a larger payload widens every row holding the sum
pick: any
raised-by: [layout/vsum/max-payload, layout/vsum/by-shape, layout/vsum/flattened]
options:
  rewrite:
    title: The change rewrites every heap holding the sum, through the heap format change
  live:
    title: Live with it
    excludes: [layout/vsum/widen/rewrite]
---
`Status += Banned { until: Instant, reason: Code8 }` raises the
maximum payload from 8 to 16 bytes, so every `users` row's `status`
changes width; under flattened, every row gains slots.

## layout/vsum/widen/rewrite

Old pages keep their format version and are read as before; new writes
use the new one (`layout/heap-format/page-version`), or the heap is
copied.

## layout/vsum/widen/live

Every value-level sum evolution is a full rewrite of every relation
using it, in one step.
