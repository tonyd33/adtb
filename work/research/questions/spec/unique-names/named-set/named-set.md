---
title: "How a unique scoped by a named set is named"
pick: one
options:
  ctors:
    title: "By the set's constructors, in code-point order: in (Apple | Google)"
  set-name:
    title: "By the named set: in OAuth"
---
Raised by the reference interpreter's writes. constraints.md §7.1 wrote
`S` as its constructors, while its example named one unique
`in OAuth`.

## spec/unique-names/named-set/ctors

`S` is always its constructors, `C` or `(C | D | ...)` in the
code-point order of their names, however the declaration wrote it:

```text
auth::identities.unique(provider, sub) in (Apple | Google)
```

The name says which constructors the unique ranges over at the
version it is read at. A named set grows as constructors join it
(constraints.md §5), so the name of a unique over one changes when it
grows.

## spec/unique-names/named-set/set-name

The name keeps the set as written, and stays the same as the set
grows. But a unique written `in OAuth` and one written `in (Google |
Apple)` get different names for one scope, and the name does not say
what the unique ranges over.
