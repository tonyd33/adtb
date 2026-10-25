---
title: What an aggregate over an empty collection gives
pick: one
raised-by: [query/optional-sum/null-ctor]
options:
  option:
    title: Aggregates without an identity return an Option
  nonempty:
    title: Groups are typed nonempty; Option only over arbitrary collections
  default:
    title: Aggregates without an identity require a default argument
  error:
    title: Run-time error on an empty collection
---
`count` and `sum` have identities (0). `max`, `min` and `avg` over no
rows return NULL in SQL. With NULL gone the result needs a type.

## lang/empty-agg/option

```
max(o.total for o in orders where o.user == u)   -- : Option Money
```

Matched or defaulted by the caller.

## lang/empty-agg/nonempty

```
from o in orders group o by o.user into g
select { user: g.key, top: max(x.total for x in g) }   -- g : NonEmpty Order, top : Money
```

A group exists only because a row put it there; Haskell
`Data.List.NonEmpty`, EdgeQL `AtLeastOne` cardinality. Elsewhere
`option`.

## lang/empty-agg/default

```
max(o.total for o in orders where o.user == u, default: 0)
```

## lang/empty-agg/error

As Python `max([])`. Fails a query on data, not on type.
