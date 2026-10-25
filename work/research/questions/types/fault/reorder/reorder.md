---
title: Reordering changes whether and which fault a query raises
pick: any
raised-by: [types/fault/abort]
options:
  ordered-guards:
    title: Guards order evaluation, and only total expressions move past them
  error-set:
    title: A faulting expression denotes a set of faults, and which is reported is unspecified
    links:
      - {rel: prior-art, to: prior/types-imprecise}
  live:
    title: Live with order-dependent faults, as SQL does
    excludes: [types/fault/reorder/ordered-guards, types/fault/reorder/error-set]
    assessment: {grade: broken, why: "a query correct under one plan aborts under another, and a replan changes it"}
---
```
from o in orders where o.qty > 0 and o.total / o.qty > 10 select o
```
Postgres and SQL Server leave subexpression order undefined, so the
guard does not guard. Here plans also change on replan
(`exec/replan/stats`).

## types/fault/reorder/ordered-guards

`and`, `or`, `match` arms, guards and `where` before `select`
evaluate left to right as written: a faulting expression is reached
only where its guards held. The checker marks each expression total or
partial; the optimizer reorders, pushes down or hoists only total ones,
and moves a partial one only into a position its guards still dominate.
Filter pushdown of `o.qty > 0` stays free; pushing the division below
it is refused.

## types/fault/reorder/error-set

Whether a statement faults is fixed by the denotation (with the guards
above); which fault it reports, when several rows or operands fault,
is any member of the set. Peyton Jones et al.: this keeps `+`
commutative and every reordering valid.

## types/fault/reorder/live

Faults depend on the plan.
