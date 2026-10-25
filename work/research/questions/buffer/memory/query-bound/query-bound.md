---
title: Query memory depends on the data under static allocation
pick: any
raised-by: [buffer/memory/static]
options:
  spill:
    title: Operators use fixed arenas and spill partitions to temp pages through the pool
  compile-bound:
    title: The compiler derives a memory bound for registered queries and admits by it
    links:
      - {rel: gains, to: evolution/breaks/invisible/registered}
  admission:
    title: Queries wait for a free fixed-size slot and fail past its arena
  live:
    title: Live with it
    excludes: [buffer/memory/query-bound/spill, buffer/memory/query-bound/compile-bound, buffer/memory/query-bound/admission]
---
TigerBeetle has eight fixed operations over 128-byte records; a query
language has joins, groups and sorts whose state grows with input.
`from o in orders group o by o.customer into g select ...` needs a table sized by distinct
customers.

## buffer/memory/query-bound/spill

Grace hash join and external sort inside a fixed arena; partitions
become temporary heaps in the pool. Static memory, unbounded disk.

## buffer/memory/query-bound/compile-bound

Registered queries are known to the engine; their plans give a bound
from schema facts (a key lookup returns at most one row, a group by a
constructor has as many groups as constructors). Unbounded plans need
`spill` or are rejected.

```
query top_orders (c: Key customers) -> Set Order.Open =
  top 10 by o.total desc (from o in orders.Open where o.customer == c select o)
-- bound: 10 rows of orders.Open + index cursor
```

## buffer/memory/query-bound/admission

A fixed slot count and arena size; a query that overflows its arena
fails with an error naming the limit.

## buffer/memory/query-bound/live

Static allocation holds for the pool only.
