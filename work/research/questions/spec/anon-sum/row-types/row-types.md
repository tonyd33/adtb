---
title: "Whether an anonymous result sum has row types"
pick: one
options:
  row-types:
    title: "An anonymous sum has row types T.C and T.(C | D) with the subtyping and joins of a declared sum"
  whole-only:
    title: "An anonymous sum is one type with no row types"
---
Raised while implementing the front end against docs/spec/. A result
sum is matched and ranged over by constructor (`q.C`) exactly as a
stored sum, but the typing part said anonymous sum types are equal or
unrelated, so `q.C` and a `match` binder had no type to take.

## spec/anon-sum/row-types/row-types

An anonymous result sum has row types `T.C` and `T.(C | D)` exactly as
a declared sum, with the same subtyping and joins; `q.C` on a set of
an anonymous sum has elements of the row type. Two anonymous sums with
the same constructors in another order are one type.

```adt
from r in login_key(u).ByEmail select r.email   -- r : (ByEmail {...} | ByGoogle {...}).ByEmail
```

## spec/anon-sum/row-types/whole-only

`q.C` and a binder in a `match` arm have the whole anonymous sum's
type, and field access on them is refused. Ranging over one
constructor of a result is then useless.
