---
title: Which number a sum value's tag is encoded as
pick: one
raised-by: [btree/sum-key/tag-payload]
options:
  decl-rank:
    title: The constructor's position in the declaration
    links:
      - {rel: gains, to: lang/sum-order/derived}
  stable-id:
    title: The constructor's catalog id
    links:
      - {rel: rivals, to: lang/sum-order/derived, for: index order is creation order}
  order-key:
    title: A per-constructor sort key in the catalog, densely insertable
---
The tag byte decides the order of constructors in the index. The
catalog gives each constructor a stable id that never changes; the
language may order constructors by declaration (`lang/sum-order/derived`).

## btree/sum-tag/decl-rank

`Draft 1, Sent 2, Paid 3`, in a uvarint. Index order is language order
with no lookup. Inserting `Void` between `Sent` and `Paid` renumbers
`Paid`.

## btree/sum-tag/stable-id

`Draft #1, Sent #2, Paid #3, Void #4`: adding or reordering never
rewrites a key. Index order is creation order, so an ordered scan by
`status` under derived order needs a sort, or the index serves only
equality and per-constructor prefixes.

## btree/sum-tag/order-key

The catalog keeps per constructor a short byte string; a constructor
inserted between two others gets a string between theirs, so nothing
else moves; a reorder rewrites only the moved constructor's keys.
Fractional indexing as used for ordered lists in collaborative editors
(unverified as index precedent).

```
Draft 40   Sent 80   Paid c0     -- add Void between Sent and Paid: Void a0
```
