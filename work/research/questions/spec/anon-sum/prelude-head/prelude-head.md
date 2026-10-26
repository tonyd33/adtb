---
title: "Whether a prelude type name before { is an anonymous constructor"
pick: one
options:
  type-app:
    title: "A prelude type name followed by { begins a type application, and no anonymous constructor takes a prelude type name"
  ctor:
    title: "Any UPPER followed by { in a type position is an anonymous constructor"
---
Raised while implementing the front end against docs/spec/. Under
spec/anon-sum/one-ctor/one-ctor, `Set { a: I64 }` read as a
one-constructor anonymous sum named `Set`, which prints the same as
the set of records it was meant to be, so a mismatch error reads
"expected Set { a: I64 }, found Set { a: I64 }".

## spec/anon-sum/prelude-head/type-app

An anonymous constructor may not be named after a prelude type, and a
prelude type name immediately followed by `{` begins a type
application whose first argument is that record type. `Set { a: I64 }`
is `Set ({ a: I64 })`; `Set { a: I64 } | Other` is refused, naming
`Set`.

```adt
query names () -> Set { name: Text } = from u in users select { name: u.name }
```

## spec/anon-sum/prelude-head/ctor

Keep `C { ... }` an anonymous constructor for every `C`, so `Set {
a: I64 }` is a one-constructor sum named `Set` and the set of records
needs parentheses. Two different types print alike.
