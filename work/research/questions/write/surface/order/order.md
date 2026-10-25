---
title: A write nested in an expression needs an evaluation order
pick: any
raised-by: [write/surface/expr]
options:
  snapshot:
    title: Every read in the statement sees the state before it
    links:
      - {rel: prior-art, to: prior/write-xquery-update}
      - {rel: prior-art, to: prior/write-datomic-tx}
  top-only:
    title: Writes only at the top of a statement or in its bindings
  live:
    title: Live with evaluation-order effects
    excludes: [write/surface/order/snapshot, write/surface/order/top-only]
---
An `insert` inside a match arm, a join or a correlated subquery runs
once per row the planner happens to evaluate. Whether a later read in
the same statement sees it depends on plan order.

```
from u in users
select match u.plan
  | Free   -> (insert audits.Downgrade { user: key u })
  | Paid p -> (update s in subs where s.user == key u set Sub { s with tier: p.tier })
```

## write/surface/order/snapshot

The statement evaluates against one snapshot and collects a pending
change set, applied at the end. Two writes to one row in a statement
are an error. XQuery Update's pending update list (XUDY0016 on two
replaces) and Datomic's unordered tx data.

## write/surface/order/top-only

As EdgeQL in practice (unverified): writes may appear in `with`
bindings and as the statement body, not inside arms or predicates.

## write/surface/order/live

The planner's order is the semantics.
