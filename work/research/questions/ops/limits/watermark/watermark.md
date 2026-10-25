---
title: A long reader holds back vacuum of every heap
pick: any
raised-by: [mvcc/watermark]
options:
  report:
    title: The engine names the transaction or bundle holding the oldest snapshot, per heap
    needs: [mvcc/long-reader/ctor-scoped]
  kill-oldest:
    title: When dead versions pass a bound the oldest snapshot holder is ended
  live:
    title: Live with it
    excludes: [ops/limits/watermark/report, ops/limits/watermark/kill-oldest]
---
One oldest-snapshot watermark (settled), narrowed per heap for
registered queries that name the constructor relations they read
(settled `mvcc/long-reader/ctor-scoped`). An operator still needs to
find who is holding it.

## ops/limits/watermark/report

```
from h in sys::heaps where h.dead_versions > 1000000
select { ctor: h.ctor, held_by: h.horizon_holder }   -- T577 queries.export (batch-1.2), 41 min
```

## ops/limits/watermark/kill-oldest

Postgres has `old_snapshot_threshold` (removed in 17, unverified),
which failed old snapshots with "snapshot too old" instead.

## ops/limits/watermark/live

Operators find it through the sessions relation.
