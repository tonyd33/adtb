---
title: How fields common to every constructor are declared
pick: one
raised-by: [evolution/move-field]
options:
  header:
    title: Fields declared on the sum, present in every constructor
  implied:
    title: A field is common when every constructor declares it alike
---
Moving `created_at` up to the sum needs a place for it to land.

## surface/shared-fields/header

```
type Identity { created_at: Time }
  = EmailPass { email: Text, hash: Bytes }
  | Google    { gid: Text }

from i in identities select i.created_at
```

`i.created_at` is in scope without a match. Each constructor heap
still stores the column. Gel abstract type properties.

## surface/shared-fields/implied

```
type Identity = EmailPass { created_at: Time, email: Text }
              | Google    { created_at: Time, gid: Text }
```

Projection without a match is allowed when every constructor has the
field at one type. Adding `Apple` without `created_at` silently makes
every such projection a type error; moving a field to the sum is then
a constraint the engine infers rather than an operator.
