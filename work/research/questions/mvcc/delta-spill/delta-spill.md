---
title: A writer whose old versions do not fit in memory
pick: any
raised-by: [engine/delta-memory]
options:
  bulk-exclusive:
    title: Large writers run alone among writers and mark pages with a bulk epoch instead of keeping versions
    links:
      - {rel: prior-art, to: prior/mvcc-umbra}
  spill:
    title: Version buffers spill to a temporary file
    links:
      - {rel: prior-art, to: prior/mvcc-undo-engines}
  live:
    title: Live with a cap on transaction size
    excludes: [mvcc/delta-spill/bulk-exclusive, mvcc/delta-spill/spill]
---
In-memory version chains (HyPer, DuckDB) bound a transaction by memory.
A split of a large constructor rewrites every row of it into new heaps.

## mvcc/delta-spill/bulk-exclusive

Umbra: a bulk writer takes exclusive write access, readers continue;
each page it touches records the writer's bulk epoch and per-tuple
flags; readers that started before the epoch's commit see the
before-state through a virtual version. One bulk operation at a time.
For a split, the old heap stays readable to old snapshots until the
epoch commits.

## mvcc/delta-spill/spill

OrioleDB writes undo to disk only when it overflows shared memory or
at checkpoints.

## mvcc/delta-spill/live

FoundationDB-style limits (`prior/engine-fdb`).
