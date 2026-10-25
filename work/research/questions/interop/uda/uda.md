---
title: How users define aggregates
pick: one
options:
  fold-decl:
    title: An aggregate declares a state, a step, a merge and a finish
    links:
      - {rel: prior-art, to: prior/interop-pg-extensions}
  collection-fn:
    title: An aggregate is a function over the nested collection a group binds
    links:
      - {rel: gains, to: lang/group/into}
  builtin-only:
    title: Only engine aggregates; anything else is a query over the group
---
Aggregates range over bindings (`lang/collection/set-agg/over-binders`)
and run morsel-parallel (`exec/parallel/morsel`), so partial states
from several workers must combine.

## interop/uda/fold-decl

```
aggregate mean (x: F64) -> F64 =
  state { s: F64, n: I64 } init { s: 0.0, n: 0 }
  step  (st, x) -> { s: st.s + x, n: st.n + 1 }
  merge (a, b)  -> { s: a.s + b.s, n: a.n + b.n }
  finish st     -> st.s / f64(st.n)
```

Postgres `CREATE AGGREGATE` with `sfunc`, `stype`, `finalfunc` and
`combinefunc` for parallel partial aggregation
(https://www.postgresql.org/docs/current/xaggr.html). `merge` must be
associative; the engine cannot check it.

## interop/uda/collection-fn

```
fn median (xs: rows F64) -> F64 = ...
from o in orders group o by o.region into g select { r: g.key, m: median(x.total for x in g) }
```

No separate declaration form; `group into` already binds the rows.
The planner sees an opaque function over a whole group.

## interop/uda/builtin-only

`sum`, `count`, `min`, `max`, `avg`, sketches. A median is written as a
query over the group's rows ordered and indexed.
