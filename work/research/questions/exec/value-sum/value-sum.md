---
title: How a value-level sum field is laid out in a batch
pick: one
raised-by: [sum/value]
options:
  dense:
    title: Tag vector plus offsets into compact per-constructor children
    links:
      - {rel: prior-art, to: prior/arrow-union}
  sparse:
    title: Tag vector plus full-length children for every constructor
    links:
      - {rel: prior-art, to: prior/arrow-union}
      - {rel: prior-art, to: prior/duckdb-union}
  tag-split:
    title: Tag vector plus compact children, offsets derived per batch
  packed:
    title: Tag plus encoded payload bytes per row, decoded on use
---
`shipping: Courier {name: Text} | Post {tracking: Text, zone: u8} |
Pickup` inside `Order` rows. Inside one constructor heap the row has a
fixed schema, but this field still varies per row.

```
from o in orders select match o.shipping
  | Courier c -> c.name | Post p -> p.tracking | Pickup -> "desk"
```

## exec/value-sum/dense

Arrow dense union: `i8` type ids, `i32` offsets increasing per child,
children hold only their rows. At most 128 type ids before a union of
unions.

```
tag     i8   [0, 1, 1, 2, 0]
offset  i32  [0, 0, 1, 0, 1]
Courier.name     ["a", "b"]
Post.tracking    ["T1", "T2"]   Post.zone [3, 4]
```

## exec/value-sum/sparse

Arrow sparse union, DuckDB `UNION` (struct with the tag first, at most
255 members). Every child spans the batch; kernels run over a child
with a selection of the rows whose tag matches, or unconditionally and
the tag picks the result.

```
tag   [0, 1, 1, 2, 0]
Courier.name   ["a", -, -, -, "b"]
Post.tracking  [-, "T1", "T2", -, -]
```

## exec/value-sum/tag-split

Children compact as in dense, but no offsets buffer: a batch builds one
selection vector per tag on demand (a single counting pass over the
tag), and each arm runs over its child with that selection. Same memory
as dense minus 4 B per row (unverified as precedent).

## exec/value-sum/packed

The heap encoding carried into execution: per row a tag and a
variable-length payload. Arms decode their payload when run. No
transposition cost at scan, a decode per access.
