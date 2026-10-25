---
title: Which computed fields a type or relation may declare
pick: any
options:
  virtual:
    title: A field defined by an expression over the row, evaluated on read
    links:
      - {rel: prior-art, to: prior/derived-pg-generated}
  stored:
    title: A field computed on write and stored in each constructor heap
    links:
      - {rel: prior-art, to: prior/derived-pg-generated}
  expr-index:
    title: An index over an expression, with no field
---
With none taken, a computed value is a schema function applied in
queries (`fn area (s: Shape) -> F64`). A field over a sum is a match,
so adding a constructor states its arm, as for match-scoped
constraints (`constraint/scope/new-ctor/arm-in-add`).

## derived/computed/virtual

```
type Shape = Circle { r: F64 } | Rect { w: F64, h: F64 }
  with area: F64 = match self | Circle c -> 3.14159 * c.r * c.r
                                | Rect r   -> r.w * r.h
```

Sugar for a schema function, read as `s.area` like a header field
(`model/sum-header`). Postgres 18 virtual generated columns (default).

## derived/computed/stored

```
  with stored area: F64 = ...
```

Computed on insert, update and retag (`write/retag/assign`), so it can
be indexed and keyed. Postgres restricts the expression to immutable
functions over the current row, no subqueries, no other generated
column. Changing the expression is a schema operator rewriting heaps.

## derived/computed/expr-index

```
add index (area(self)) in shapes
```

The stored benefit without a field; the planner matches the
expression. Postgres expression indexes.
