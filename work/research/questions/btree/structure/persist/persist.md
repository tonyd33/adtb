---
title: An in-memory index must be rebuilt after every restart
pick: any
raised-by: [btree/structure/art]
options:
  rebuild:
    title: Rebuild each sum's indexes from its heaps at startup
    links:
      - {rel: gains, to: engine/logical-redo}
  checkpoint:
    title: Serialize the tree at checkpoint and replay changes after it
  live:
    title: Live with startup time proportional to data size
    excludes: [btree/structure/persist/rebuild, btree/structure/persist/checkpoint]
---
An ART is not paged, so it is lost on crash and does not fit when an
index outgrows memory. Hekaton logs no index and rebuilds on recovery.

## btree/structure/persist/rebuild

Scan every constructor heap of the sum and bulk load (see
`btree/build`). Pairs with `engine/logical-redo`, which already
rebuilds indexes.

## btree/structure/persist/checkpoint

Write the leaves in key order at checkpoint; recovery bulk loads the
sorted run and replays WAL past it.

## btree/structure/persist/live

Accept the rebuild and memory-resident indexes only.
