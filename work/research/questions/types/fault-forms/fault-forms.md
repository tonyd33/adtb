---
title: Which explicit forms handle faults inside a query
pick: any
raised-by: [types/fault/abort]
options:
  checked:
    title: Checked operators return Option
  try:
    title: A try expression turns a fault into a Result
  wrapping:
    title: Named wrapping and saturating operators
---
With `abort` the default, some queries must test for a fault instead
of dying, e.g. a report over rows of which a few have `qty == 0`.

## types/fault-forms/checked

```
from o in orders select match checked_div(o.total, o.qty) | None -> 0 | Some q -> q.value
```

Rust `checked_div`, Pony `/?`.

## types/fault-forms/try

```
from r in raw select try i32(r.n)       -- : Result I32 Fault
```

Exposes which fault occurred as a value, so with
`types/fault/reorder/error-set` the `Err` payload is nondeterministic;
Peyton Jones et al. confine that choice to IO for this reason.

## types/fault-forms/wrapping

`wrapping_add`, `saturating_mul`, Rust RFC 560's methods, for hashes
and counters that mean modular arithmetic.
