---
title: What an enumeration is
pick: one
raised-by: [sum/value]
options:
  nullary-sum:
    title: An enum is a sum whose constructors have no fields
  enum-kind:
    title: Enums are a separate scalar kind
---
`type Status = Active | Suspended | Deleted`.

## model/nullary-sum

```
type Status = Active | Suspended | Deleted
type User = { status: Status }
from u in users select match u.status | Active -> 1 | Suspended -> 2 | Deleted -> 3
```

Same `match`, exhaustiveness and evolution as any sum; a later
`Suspended { until: Time }` is `evolution` on one constructor. Haskell,
Rust, Soufflé ADTs.

## model/enum-kind

Postgres `CREATE TYPE ... AS ENUM`, ordered by declaration
(https://www.postgresql.org/docs/current/datatype-enum.html). Giving a
constructor fields later is a change of kind.
