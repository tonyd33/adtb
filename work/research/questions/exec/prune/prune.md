---
title: When arms of a lowered match are dropped from the plan
pick: some
raised-by: [query/union-all]
options:
  compile:
    title: At compile time from types and constant arms
  init:
    title: At executor start from parameter values
  run:
    title: During execution per outer row
  empty:
    title: Skip arms whose heap is empty in the snapshot
links:
  - {rel: prior-art, to: prior/exec-pg-planner}
---
Postgres prunes partitions at plan time, at executor initialization
and whenever an execution parameter changes. Constructors play the
partitions, with types as well as values to prune on.

## exec/prune/compile

An arm whose body is the empty collection, or a source typed to a
constructor subset (`fk/to-subset`), never reads the other heaps.

```
from p in payments select p.card.last4   -- card : Key methods.(Card | Wallet)
-- arms for Bank are not planned
```

## exec/prune/init

A parameter that fixes the constructor (`where tag(i) == t`) prunes
when the plan starts; a generic cached plan keeps every arm.

## exec/prune/run

In an index nested loop the union index entry names the constructor,
so only that heap is fetched per outer row (`exec/probe/fuse`).

## exec/prune/empty

A constructor added by migration has an empty heap; its arm costs one
page check (unverified whether worth a special case).
