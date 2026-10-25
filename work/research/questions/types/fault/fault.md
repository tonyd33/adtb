---
title: What a runtime fault in a query does
pick: one
options:
  abort:
    title: The statement aborts with an error and its transaction cannot commit it
    links:
      - {rel: prior-art, to: prior/types-sql-errors}
  total-results:
    title: Partial operations return Option or Result, so every expression is total
  total-defaults:
    title: Every operation is total by a defined default result
    assessment: {grade: hack, why: a wrong answer is returned where the input was out of domain}
links:
  - {rel: prior-art, to: prior/types-numeric}
---
Division by zero, overflow, an out-of-range conversion
(`i32(5000000000)`), a calendar date that does not exist. Queries are
pure (`write/surface/txdata`) and the optimizer reorders them.

## types/fault/abort

```
from o in orders select o.total / o.qty     -- qty == 0 aborts the statement
```

Postgres, Rust panics, OCaml `Division_by_zero`. A registered query's
signature stays `Set I64`.

## types/fault/total-results

```
from o in orders select o.total / o.qty     -- : Set (Option I64)
```

Every partial operator returns `Option`; `/` on `I64` has type
`I64 -> I64 -> Option I64`. Pony's partial operators are the opt-in
form of this. Arithmetic chains become nested matches.

## types/fault/total-defaults

Pony's default `/`: `x / 0 = 0`, integer overflow wraps.
