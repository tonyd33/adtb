---
title: Typing a query with refinements needs entailment
pick: any
raised-by: [constraint/refine/typed]
options:
  fragment:
    title: Refinements limited to a decidable fragment
  cast:
    title: Undecided refinements become explicit checked casts
  live:
    title: Live with an SMT solver in the query checker
    excludes: [constraint/refine/entail/fragment, constraint/refine/entail/cast]
---
`o.qty * 2 : Pos` holds, `o.qty - 1 : Pos` does not, and deciding
which in general is an SMT problem. Queries are compiled at schema
change and at registration, so solver time and solver failures land
there.

## constraint/refine/entail/fragment

Intervals, finite value sets and regular languages, closed under the
operators the checker knows; anything else is `write-only`.

## constraint/refine/entail/cast

```
from o in orders select per_unit(o.total, (o.qty - 1) as Pos)
```

The cast fails the query at run time on a violating row, as Liquid
Haskell's escape to a runtime assert (unverified).

## constraint/refine/entail/live

Liquid types discharge obligations with Z3; the checker inherits its
timeouts.
