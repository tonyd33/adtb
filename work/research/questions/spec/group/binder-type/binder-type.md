---
title: "Where Group K T is a type"
pick: one
options:
  binder-only:
    title: "Group K T is only the type of a group binder; passed on, a group is its nonempty Set T"
  first-class:
    title: "Group K T is a type written in signatures"
---
Raised while implementing the front end against docs/spec/. A group
binder has type `Group K T`, and the spec did not say whether that
type may be written, or what a function receives when a group is
passed to it.

## spec/group/binder-type/binder-only

`Group K T` is only the type of a group binder; it is never written in
a signature. A group passed as an argument is passed as the nonempty
set it denotes, at type `Set T`.

```adt
fn n_of (xs: Set (Key users)) -> I64 = count(x for x in xs)
from i in identities group i.user by i.user into g select { user: g.key, n: n_of(g) }
```

## spec/group/binder-type/first-class

`Group K T` may be written as a parameter type, and the function sees
`g.key` and bag semantics. Aggregates inside a function then range
over bindings of a caller's group.
