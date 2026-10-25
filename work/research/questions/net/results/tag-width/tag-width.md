---
title: Arrow union type ids are 8-bit; stable constructor ids are unbounded
pick: any
raised-by: [net/results/arrow]
options:
  type-map:
    title: The schema maps each stable id to a type code via the union's typeIds
  nest:
    title: Beyond 127 constructors, a union of unions
  live:
    title: Live with a 127-constructor limit on Arrow results
    excludes: [net/results/tag-width/type-map, net/results/tag-width/nest]
---
`surface/sum-encoding/stable-tag` puts the never-reused constructor id
on the wire; `btree/ctor-width/uvarint` makes it unbounded. Arrow's
types buffer is signed 8-bit and more than 128 types need a union of
unions (Arrow columnar format, Unions).

## net/results/tag-width/type-map

```
DenseUnion typeIds: [0 -> ctor 41, 1 -> ctor 57]
```

The batch's code is local; the schema message carries the stable id,
so stability holds per stream.

## net/results/tag-width/nest

Arrow's own answer; nesting depth grows with the constructor count.

## net/results/tag-width/live

A sum past 127 constructors cannot be returned as Arrow.
