---
title: Folding a stream needs event order and relations are sets
pick: any
raised-by: [derived/event-sourcing/sum-log]
options:
  seq-recursion:
    title: The fold is a fixpoint over sequence numbers
    links:
      - {rel: gains, to: lang/recursion/fix}
  ordered-agg:
    title: An ordered aggregate folds a group in key order
  commutative:
    title: State is built only from order-free aggregates over events
  live:
    title: Live with projections computed outside the database
    excludes: [derived/event-sourcing/fold/seq-recursion, derived/event-sourcing/fold/ordered-agg, derived/event-sourcing/fold/commutative]
---
Order is presentation of a final result (`lang/order/result-only`), so
"apply events in sequence" has no direct form.

## derived/event-sourcing/fold/seq-recursion

State after event n is a function of state after n-1 and event n;
incremental maintenance of an append is one step.

## derived/event-sourcing/fold/ordered-agg

```
from e in events group e by order(e.ev) into es
select fold(es order by seq, init: Draft, step: apply)
```

An aggregate over a group with a declared order (`lang/sum-order/declared`);
incremental only for appends at the end.

## derived/event-sourcing/fold/commutative

Status is the latest event by `seq` (a max), totals are sums.

## derived/event-sourcing/fold/live

Applications read the log and keep state themselves.
