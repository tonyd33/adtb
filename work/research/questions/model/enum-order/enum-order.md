---
title: Enums need an order and sums have none
pick: any
raised-by: [model/nullary-sum]
options:
  decl-order:
    title: Every sum is ordered by constructor declaration order then payload
  derived-order:
    title: A sum is ordered only when it declares an order
  live:
    title: Live with unordered sums
    excludes: [model/enum-order/decl-order, model/enum-order/derived-order]
---
`order by severity` on `Low | Medium | High`, and any key or index whose
field is a sum.

## model/enum-order/decl-order

Haskell `deriving Ord` order; Postgres enums sort by declaration
(https://www.postgresql.org/docs/current/datatype-enum.html). Reordering
constructors is then an evolution step that changes query results.

## model/enum-order/derived-order

```
type Severity ordered = Low | Medium | High
```

## model/enum-order/live

Sorting needs an explicit `match` to a number; indexes order sums by an
internal tag no query observes.
