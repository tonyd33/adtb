---
title: "Equality operator"
pick: one
options:
  double:
    title: "Double equals"
  single:
    title: "Single equals"
---
One operator for equality in queries, guards and routing.

## syntax/equality/double

`where u.name == n`; `=` only binds (`let x = ...`) and gives fields in writes.

## syntax/equality/single

`where u.name = n`, as SQL and Tutorial D.
