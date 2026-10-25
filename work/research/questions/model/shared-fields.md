---
title: Where fields common to every constructor are declared
pick: one
raised-by: [sum/relation, sum/value]
options:
  sum-header:
    title: Common fields are declared once on the sum
  repeated:
    title: Each constructor repeats them and the sum exposes fields all constructors agree on
---
`id` and `user` are on every `Identity`. Whether `i.user` is readable
without a match.

## model/sum-header

```
type Identity { user: Key users } =
    EmailPass { email: Email, hash: Bytes }
  | Google    { gid: Text }
relation identities : Identity key id

from i in identities select i.user
```

Stored on each constructor's heap (`storage/table-per-ctor`); the header
is not a relation of its own. A retag keeps header fields. TypeScript
discriminated unions read common fields without narrowing
(https://www.typescriptlang.org/docs/handbook/2/narrowing.html).

## model/repeated

```
type Identity = EmailPass { user: Key users, email: Email }
              | Google    { user: Key users, gid: Text }
```

`i.user` typechecks because every constructor has `user` at one type.
Adding a constructor without `user` breaks every such read.
