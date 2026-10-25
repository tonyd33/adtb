---
title: Constraints do not reach inside nested collections
pick: any
raised-by: [model/nf2]
options:
  values-only:
    title: Nested values carry no keys or uniques and are promoted to relations when they need them
  nested-uniques:
    title: A collection field may declare a key among its elements
  live:
    title: Live without integrity inside nested values
    excludes: [model/nested-integrity/values-only, model/nested-integrity/nested-uniques]
---
`sum/value` says constructor fields of a value-level sum are not keys.
`lines: List Line` raises the same: unique `sku` per order, FKs from
lines, an index on `lines.sku`.

## model/nested-integrity/values-only

A collection is a value; when a line needs a key it becomes
`relation lines : Line key (order, sku)`, an `evolution/nest` inverse.

## model/nested-integrity/nested-uniques

```
type Order = { lines: Set Line key (sku) }
```

## model/nested-integrity/live

Integrity inside a nested value is the writer's problem.
