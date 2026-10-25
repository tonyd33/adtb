---
title: Dropping a constructor leaves its entries scattered in the index
pick: any
raised-by: [[encoding/native-cost/union-index, evolution/remove]]
options:
  eager:
    title: Scan the dropped heap and delete its entries one by one
    links:
      - {rel: prior-art, to: prior/tidb-global-index}
  lazy:
    title: Invalidate the constructor id, skip its entries, collect later
    links:
      - {rel: prior-art, to: prior/oracle-global-index}
      - {rel: prior-art, to: prior/pg-global-index}
  live:
    title: Live with rebuilding the index on drop
    excludes: [union-index/drop-ctor/eager, union-index/drop-ctor/lazy]
---
Entries are ordered by key, not constructor, so a constructor's entries
are not a contiguous range. Leading the key with the constructor would
make them contiguous but break uniqueness across constructors.

## union-index/drop-ctor/eager

Drop costs a scan of the heap plus a delete per row (TiDB).

## union-index/drop-ctor/lazy

Drop is a catalog change. Probes check the constructor id against the
catalog and ignore invalid ones; a background job removes them (Oracle
12c, Kumar 2025). Orphaned space until cleanup; a uniqueness probe must
not count orphans as conflicts.

## union-index/drop-ctor/live

Mark the index unusable and rebuild, blocking writes to the sum.
