---
title: What makes an old version reclaimable
pick: one
raised-by: [engine/in-heap, engine/undo-store, engine/delta-memory]
options:
  watermark:
    title: Older than the oldest snapshot still running
    links:
      - {rel: prior-art, to: prior/mvcc-pg-heap}
      - {rel: prior-art, to: prior/mvcc-hyper}
  interval:
    title: Visible to no running snapshot, including versions between two snapshots
    links:
      - {rel: prior-art, to: prior/mvcc-steam}
links:
  - {rel: prior-art, to: prior/mvcc-wu-2017}
---
The settled per-sum vacuum decides when dead index entries are removed,
not which versions are dead.

```
active snapshots: 20, 95        chain of row 7: v100 v90 v50 v30 v20 v10
watermark (20): keep v100 v90 v50 v30 v20       interval: keep v100 v90 v20
```

## mvcc/watermark

Postgres `xmin` horizon, HyPer's oldest visible transaction id. One
number; a single long reader pins every version written since it began,
in every sum.

## mvcc/interval

HANA's interval GC and Steam's eager pruning keep, per row, only the
version each active snapshot sees, so a chain is never longer than the
number of active snapshots. Needs the sorted list of active snapshot
timestamps (Steam refreshes it about every 5 ms). Excising a middle
version is a pointer splice in an undo or delta chain; in a heap chain
it means relinking `next` across pages, which Postgres pruning never
does (it removes only a prefix within one page).
