---
title: "Whether add constructor applies to a record type"
pick: one
options:
  allowed:
    title: "Allowed, as on any sum: a record is a one-constructor sum; registered queries are rechecked"
  refused:
    title: "Refused: a record keeps one constructor"
---
Raised by the migration operators. model.md §2.2 says adding a second
constructor to a record is an ordinary schema operator; evolution.md
did not say, and the checker refused it.

## spec/record-add-ctor/allowed

```adt
add constructor Point.Polar { r: F64, theta: F64 }
-- type Point = Point { x: I64, y: I64 } | Polar { r: F64, theta: F64 }
```

The record's fields stay those of its constructor. Schema objects and
the registered queries and transactions of live bundles are rechecked;
the migration is refused if one fails.

## spec/record-add-ctor/refused

A record keeps one constructor; growing it is `add type` of a new sum
and a migration of the data.
