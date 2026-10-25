---
title: Whether a domain predicate is part of the type queries see
pick: one
raised-by: [constraint/domain]
options:
  typed:
    title: A refinement type the query checker knows and propagates
    links:
      - {rel: prior-art, to: prior/constraint-liquid}
  write-only:
    title: A check on write; reads see the base type
    links:
      - {rel: prior-art, to: prior/constraint-tutorial-d}
---
`type Pos = Int where self > 0` either is a type, so `from o in orders
select o.qty` has type `Pos` and feeds a function demanding `Pos`, or
is a check at the storage boundary with `Int` everywhere else.

## constraint/refine/typed

```
fn per_unit (total: Money, qty: Pos) -> Money = ...
from o in orders select per_unit(o.total, o.qty)     -- accepted, qty: Pos
from o in orders select per_unit(o.total, o.qty - 1) -- refused, Int
```

Liquid types. A refinement crossing a write, a function or a schema
change must be proved or checked.

## constraint/refine/write-only

```
type Pos = Int where self > 0
from o in orders select o.qty    -- Int
```

SQL domains: a value read back is the base type, and any narrowing is
the reader's own check.
