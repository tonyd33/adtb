---
title: What number a stored value-level sum tag holds
pick: one
raised-by: [sum/value]
options:
  stable-ordinal:
    title: A per-sum ordinal assigned once at declaration and never reused
    links:
      - {rel: prior-art, to: prior/surface-capnp}
  catalog-id:
    title: The constructor's global catalog id
  declaration-index:
    title: The constructor's current position in the declaration
    assessment: {grade: costly, why: removing or reordering a constructor rewrites every stored value}
---
Relation-level constructor ids come from a catalog counter and are
never reused (settled). Stored value-level tags face the same evolution
operators: remove, split, merge, rename.

## layout/tag-value/stable-ordinal

Cap'n Proto `@n` ordinals, protobuf field numbers: removing a
constructor retires its number. Fits `u8` or `u16` tags. Key order on
sum values (`lang/sum-order`) cannot be the raw tag once a constructor
is added in the middle, so ordered encodings map ordinal to rank.

```
type Status = Active | Suspended { until: Instant } | Deleted   -- stored tags 0, 1, 2
-- remove constructor Status.Suspended: tags 0, 2; tag 1 never reappears
```

## layout/tag-value/catalog-id

One id space for relation-level and value-level constructors; 4 bytes
per value, or a per-heap dictionary mapping it down.

## layout/tag-value/declaration-index

Dense and order-preserving; `remove constructor Status.Suspended` renumbers `Deleted`
from 2 to 1 in every row.
