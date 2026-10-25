---
title: "Which payload forms a named-set pattern takes"
pick: one
options:
  like-ctor:
    title: "The payload forms of a constructor pattern, over the fields every member shares"
  binder-only:
    title: "Only S x"
---
Raised while implementing the checker against docs/spec/. queries.md
§6.1 and typing.md §9.1 give only `S x` for a named constructor set
`S`, while a constructor pattern takes `C`, `C _`, `C x` and
`C { ... }`.

## spec/pattern/named-set/like-ctor

A named set takes the payload forms a constructor takes, over the
fields every member shares: `S _`, `S x`, `S { f: p, g }` with each
listed field shared by every member, and bare `S` only when every
member is nullary.

```adt
match i
  | EmailPass { user } -> user
  | OAuth { user }     -> user        -- user is shared by every member
```

## spec/pattern/named-set/binder-only

Only `S x`. An arm that ignores the value must still bind it, and an
arm that reads a shared field binds the whole value and projects.
A named set reads differently from a constructor in the same match.
