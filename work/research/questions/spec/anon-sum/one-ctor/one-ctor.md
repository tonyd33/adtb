---
title: "Whether C { ... } alone in a type position is an anonymous sum"
pick: one
options:
  one-ctor:
    title: "C { ... } alone is a one-constructor anonymous sum"
  two-or-more:
    title: "An anonymous sum has at least two constructors"
---
Raised while implementing the front end against docs/spec/. The
`sum_type` production required two constructors, while the grammar
notes said `C { ... }` in a type position is always an anonymous
constructor.

## spec/anon-sum/one-ctor/one-ctor

In a type position `C { ... }` alone is a one-constructor anonymous
sum, as a declared record is a one-constructor sum. A lone `C` without
a record is a named type.

```adt
query by_email (e: Text) -> Set (Found { user: Key users }) = ...
```

## spec/anon-sum/one-ctor/two-or-more

An anonymous sum has two or more constructors and `C { ... }` alone is
refused. A one-constructor result is written as an anonymous record,
and the note on `C { ... }` is dropped.
