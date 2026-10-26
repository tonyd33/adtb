---
title: "How many parameters a user-defined aggregate takes"
pick: one
options:
  one:
    title: "Exactly one; several values pass as a record; a declaration with more is refused"
  several:
    title: "Any number; a call passes a value per parameter"
---
Raised by the user-defined aggregate checker. `aggregate mean (x: F64)
-> F64` takes one input, and a call `mean(v for x in q)` gives one
value per binding, so nothing said what `aggregate f (x: A, y: B)`
means.

## spec/agg-params/one

An aggregate takes exactly one parameter. Several values are passed as
one record:

```adt fragment
aggregate wmean (p: { x: F64, w: F64 }) -> F64 = ...
wmean({ x: o.total, w: o.qty } for o in orders)
```

A declaration with another number of parameters is refused.

## spec/agg-params/several

`agg(a, b for x in q)` passes one value per parameter. Needs a call
form with several values before `for`, which the aggregate syntax does
not have.
