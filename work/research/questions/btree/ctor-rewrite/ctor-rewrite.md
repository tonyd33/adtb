---
title: Merging or splitting constructors changes the constructor id in index entries
pick: one
raised-by: [[encoding/native-cost/union-index, evolution/merge], [encoding/native-cost/union-index, evolution/split]]
options:
  rewrite:
    title: The operator moves rows to the new heaps and rewrites every entry
  heap-alias:
    title: A constructor may own several heaps, old heaps and entries stay
  live:
    title: Live with rebuilding every index of the sum
---
Merging `Google | Apple` into `OAuth` is, physically, a retag of every
`Google` and `Apple` row. With entries `(key, ctor, tid)`, every entry
of those rows names a constructor that no longer exists.

## btree/ctor-rewrite/rewrite

The operator copies rows into the `OAuth` heap and, per index, deletes
`(k, Google, tid)` and inserts `(k, OAuth, tid')`; batched by sorting
the changes by key. Cost proportional to rows moved times indexes.

## btree/ctor-rewrite/heap-alias

The catalog maps ids `Google #2` and `Apple #4` to logical constructor
`OAuth`; their heaps stay, entries keep `#2` and `#4`, and a scan of
`OAuth` is a scan of its heaps. New `OAuth` rows go to a fresh heap
`#9`. Ids are never reused, so an alias never becomes ambiguous. A
split needs the rows partitioned, so it still moves rows.

```
catalog: OAuth -> heaps [#2, #4, #9]
entry (7, #2, (4,1)) is an OAuth row
```

## btree/ctor-rewrite/live

Drop and rebuild the sum's indexes in the operator.
