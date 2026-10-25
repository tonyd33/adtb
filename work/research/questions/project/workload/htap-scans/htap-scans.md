---
title: Row heaps under analytic scans
pick: any
raised-by: [project/workload/htap]
options:
  row-vectorized:
    title: Scan row heaps into Arrow batches, no second copy
    links:
      - {rel: prior-art, to: prior/exec-umbra}
  column-replica:
    title: A columnar copy of chosen constructor heaps, maintained from the log
  live:
    title: Live with row-store scan speed
    excludes: [project/workload/htap-scans/row-vectorized, project/workload/htap-scans/column-replica]
---
Per-constructor heaps are row pages (`layout/slots/by-schema`). A scan
of one field of a wide constructor reads every field.

## project/workload/htap-scans/row-vectorized

The scan decodes fixed-slot pages straight into column vectors; one
schema per heap makes the decoder a monomorphic loop. Bandwidth is
still that of the whole row.

```
from i in identities.Google select i.sub   -- reads full Google rows
```

## project/workload/htap-scans/column-replica

TiDB TiFlash and SQL Server columnstore shape (unverified): a columnar
replica per constructor heap, fed by redo, read by snapshots it has
caught up to. A second storage format and its recovery.

## project/workload/htap-scans/live

Scans run at row-store speed; analytics is a secondary use.
