---
title: "How a unique with no constructor scope is named"
pick: one
options:
  bare:
    title: "Without an in clause: m::r.unique(p1, ..., pn)"
  all:
    title: "With every constructor: m::r.unique(p1, ..., pn) in (C | D | ...)"
---
Raised by the reference interpreter's writes. constraints.md §7.1
named a field unique `m::r.unique(p1, ..., pn) in S` and did not say
what `S` is for a unique written without `in`.

## spec/unique-names/unscoped/bare

A unique with no `in` is named without one:

```text
auth::users.unique(name)
```

The name is the declaration as written.

## spec/unique-names/unscoped/all

The unique is named with every constructor of the relation. The name
then changes when the sum gains a constructor, though the constraint
does not.
