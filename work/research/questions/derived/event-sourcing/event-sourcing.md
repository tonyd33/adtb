---
title: How event sourcing is expressed
pick: one
options:
  sum-log:
    title: An append-only relation of an event sum, with state as stored views over it
    links:
      - {rel: prior-art, to: prior/derived-kurrentdb}
  change-log:
    title: State relations are primary and their kept change sets are the events
  none:
    title: No particular support
---
KurrentDB events are typed by a free string and projections dispatch
on it, an open match. A closed sum makes the event set declared and
every projection exhaustive.

## derived/event-sourcing/sum-log

```
type OrderEvent = Placed { order: Id, at: Instant } | Shipped { order: Id, at: Instant }
                | Cancelled { order: Id, reason: Text }
relation events : { seq: I64, ev: OrderEvent } key seq append only
fn orders () -> Set Order = ...   -- fold over events per order
store orders
```

Adding an event constructor breaks every projection until it states
its arm (`lang/wildcard/scalars-only`).

## derived/event-sourcing/change-log

Writes go to `orders`; `changes(orders)` (`derived/history/changes`) is
the event stream, typed by the sum of constructors.

## derived/event-sourcing/none

Users model events as relations with no engine help.
